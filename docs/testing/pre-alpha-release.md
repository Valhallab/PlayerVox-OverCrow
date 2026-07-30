# Pre-alpha release acceptance

Run this checklist on the real Arch, Fedora/Bazzite, and Ubuntu desktop before
publishing `v0.1.0-pre-alpha.5`. Check only results you personally observe.

## 1. Candidate integrity

```sh
cd /home/grmpy/Projects/OverCrow/dist/release
sha256sum -c SHA256SUMS
ls -lh
```

- [ ] All three package checksums report `OK`.
- [ ] The directory contains only the Arch package, Fedora 42 RPM,
  Ubuntu-baseline DEB, and `SHA256SUMS`.

## 2. Native installation

```sh
sudo pacman -Rns overcrow-bin 2>/dev/null || true
sudo pacman -U overcrow-bin-0.1.0prealpha5-1-x86_64.pkg.tar.zst
overcrow-control
```

On Fedora 43 or 44 enable COPR with
`sudo dnf copr enable grmpy/playervox-overcrow`, then use
`sudo dnf install overcrow`. On Bazzite based on Fedora 43 or 44 use
`sudo dnf5 copr enable grmpy/playervox-overcrow`, then
`sudo rpm-ostree install overcrow` and reboot.

Fedora/Bazzite 42 are no longer available as new COPR build targets. The
standalone Fedora 42 RPM remains the fallback:
`sudo dnf install ./overcrow-0.1.0.pre_alpha.5-1.fc42.x86_64.rpm`.

On the Xubuntu 24.04 X11 validation guest:

```sh
sudo apt remove overcrow 2>/dev/null || true
sudo apt install ./overcrow_0.1.0~pre.alpha.5-1_amd64.deb
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

Use `sudo dnf remove overcrow` on Fedora or `sudo apt remove overcrow` on
Ubuntu/Debian. On Bazzite use
`sudo rpm-ostree uninstall overcrow`, then reboot.

- [ ] No OverCrow runtime process remains.
- [ ] The application launcher no longer contains PlayerVox OverCrow.

User settings are intentionally preserved under
`${XDG_CONFIG_HOME:-$HOME/.config}/overcrow/`.

## 4. DEB validation

Build only inside the clean Ubuntu 24.04 x86_64 guest:

```sh
./scripts/build-deb-package.sh
dpkg-deb --info dist/overcrow_0.1.0~pre.alpha.5-1_amd64.deb
dpkg-deb --contents dist/overcrow_0.1.0~pre.alpha.5-1_amd64.deb
```

- [ ] The builder produces exactly one non-symlinked `amd64.deb`.
- [ ] Package identity is `overcrow`, version `0.1.0~pre.alpha.5-1`,
  architecture `amd64`.
- [ ] Installation performs no service activation or compositor mutation.
- [ ] A fresh Xubuntu 24.04 X11 session completes the installation and overlay
  checklist.
- [ ] A fresh Debian 13 Plasma 6 Wayland session completes the same checklist
  with the identical DEB.
- [ ] Removal leaves no runtime process while preserving user settings.

## 5. COPR publication

The Fedora builder also produces
`dist/overcrow-0.1.0.pre_alpha.5-1.fc42.src.rpm`. Verify that source package
locally, then submit it explicitly:

```sh
rpm -qpi dist/overcrow-0.1.0.pre_alpha.5-1.fc42.src.rpm
rpm -qpl dist/overcrow-0.1.0.pre_alpha.5-1.fc42.src.rpm
copr-cli build grmpy/playervox-overcrow \
  dist/overcrow-0.1.0.pre_alpha.5-1.fc42.src.rpm
```

- [ ] COPR reports a successful build.
- [ ] The public repository exposes `overcrow` at the expected version.
- [ ] A clean Fedora system can install it by package name.
- [ ] A Fedora 43/44 or matching Bazzite system can resolve it through its
  native package transaction.
