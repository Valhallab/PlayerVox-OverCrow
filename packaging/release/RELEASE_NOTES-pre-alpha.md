# PlayerVox OverCrow 0.1.0 — Pre-alpha 3

This is the third public technical preview of PlayerVox OverCrow. Expect bugs,
rough edges, and compatibility gaps. Please disable OverCrow and attach the
sanitized Diagnostics logs when reporting a problem.

## What changed

- Redesigned PlayerVox overlay, widget library, and compact widget controls
- Reliable widget movement, resizing, scaling, passive sizing, and top-left anchoring
- Multi-note workspace with titled tabs, independent notes, and checklists
- Twitch chat with channel favorites, sending, replies, and bounded native emotes
- Private in-app support reporting with an exact user-reviewed preview
- Product version shown in the Control Center, overlay, and About surfaces
- Hardened renderer, provider, portal diagnostics, and bounded external work

OverCrow does not inject code, hook graphics APIs, read game memory, inspect
packets, or modify game files.

## Compatibility

- Linux x86_64 only
- Hyprland 0.55+ Wayland: primary validated target
- Plasma 6 Wayland: implemented, validation in progress
- X11/EWMH: experimental
- GNOME, Sway, Gamescope, and exclusive fullscreen: not compatible for now

## Installation

Install `overcrow-bin` for the complete PlayerVox OverCrow application,
runtime, overlay, and compositor integrations.

See the [pre-alpha acceptance checklist](https://github.com/Valhallab/PlayerVox-OverCrow/blob/master/docs/testing/pre-alpha-release.md)
before publication.

## Known limitations

- No true in-game FPS counter
- Windowed and borderless-fullscreen games only
- Display behavior still needs broader Plasma, X11, hardware, and scaling tests
- DEB, RPM, ARM64, GNOME, Sway, and Gamescope support are not included
