# OverCrow

OverCrow is an opt-in external Linux game overlay built by Valhallab and
distributed under the PlayerVox brand. It adds movable widgets without injection.

The project is usable today on its primary Hyprland target, but remains an
early release. Plasma and X11 need more real-machine coverage before they can
be considered equally mature.

## Compatibility

| Environment | Status | Notes |
| --- | --- | --- |
| Hyprland 0.55+ Wayland | **Supported** | Primary Arch/Omarchy target; native Wayland and XWayland games. |
| Plasma 6 Wayland | **Support validation in progress** | KWin bridge is implemented; broader hardware testing is needed. |
| Generic X11/EWMH (including Plasma and XFCE) | **Experimental — for now** | Window tracking works without a Wayland bridge; shortcuts, HiDPI, and desktop-specific behavior need more validation. |
| GNOME Wayland | **Not compatible — for now** | Requires a dedicated GNOME Shell/Mutter bridge. |
| Sway Wayland | **Not compatible — for now** | Requires a dedicated Sway IPC and layer-shell bridge. |
| Gamescope / Steam Deck Game Mode | **Not compatible — for now** | No nested-compositor integration yet. |
| Other Wayland compositors | **Not compatible — for now** | A transparent window alone cannot provide safe placement and input control. |

Windowed and borderless-fullscreen games are supported. Exclusive fullscreen
may bypass compositor windows and is outside the current design.

## Quick start

The current pre-alpha supports Arch Linux and Arch-based distributions on
x86_64. Install the complete application from the AUR:

```sh
yay -S overcrow-bin
```

