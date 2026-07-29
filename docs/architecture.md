# Architecture

OverCrow is an external-window overlay. No component is loaded into the game
process, and the compositor integration is an explicit part of the runtime
contract rather than an optional visual enhancement.

## Process model

- `overcrow-control` is the Tauri Control Center. Its Rust command boundary
  owns installation integration, game selection, compatibility checks, and the
  global enabled state. It also owns the resident system tray; closing its
  window only hides the webview, while the explicit tray Quit action disables
  the runtime before exiting. A single-instance guard reopens the existing
  window instead of starting another authority process. The webview has no
  shell capability, and a new installation is disabled.
- `overcrow-core` is the authority for the active game, overlay mode, session
  timing, telemetry, stopwatch state, and D-Bus API.
- `overcrow-overlay` renders the transparent egui surface and its widgets. It
  does not decide whether a process is an authorized game.
- `overcrow-hyprland` maps Hyprland IPC events and dynamic shortcuts onto the
  Core contract. Plasma uses `integrations/kwin`; X11 uses EWMH directly.
- `overcrowctl` is a small diagnostic and control client for the same D-Bus API.

The Control Center starts Core only after a selected game and supported display
environment pass validation. The renderer and Wayland bridge run on demand
while an authorized game process exists.

Steam discovery joins installed manifests with bounded `appinfo.vdf` type
metadata and non-Steam `shortcuts.vdf` identities. A local iterative parser
extracts only app IDs, names, and application types. Known non-games are
excluded, unknown types remain explicitly marked, and shortcuts reuse Core's
existing `SteamAppId` authority without executable-path matching.

### Native portal identity

Before Core uses an application-scoped desktop portal, it registers the same
D-Bus connection with `org.freedesktop.host.portal.Registry` as
`com.playervox.OverCrow`. The ID must match the installed desktop file. A portal
without the host registry may fall back to its normal cgroup-based identity
detection; any other registration failure leaves portal shortcuts unavailable.
Registration always precedes portal calls and is repeated only after a new
portal connection is established.

## State and event flow

Core publishes a monotonically versioned snapshot when semantic state changes.
The overlay subscribes before reading its baseline, ignores stale revisions,
and treats a Core owner change as a new generation. Notifications are bounded
and coalescing: they wake consumers to read the newest state and are not an
unbounded event log.

A slow reconciliation deadline repairs missed notifications. Time displays are
interpolated locally from authoritative samples, so visual ticking does not
create continuous D-Bus traffic. Providers publish only their newest immutable
result through bounded channels and never block the egui thread.

## Display contracts

- **Hyprland:** a Rust bridge observes its user-session sockets, validates the
  active game, places the overlay, manages the temporary shortcut, and guards
  game input during Interactive mode.
- **Plasma 6 Wayland:** the KWin script reports active-window geometry and keeps
  overlay windows borderless, above the game, and out of desktop switchers.
- **X11:** Core uses EWMH active-window and geometry information; the overlay
  requests the portable always-on-top hint. The Control Center does not install
  or invoke a compositor bridge, and Core rejects bridge window reports so a
  KWin integration left from another login cannot replace EWMH authority.

Every backend must provide the full identity, placement, focus, input, and
cleanup contract. Generic Wayland is therefore not supported. Invalid, stale,
or ambiguous compositor state clears the active game and forces passive input
passthrough.

X11 uses a native passive key grab only while Core reports an authorized active
game. A conflicting grab fails closed and every shutdown path releases owned
keys. Wayland continues to use the desktop portal. EWMH geometry remains in
physical pixels; the renderer converts it with its current native window scale
and reapplies placement after a scale transition.

The Control Center discovers graphics adapters through bounded, read-only PCI
metadata. It reports only normalized vendor classes in the local compatibility
view. Graphics hardware does not independently authorize activation, and raw
device metadata is neither logged nor submitted.

## Persistence

Private user files live below `${XDG_CONFIG_HOME:-$HOME/.config}/overcrow/`:

