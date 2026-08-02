# OverCrow

OverCrow is an opt-in external Linux game overlay built by Valhallab and
distributed under the PlayerVox brand. It adds movable widgets without
injection. Hyprland and Plasma 6 Wayland are supported; GNOME Wayland and X11
are available as experimental integrations.

## Compatibility

| Environment | Status | Notes |
| --- | --- | --- |
| Hyprland 0.55+ Wayland | **Supported** | Primary Arch/Omarchy target; native Wayland and XWayland games. |
| Plasma 6 Wayland | **Supported** | Validated on Bazzite KDE/Fedora 42; native Wayland and XWayland games through the KWin bridge. |
| XFCE X11 on Xubuntu 24.04 | **Experimental — validated in VM** | DEB installation, native-window tracking, geometry, stacking, interactive clicks, shortcuts, and focus restoration passed. Steam/Proton, physical GPU, HiDPI, and multi-monitor validation remain. |
| Other X11/EWMH desktops | **Experimental — for now** | The shared X11 path is implemented, but Plasma X11 and other window managers still need real-machine validation. |
| GNOME 46 Wayland | **Experimental — validated in VM** | Overlay placement, passive click-through, interactive input, and `Meta+Alt+O` passed on Ubuntu 24.04. Steam/Proton, physical GPU, HiDPI, and multi-monitor validation remain. |
| GNOME 47–50 Wayland | **Experimental — for now** | The packaged Shell/Mutter bridge declares compatibility, but these Shell versions still need real-machine validation. |
| Sway Wayland | **Not compatible — for now** | Requires a dedicated Sway IPC and layer-shell bridge. |
| Gamescope / Steam Deck Game Mode | **Not compatible — for now** | No nested-compositor integration yet. |
| Other Wayland compositors | **Not compatible — for now** | A transparent window alone cannot provide safe placement and input control. |
| Windows host | **Not compatible — for now** | OverCrow is Linux-only; Windows games may run through Steam/Proton on a supported Linux desktop. |

Windowed and borderless-fullscreen games are supported. Exclusive fullscreen
may bypass compositor windows and is outside the current design.

## Install

> **Pre-alpha · x86_64 only.** Installation never starts OverCrow. The runtime
> stays disabled until you open the Control Center, select a game, and choose
> **Start**.

### Arch Linux · AUR

```sh
yay -S overcrow-bin
```

`paru -S overcrow-bin` works too.

### Fedora 43–44 · COPR + DNF

```sh
sudo dnf copr enable grmpy/playervox-overcrow
sudo dnf install overcrow
```

### Bazzite 43–44 · COPR + rpm-ostree

```sh
sudo dnf5 copr enable grmpy/playervox-overcrow
sudo rpm-ostree install overcrow
systemctl reboot
```

### Ubuntu 24.04 · Launchpad PPA + APT

```sh
sudo add-apt-repository ppa:valhallab/overcrow
sudo apt install overcrow
```

Linux Mint, Debian, newer Ubuntu releases, and other Debian-family desktops are
not yet validated.

### Direct package fallback