`paru -S overcrow-bin` works as an alternative. Without an AUR helper,
download the package from the
[latest pre-alpha release](https://github.com/Valhallab/PlayerVox-OverCrow/releases/tag/v0.1.0-pre-alpha.3)
and install it with `sudo pacman -U ./overcrow-bin-*.pkg.tar.zst`.

Nothing starts during installation. Open **PlayerVox OverCrow** from the
application menu, or run:

```sh
overcrow-control
```

OverCrow starts disabled. Select at least one detected game, then enable the
runtime from the System Status card. Runtime services and shortcuts become
available only for explicitly selected games.

Closing the Control Center hides its window while PlayerVox OverCrow remains
available from the system tray. The tray menu shows the current status and
provides **Start OverCrow**, **Stop OverCrow**, **Open Control Center**, and
**Quit**. Quit disables the runtime before the tray application exits.

To uninstall, choose **Quit** from the tray, then run:

```sh
sudo pacman -R overcrow-bin
```

User settings are deliberately left in `${XDG_CONFIG_HOME:-$HOME/.config}/overcrow/`.

### Release artifacts

The pre-alpha release contains one complete x86_64 package and its checksum:

- `overcrow-bin-0.1.0prealpha3-1-x86_64.pkg.tar.zst`
- `SHA256SUMS`

## Using OverCrow

Start a selected game and focus its window. The default shortcuts are:

| Shortcut | Action |
| --- | --- |
| `Meta+Alt+O` | Open or close the interactive overlay. |
| `Esc` | Return an open overlay to passive mode. |
| `Meta+Alt+P` | Start or pause the manual stopwatch. |
| `Meta+Alt+R` | Reset the manual stopwatch. |

Plasma Wayland asks once for permission to register these global shortcuts
when the first selected game becomes active. Review the listed shortcuts, then
confirm KDE's system dialog to enable them.

The PlayerVox-styled **Widget library** groups general tools separately from
game-specific widgets. It lets you enable, move, resize, scale, and reset each
widget without leaving the overlay. Passive widgets are read-only and
click-through. Interactive mode captures input only over the authorized game
and always retains a close path.

Useful diagnostics:

```sh
overcrowctl status
overcrowctl logs
./scripts/diagnose.sh
```

Logs stay local under `${XDG_STATE_HOME:-$HOME/.local/state}/overcrow/logs/`.
They rotate, are bounded, and exclude private content. See
[troubleshooting](docs/troubleshooting.md) for recovery steps.

To report a bug, open **Diagnostics → Report a problem**. Review the exact
in-memory preview and optional sanitized logs before explicitly sending it to
PlayerVox support. Nothing is uploaded automatically. No GitHub account is required.

## Built-in widgets

### General

- **Session** — elapsed time since the detected game process started.
- **Clock** — local date and time.
- **Performance** — host CPU, memory, and available temperatures.
- **Manual stopwatch** — Core-owned timer with overlay controls and shortcuts.
- **Media** — current MPRIS media with interactive playback controls.
- **Notes** — private titled notes with independent note/checklist visibility,
  horizontally scrollable tabs, and per-note checklists.
- **Twitch chat** — read and send messages in any selected public channel,
  with ordered favorites, replies, native emotes, and colorized usernames.

### Warframe

- **Warframe status** — open-world cycles, daily reset, and Baro Ki'Teer.
- **Fissures** — current Void Fissures with local filters.
- **Market** — warframe.market search, orders, and trade templates.
- **Sortie & Archon** — the current three Sortie and Archon Hunt missions.
- **Invasions** — current invasion missions, progress, and rewards.

Warframe widgets appear only for Steam App ID `230410`. They use bounded,
unauthenticated requests to the official Warframe world-state endpoint and
warframe.market; they never access a game account or game memory.

Resizable widgets keep their configured editing size in Interactive mode. In
Passive mode their height follows visible content, up to the game viewport, so
hidden sections and short content do not leave an empty panel.

Twitch chat is disabled by default. It connects only while an explicitly
selected game is active and the widget is enabled. Incoming chat uses Twitch
EventSub and outgoing messages use the Helix API. Messages and drafts remain
in memory; only the selected channel, favorites, and passive display lifetime
are stored locally. Native Twitch emotes use static PNGs from Twitch's CDN with
a bounded in-memory cache and text fallback; badge images are not rendered.
OAuth tokens use the desktop Secret Service when available and otherwise
remain in memory until OverCrow exits. The release build contains a public
Twitch application Client ID, never a Client Secret.

## Safety

OverCrow stays outside the game process. It does **not**:

- inject DLLs or shared objects;
- install Vulkan layers or use `LD_PRELOAD`;
- hook graphics APIs, use `ptrace`, or read game memory;
- inspect packets, synthesize input, or modify game files.

It reads ordinary same-user process metadata and uses compositor APIs plus the
user-session D-Bus. Unsupported or ambiguous environments fail closed.

This design is intended to be anti-cheat-friendly, but it cannot guarantee
compatibility with every game or future anti-cheat policy. Follow each game's
current rules.

Security issues should be reported privately as described in
[SECURITY.md](SECURITY.md), not in a public issue.

## Limitations

- Linux support currently means native Arch/Omarchy distribution and the
  display matrix above. A Fedora 42 RPM is available for maintainer testing;
  a public Fedora repository and DEB distribution are still planned.
- A selected game must be focused before a passive overlay can appear.
- Compositor-level placement can briefly lag during geometry changes.
- Performance telemetry describes host/game resource use, not injected frame
  timing; OverCrow does not currently provide a true FPS counter.
- Repository tests cannot prove live Hyprland, Plasma, X11, Proton, or game
  behavior. Those paths require the [manual MVP checklist](docs/testing/manual-mvp.md).

## Architecture

Small Rust processes separate lifecycle, game authority, rendering, and
compositor integration. See [docs/architecture.md](docs/architecture.md) for
the D-Bus event flow, display contracts, persistence, providers, and security.

## Development

Build and run the Control Center from the repository:

```sh
cd crates/overcrow-control-ui
npm ci --ignore-scripts
npm run tauri dev
```

The main local checks are:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets --locked
```

Twitch uses a compiled public Client ID, never a Client Secret. Forks can
override it at compile time:

```sh
OVERCROW_TWITCH_CLIENT_ID="<public-client-id>" cargo build --workspace
```

Shell, KWin, packaging, and release checks are documented in
[AGENTS.md](AGENTS.md). Contributions should stay focused and preserve the
external-window safety model; see [CONTRIBUTING.md](CONTRIBUTING.md).

## License

OverCrow is open-source software licensed under
[AGPL-3.0-only](LICENSE). It was originally created by **Valhallab SASU** and
is distributed under the PlayerVox brand; required attribution is recorded in
[NOTICE](NOTICE).

PlayerVox is a registered trademark owned by Valhallab SASU. The AGPL license
does not grant permission to present modified distributions as official
PlayerVox products. See [TRADEMARKS.md](TRADEMARKS.md).

Third-party dependencies retain their respective licenses.