- `settings.json` stores the master switch and selected games;
- `widgets.json` stores the global widget profile and normalized geometry;
- `warframe.json` stores Warframe filters and local activity preferences.
- `twitch.json` stores one normalized active channel, up to 20 ordered
  favorites, and the passive message lifetime.

Private note content lives separately below
`${XDG_DATA_HOME:-$HOME/.local/share}/overcrow/notes/global.json`. The bounded
schema stores stable titled-note and checklist-entry IDs in one atomic
document. Schema-v1 single-note data is migrated in memory without rewriting
the source file until the next explicit user mutation. Widget configuration
stores only whether the note and checklist sections are visible.

Writes validate ownership and paths, reject unsafe symlinks, and publish
transactionally. Widget positions are normalized to the game viewport so they
remain usable after resizing. Stable IDs and stored keys are treated as public
compatibility contracts.

The renderer maintains separate transient size measurements for Interactive
and Passive modes. Interactive resize changes the persisted panel size;
Passive content-fit measurements affect placement only and never overwrite
saved geometry.

Twitch OAuth credentials are not stored in these files. They use one versioned
desktop Secret Service entry. If Secret Service is unavailable, the active
credentials stay only in zeroizing process memory and the user must reconnect
after restart. Twitch messages, drafts, and replies are never persisted.

## External providers

MPRIS uses the user-session D-Bus. Warframe data uses only allowlisted HTTPS
hosts, rejects redirects and credentials, and bounds time, bytes, entries, and
strings before parsing or rendering. Last-good data expires rather than being
shown indefinitely.

Provider logging contains stable component IDs and fixed failure categories,
never responses, queries, notes, media titles, paths, or other user content.

The Twitch chat connection is owned by one renderer provider worker and remains
inert until the application lifecycle, selected active game, widget, channel,
and credential gates are all valid. It uses Device Code OAuth, allowlisted
Twitch HTTPS, EventSub WebSocket for incoming chat, and Helix for subscriptions
and outgoing messages. Commands use a bounded queue; EventSub deliveries are
deduplicated in a bounded set and snapshots coalesce to the newest 200-message
ring.
Responses, WebSocket messages, strings, retries, and shutdown operations are
bounded. EventSub fragments preserve native Twitch emotes while malformed
optional fragments fall back to the validated message text.

Static emote PNGs are loaded off the render thread from the exact
`static-cdn.jtvnw.net` HTTPS host with redirects and credentials disabled.
Request/result queues, encoded bytes, decoded dimensions, pending identities,
and the memory-only texture cache all have fixed limits. The asset worker is
event-driven, stops before draining queued requests, and waits at shutdown only
for its current bounded request. Failed or unavailable assets remain readable
as their original text. Badge assets and third-party emote providers are
ignored.

Usernames use the semibold UI family while message content stays regular.
Losing active-game authority closes the chat connection; disabling the widget,
switching channel, signing out, or stopping the renderer also clears the
relevant transient private state. Passive presentation remains read-only and
click-through.

The native Control Center reads recent diagnostics through the hardened
private log reader. It returns at most the newest 500 sanitized lines and
256 KiB of versioned data. The UI cannot choose a path, run an arbitrary
command, or upload logs automatically. Support reports are prepared in memory,
previewed exactly, and submitted only after explicit confirmation. The native
client posts to one fixed PlayerVox HTTPS endpoint with no redirects or
credentials and with bounded request, response, and timeout. The Tauri host
accepts only the latest native-generated report ID and permits one submission
at a time.

## Security boundary

OverCrow reads same-user process metadata and compositor state. It does not use
injection, graphics hooks, Vulkan layers, `LD_PRELOAD`, `ptrace`, game-memory
reads, packet interception, input synthesis, or game-file modification.

Interactive input is authorized only for an explicitly selected active game;
passive surfaces remain click-through. Unsupported environments fail closed.
This minimizes anti-cheat exposure but cannot guarantee a publisher's policy.