The [latest pre-alpha release](https://github.com/Valhallab/PlayerVox-OverCrow/releases/tag/v0.1.0-pre-alpha.5)
contains one Arch package, one Fedora 42 RPM, one Ubuntu 24.04-baseline DEB,
and `SHA256SUMS`:

- **Arch:** `sudo pacman -U ./overcrow-bin-*.pkg.tar.zst`
- **Fedora 42:** `sudo dnf install ./overcrow-*.rpm`
- **Bazzite 42:** `sudo rpm-ostree install ./overcrow-*.rpm`, then reboot.
- **Ubuntu 24.04:** `sudo apt install ./overcrow_0.1.0~pre.alpha.5-1_amd64.deb`

### First launch

Open **PlayerVox OverCrow** from the application menu, or run:

```sh
overcrow-control
```

OverCrow starts disabled. Select at least one detected game, then enable the
runtime from the System Status card. Runtime services and shortcuts become
available only for explicitly selected games.

For a non-Steam Windows game, add it to Steam, force Proton, launch it once,
then select **Refresh games**. The native picker accepts Linux executables only.
Known Steam tools and runtimes are hidden; uncertain entries remain selectable
with a **Type unverified** label.

Closing the Control Center keeps OverCrow available from the tray. **Quit**
disables the runtime before exiting.

### Uninstall

Choose tray **Quit**, then use the command for your installation:

- **Arch:** `sudo pacman -Rns overcrow-bin`
- **Fedora:** `sudo dnf remove overcrow`
- **Ubuntu:** `sudo apt remove overcrow`
- **Bazzite:** `sudo rpm-ostree uninstall overcrow`, then reboot.

User settings are deliberately left in `${XDG_CONFIG_HOME:-$HOME/.config}/overcrow/`.

## Updates

The Control Center checks official GitHub releases at startup and at most every
six hours; **About → Check for updates** also checks manually. Nothing is
downloaded until **Update now** is selected. OverCrow verifies the package's
GitHub SHA-256 digest, stops its runtime, then asks PolicyKit to run the native
pacman, dnf, apt, or rpm-ostree transaction. Native installations offer **Restart now** after a successful update; Bazzite requires a system reboot.
Source and unrecognized installations open the official release page instead.
Cancelling or failing a step leaves the current package unchanged.

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

GNOME Wayland uses the packaged OverCrow Shell extension to register the same
shortcuts only while a selected game is active. The Control Center enables this
extension during explicit setup; package installation alone never enables it.
After the first package installation, GNOME may require one logout/login before
the current Shell session discovers the system extension.

The PlayerVox-styled **Widget library** groups general tools separately from
game-specific widgets. It lets you enable, move, scale, and reset widgets
without leaving the overlay; layouts that benefit from a fixed panel also
provide a resize grip. Passive widgets are read-only and click-through.
Interactive mode captures input only over the authorized game and always
retains a close path.

Useful diagnostics are available in the Control Center. The CLI equivalents are
`overcrowctl status` and `overcrowctl logs`; source checkouts can additionally
run `./scripts/diagnose.sh`.

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
- **Discord voice (experimental)** — voice participants, speaking state,
  mute/deafen status, and bounded avatars from the Discord desktop client.

### Warframe

- **Warframe status** — open-world cycles, daily reset, and Baro Ki'Teer.
- **Fissures** — current Void Fissures with local filters.
- **Market** — warframe.market search, orders, and trade templates.
- **Sortie & Archon** — the current three Sortie and Archon Hunt missions.
- **Invasions** — current invasion missions, progress, and rewards.

Warframe widgets appear only for Steam App ID `230410`. Their bounded public
requests never access a game account or game memory.

Resizable widgets keep their configured editing size in Interactive mode; in
Passive mode their height follows visible content up to the game viewport.

Twitch chat connects only for an active selected game. Messages stay in memory;
credentials use Secret Service and the release contains no Client Secret.

Discord voice requests only `identify` and `rpc` after **Connect Discord**. It
uses the desktop client's native RPC socket and a stateless PlayerVox OAuth
broker, so no Client Secret ships in OverCrow. Tokens use Secret Service with a
memory-only fallback; voice state and avatars are never persisted.

## Safety

OverCrow stays outside the game process. It does **not**:

- inject DLLs or shared objects;
- install Vulkan layers or use `LD_PRELOAD`;
- hook graphics APIs, use `ptrace`, or read game memory;
- inspect packets, synthesize input, or modify game files.

It uses same-user metadata, compositor APIs, and D-Bus; ambiguity fails closed.

This design is anti-cheat-friendly but cannot guarantee future compatibility;
follow each game's current rules.

Security issues should be reported privately as described in
[SECURITY.md](SECURITY.md), not in a public issue.

## Limitations

- Distribution is currently through the AUR, Fedora COPR, the Ubuntu PPA, and
  direct release packages.
- Non-Steam Windows games must be launched as Steam shortcuts through Proton;
  direct Wine executable matching is not supported.
- A selected game must be focused before a passive overlay can appear.
- Compositor-level placement can briefly lag during geometry changes.
- Performance telemetry describes host/game resource use, not injected frame
  timing; OverCrow does not currently provide a true FPS counter.
- Repository tests cannot prove live Hyprland, Plasma, GNOME, X11, Proton, or
  game behavior. Those paths require the
  [manual MVP checklist](docs/testing/manual-mvp.md).

## Architecture

Small Rust processes separate lifecycle, game authority, rendering, and display
integration. See [docs/architecture.md](docs/architecture.md) for details.

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

Forks can override the compiled public Twitch application ID with the
`OVERCROW_TWITCH_CLIENT_ID` build variable.

Official builds embed a public Discord application ID. Forks can override it
through `OVERCROW_DISCORD_CLIENT_ID` and must use their own broker credentials.

Shell, KWin, packaging, and release checks are documented in
[AGENTS.md](AGENTS.md). Contributions should stay focused and preserve the
external-window safety model; see [CONTRIBUTING.md](CONTRIBUTING.md).

## License

OverCrow is [AGPL-3.0-only](LICENSE), created by Valhallab SASU, and distributed under PlayerVox.
See [NOTICE](NOTICE) and [TRADEMARKS.md](TRADEMARKS.md); dependencies retain their licenses.

Proprietary-use [commercial licenses](COMMERCIAL-LICENSING.md) are available from
Valhallab via [contact@valhallab.com](mailto:contact@valhallab.com); no other rights are granted.
