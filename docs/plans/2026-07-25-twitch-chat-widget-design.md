# Twitch Chat Widget Design

## Status

Approved for implementation on `feature/twitch-chat-widget`.

This design replaces the earlier IRC and image-asset design. OverCrow uses the
Twitch-recommended desktop chat-client architecture:

- EventSub over WebSocket for incoming chat events;
- Helix APIs for channel resolution, subscriptions, and outgoing messages;
- Device Code OAuth with `user:read:chat` and `user:write:chat`;
- text-only presentation with bold usernames.

Badge and emote metadata may appear in provider payloads, but OverCrow does not
download or render badge/emote images in this version.

## Product behavior

The user can authenticate one Twitch account, choose any valid public channel,
store an ordered list of favorites, read chat, send messages, and reply to a
specific message. The selected channel does not have to belong to the
authenticated account.

Twitch remains disabled by default and inert unless all existing gates are
open:

1. OverCrow lifecycle is enabled;
2. Core authority is live;
3. an explicitly selected game is active;
4. the Twitch widget is enabled;
5. a valid channel is selected.

Closing chat keeps the authenticated account. Signing out revokes and deletes
credentials. Switching channel clears transient chat state.

Interactive mode provides channel controls, chat history, replies, and the
composer. Passive mode shows recent messages only, remains click-through, and
never exposes controls.

## Provider architecture

One renderer-owned worker contains the provider state. The egui thread sends
bounded semantic commands and receives coalesced immutable snapshots. It never
performs network, Secret Service, or durable filesystem work.

The worker owns:

- the OAuth state machine and credential store;
- one EventSub WebSocket;
- the active channel identity;
- the three required EventSub subscription definitions;
- a bounded message buffer and deduplication set;
- one bounded outbound command queue;
- retry and keepalive deadlines.

The connection sequence is:

1. restore and validate credentials;
2. resolve the normalized channel login through `GET /helix/users`;
3. connect to `wss://eventsub.wss.twitch.tv/ws`;
4. validate `session_welcome` and its session ID;
5. create the required EventSub subscriptions for that session;
6. publish `Joined` only after every required subscription is enabled;
7. process notifications until lifecycle loss, reconnect, or failure.

Required subscriptions are:

- `channel.chat.message` version 1;
- `channel.chat.clear` version 1;
- `channel.chat.message_delete` version 1.

WebSocket reconnect messages are accepted only when the URL remains exact WSS
on `eventsub.wss.twitch.tv`. The replacement socket must receive a valid
welcome before the old socket is discarded.

EventSub delivery is at-least-once. Notification `message_id` values are
deduplicated in a bounded FIFO set before applying payloads.

## Sending

`POST /helix/chat/messages` sends normal messages and replies. Requests include
the active broadcaster ID, authenticated sender ID, and optional parent
message ID.

The Helix response is authoritative:

- `is_sent=true` supplies the final Twitch message ID;
- `is_sent=false` marks the local row failed and retains the draft for retry;
- transport uncertainty marks the local row failed and never retries
  automatically.

The later EventSub echo replaces the matching optimistic local row by message
ID. No `USERSTATE`, message order, username, or text heuristic is used.

## Authentication and recovery

The new scopes invalidate stored IRC-era credentials. Tokens missing the exact
required scopes are removed and the UI asks the user to connect again.

Secret Service failures use bounded 30-second retries while lifecycle authority
remains open. A failed restore never creates a 24-hour sleep and never lets
`BeginAuthentication` remain permanently blocked without a retry path.

Tokens and device codes remain zeroized. OAuth, account, channel, chat,
message, reply, URL, and credential data are never logged.

## Bounds and validation

- 200 retained chat messages;
- 500 Unicode scalar values per outgoing message;
- 64 KiB EventSub WebSocket message;
- 256 KiB Helix response;
- 32 queued UI commands;
- one EventSub socket and three subscriptions;
- 10-second HTTP/WebSocket operations;
- exponential reconnect backoff capped at 60 seconds;
- fixed HTTPS/WSS host allowlists and no HTTP redirects;
- strict response structure, string, collection, and identifier bounds.

Provider failures use only the existing stable Twitch diagnostic identifiers
and fixed categories.

## Display lifecycle hardening

The Twitch work also includes the regressions discovered while reviewing this
branch:

- Hyprland distinguishes a genuinely empty active window from an invalid active
  window. A remembered game survives an empty focus only while it is still
  mapped on the focused monitor's active workspace. Invalid, hidden, malformed,
  or off-workspace state clears authority.
- Loss of live Core authority makes the renderer immediately passive and
  click-through regardless of the retained presentation snapshot.
- An unexpected successful return from the overlay event loop is treated as a
  process failure so systemd restarts it.

X11 and Plasma behavior is unchanged, but their existing passive, interactive,
authority-loss, and close-path tests remain mandatory.

## Acceptance

- Clicking a Hyprland game through a Passive overlay does not clear widgets.
- Leaving the game's workspace clears active-game authority.
- Core loss cannot leave an input-capturing overlay.
- Event-loop termination causes a renderer restart under systemd.
- Any chosen valid public Twitch channel can be read and written.
- Duplicate EventSub notifications appear once.
- Rejected sends appear failed and are never retried automatically.
- Usernames are bold; badge/emote icons are absent.
- Twitch and lifecycle diagnostics contain no private content.
