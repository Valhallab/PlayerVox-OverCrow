# Pre-alpha release acceptance

Run this checklist on the real Arch desktop before publishing
`v0.1.0-pre-alpha.2`. Check only results you personally observe.

## 1. Candidate integrity

```sh
cd /home/grmpy/Projects/OverCrow/dist/release
sha256sum -c SHA256SUMS
ls -lh
```

- [ ] The package checksum reports `OK`.
- [ ] The directory contains only the Arch package and `SHA256SUMS`.

## 2. Native installation

```sh
sudo pacman -Rns overcrow-bin 2>/dev/null || true
sudo pacman -U overcrow-bin-0.1.0prealpha2-1-x86_64.pkg.tar.zst
overcrow-control
```

- [ ] The app is identified as **PlayerVox OverCrow**.
- [ ] Onboarding correctly identifies the current desktop compatibility.
- [ ] OverCrow remains stopped until a game is selected and **Start** is used.
- [ ] Steam discovery shows valid installed games without stale-library noise.
- [ ] Closing the window keeps one tray icon and **Open Control Center** restores it.
- [ ] Launching `overcrow-control` again restores the same instance.
- [ ] Tray **Start OverCrow** and **Stop OverCrow** stay synchronized with the window.

Select one game, press **Start**, launch and focus the game, then test:

- [ ] `Meta+Alt+O` opens and closes the overlay only for the selected game.
- [ ] Passive widgets remain click-through.
- [ ] Interactive mode blocks game input and `Esc` returns to passive mode.
- [ ] Moving, resizing, and toggling widgets behaves correctly.
- [ ] Quitting the game closes the overlay without trapping input.
- [ ] Diagnostics → Overview reports healthy services.
- [ ] Diagnostics → Logs loads bounded logs and **Refresh** works.
- [ ] **Stop** disables runtime services and shortcuts.

## 3. Clean removal

Choose **Quit** from the tray, then run:

```sh
sudo pacman -Rns overcrow-bin
systemctl --user daemon-reload
pgrep -af '^(.*/)?overcrow-(core|overlay|hyprland)( |$)' || true
```

- [ ] No OverCrow runtime process remains.
- [ ] The application launcher no longer contains PlayerVox OverCrow.

User settings are intentionally preserved under
`${XDG_CONFIG_HOME:-$HOME/.config}/overcrow/`.
