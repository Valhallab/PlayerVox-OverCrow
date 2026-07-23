# OverCrow

OverCrow is an opt-in, external game overlay for Linux, built by Valhallab and
distributed under the PlayerVox brand. It provides movable widgets without
injecting code into the game process.

The project is usable today on its primary Hyprland target, but remains an
early release. Plasma and X11 need more real-machine coverage before they can
be considered equally mature.

## Compatibility

| Environment | Status | Notes |
| --- | --- | --- |
| Hyprland 0.55+ Wayland | **Supported** | Primary Arch/Omarchy target; native Wayland and XWayland games. |
| Plasma 6 Wayland | **Support validation in progress** | KWin bridge is implemented; broader hardware testing is needed. |
| Generic X11/EWMH | **Experimental — for now** | Window tracking works; shortcuts, HiDPI, and desktop integration need more validation. |
| GNOME Wayland | **Not compatible — for now** | Requires a dedicated GNOME Shell/Mutter bridge. |
| Sway Wayland | **Not compatible — for now** | Requires a dedicated Sway IPC and layer-shell bridge. |
| XFCE X11 | **Not compatible — for now** | Expected to build on the X11 backend after dedicated validation. |
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
[latest pre-alpha release](https://github.com/Valhallab/PlayerVox-OverCrow/releases/tag/v0.1.0-pre-alpha.1)
and install it with `sudo pacman -U ./overcrow-bin-*.pkg.tar.zst`.

Nothing starts during installation. Open **PlayerVox OverCrow** from the
application menu, or run:

```sh
overcrow-control
```

OverCrow starts disabled. Select at least one detected game, then enable the
runtime from the System Status card. Runtime services and shortcuts become
available only for explicitly selected games.

To uninstall, first disable OverCrow in the Control Center, then run:

```sh
sudo pacman -R overcrow-bin
```

User settings are deliberately left in `${XDG_CONFIG_HOME:-$HOME/.config}/overcrow/`.

### Release artifacts

The pre-alpha release contains one complete x86_64 package and its checksum:

- `overcrow-bin-0.1.0prealpha1-1-x86_64.pkg.tar.zst`
- `SHA256SUMS`

Maintainers prepare future releases with `./scripts/prepare-release.sh` from a
clean `master` checkout after the
[real-machine pre-alpha checklist](docs/testing/pre-alpha-release.md) passes.

## Using OverCrow

Start a selected game and focus its window. The default shortcuts are:

| Shortcut | Action |
| --- | --- |
| `Meta+Alt+O` | Open or close the interactive overlay. |
| `Esc` | Return an open overlay to passive mode. |
| `Meta+Alt+P` | Start or pause the manual stopwatch. |
| `Meta+Alt+R` | Reset the manual stopwatch. |

The overlay menu lets you enable, move, resize, scale, and reset widgets.
Passive widgets are read-only and click-through. Interactive mode captures
input only over the authorized game and always retains a close path.

Useful diagnostics:

```sh
overcrowctl status
overcrowctl logs
./scripts/diagnose.sh
```

Logs stay local under
`${XDG_STATE_HOME:-$HOME/.local/state}/overcrow/logs/`. They rotate, are bounded,
and exclude titles, paths, keystrokes, notes, media metadata, and provider
payloads. See [troubleshooting](docs/troubleshooting.md) for recovery steps.

## Built-in widgets

- **Session** — elapsed time since the detected game process started.
- **Clock** — local date and time.
- **Performance** — host CPU, memory, and available temperatures.
- **Manual stopwatch** — Core-owned timer with overlay controls and shortcuts.
- **Media** — current MPRIS media with interactive playback controls.
- **Notes** — a private local note and checklist.
- **Warframe status** — open-world cycles, daily reset, and Baro Ki'Teer.
- **Fissures** — current Void Fissures with local filters.
- **Market** — warframe.market search, orders, and trade templates.
- **Sortie & Archon** — the current three Sortie and Archon Hunt missions.
- **Invasions** — current invasion missions, progress, and rewards.

Warframe widgets appear only for Steam App ID `230410`. They use bounded,
unauthenticated requests to the official Warframe world-state endpoint and
warframe.market; they never access a game account or game memory.

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

- Linux support currently means native Arch/Omarchy packaging and the display
  matrix above. DEB and RPM packages are planned.
- A selected game must be focused before a passive overlay can appear.
- Compositor-level placement can briefly lag during geometry changes.
- Performance telemetry describes host/game resource use, not injected frame
  timing; OverCrow does not currently provide a true FPS counter.
- Repository tests cannot prove live Hyprland, Plasma, X11, Proton, or game
  behavior. Those paths require the [manual MVP checklist](docs/testing/manual-mvp.md).

## Architecture

OverCrow separates lifecycle, game authority, rendering, and compositor
integration into small Rust processes. State is delivered through versioned
D-Bus snapshots and bounded/coalescing events, with limited reconciliation for
missed notifications.

See [docs/architecture.md](docs/architecture.md) for the process model,
display contracts, persistence, providers, and security boundaries.

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
