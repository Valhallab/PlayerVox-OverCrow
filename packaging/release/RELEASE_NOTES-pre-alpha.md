# PlayerVox OverCrow 0.1.0 — Pre-alpha 5

This is the fifth public technical preview of PlayerVox OverCrow. Expect bugs,
rough edges, and compatibility gaps. Please disable OverCrow and attach the
sanitized Diagnostics logs when reporting a problem.

## What changed

- Secure, digest-verified native updates through pacman, dnf, apt, and rpm-ostree
- One-click Control Center restart after native updates; Bazzite keeps its reboot flow
- Experimental GNOME 46–50 Shell/Mutter bridge, with GNOME 46 exercised in VM
- Ubuntu 24.04-baseline DEB and stronger X11/EWMH focus, geometry, and shortcut handling
- Steam shortcut discovery for Proton games, with tools filtered and games sorted

OverCrow does not inject code, hook graphics APIs, read game memory, inspect
packets, or modify game files.

## Compatibility

- Linux x86_64 only
- Hyprland 0.55+ Wayland: primary validated target
- Plasma 6 Wayland: supported and validated on Bazzite KDE/Fedora 42
- XFCE X11: experimental native-window validation passed on Xubuntu 24.04
- Other X11/EWMH desktops: experimental and awaiting broader validation
- GNOME 46 Wayland: experimental validation passed in an Ubuntu 24.04 VM
- GNOME 47–50: experimental and awaiting real-machine validation
- Sway, Gamescope, and exclusive fullscreen: not compatible for now

## Installation

Install `overcrow-bin` on Arch, the Fedora 42 `overcrow` RPM, or the
Ubuntu-baseline `amd64.deb` for the complete application, runtime, overlay, and
compositor integrations.

See the [pre-alpha acceptance checklist](https://github.com/Valhallab/PlayerVox-OverCrow/blob/master/docs/testing/pre-alpha-release.md)
for installation and validation details.

## Known limitations

- No true in-game FPS counter
- Windowed and borderless-fullscreen games only
- Display behavior still needs broader X11, hardware, and scaling tests
- The DEB still needs Steam/Proton and broader Ubuntu, Mint, and Debian
  desktop validation
- ARM64, Sway, and Gamescope support are not included
