# PlayerVox OverCrow 0.1.0 — Pre-alpha 4

This is the fourth public technical preview of PlayerVox OverCrow. Expect bugs,
rough edges, and compatibility gaps. Please disable OverCrow and attach the
sanitized Diagnostics logs when reporting a problem.

## What changed

- Native Fedora 42 RPM validated end-to-end on Bazzite KDE
- Stabilized Plasma 6 Wayland placement, focus, input, and KWin integration
- Native X11/EWMH shortcuts, focus tracking, HiDPI geometry, and fail-closed recovery
- NVIDIA Wayland startup compatibility and GPU detection in the Control Center
- Game-specific widgets stay hidden when their matching game is not active

OverCrow does not inject code, hook graphics APIs, read game memory, inspect
packets, or modify game files.

## Compatibility

- Linux x86_64 only
- Hyprland 0.55+ Wayland: primary validated target
- Plasma 6 Wayland: supported and validated on Bazzite KDE/Fedora 42
- X11/EWMH: experimental
- GNOME, Sway, Gamescope, and exclusive fullscreen: not compatible for now

## Installation

Install `overcrow-bin` on Arch or the Fedora 42 `overcrow` RPM for the complete
application, runtime, overlay, and compositor integrations.

See the [pre-alpha acceptance checklist](https://github.com/Valhallab/PlayerVox-OverCrow/blob/master/docs/testing/pre-alpha-release.md)
before publication.

## Known limitations

- No true in-game FPS counter
- Windowed and borderless-fullscreen games only
- Display behavior still needs broader X11, hardware, and scaling tests
- DEB, ARM64, GNOME, Sway, and Gamescope support are not included
