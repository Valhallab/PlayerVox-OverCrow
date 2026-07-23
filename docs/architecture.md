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
  requests the portable always-on-top hint.

Every backend must provide the full identity, placement, focus, input, and
cleanup contract. Generic Wayland is therefore not supported. Invalid, stale,
or ambiguous compositor state clears the active game and forces passive input
passthrough.

## Persistence

Private user files live below `${XDG_CONFIG_HOME:-$HOME/.config}/overcrow/`:

- `settings.json` stores the master switch and selected games;
- `widgets.json` stores the global widget profile and normalized geometry;
- `warframe.json` stores Warframe filters and local activity preferences.

Writes validate ownership and paths, reject unsafe symlinks, and publish
transactionally. Widget positions are normalized to the game viewport so they
remain usable after resizing. Stable IDs and stored keys are treated as public
compatibility contracts.

## External providers

MPRIS uses the user-session D-Bus. Warframe data uses only allowlisted HTTPS
hosts, rejects redirects and credentials, and bounds time, bytes, entries, and
strings before parsing or rendering. Last-good data expires rather than being
shown indefinitely.

Provider logging contains stable component IDs and fixed failure categories,
never responses, queries, notes, media titles, paths, or other user content.

The native Control Center reads recent diagnostics through the hardened
private log reader. It returns at most the newest 500 sanitized lines and
256 KiB of versioned data. The UI cannot choose a path, run an arbitrary
command, or upload logs automatically.

## Security boundary

OverCrow reads same-user process metadata and compositor state. It does not use
injection, graphics hooks, Vulkan layers, `LD_PRELOAD`, `ptrace`, game-memory
reads, packet interception, input synthesis, or game-file modification.

Interactive input is authorized only for an explicitly selected active game;
passive surfaces remain click-through. Unsupported environments fail closed.
This minimizes anti-cheat exposure but cannot guarantee a publisher's policy.
