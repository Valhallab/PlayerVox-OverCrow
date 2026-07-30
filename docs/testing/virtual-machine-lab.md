# Virtual machine laboratory

This laboratory exercises OverCrow's supported display contracts on isolated
Linux guests. It supplements the real Arch/Hyprland baseline; it does not prove
physical GPU performance or driver compatibility.

## Scope and limitations

| Environment | Display path | Resources |
| --- | --- | --- |
| Bazzite KDE Desktop | Plasma Wayland | 6 vCPU, 10 GiB RAM, 100 GiB qcow2 |
| Xubuntu 24.04 LTS | XFCE X11 and DEB build baseline | 4 vCPU, 6 GiB RAM, 80 GiB qcow2 |
| Ubuntu 24.04 LTS | GNOME 46 Wayland and DEB runtime | 4 vCPU, 8 GiB RAM, 80 GiB qcow2 |
| Debian 13 KDE | Plasma Wayland | 4 vCPU, 8 GiB RAM, 80 GiB qcow2 |
| CachyOS KDE | Plasma Wayland and Plasma X11 | 4 vCPU, 8 GiB RAM, 80 GiB qcow2 |
| CachyOS XFCE | XFCE X11 | 4 vCPU, 6 GiB RAM, 60 GiB qcow2 |

Run one guest at a time. Sway, Gamescope, exclusive fullscreen, GPU passthrough,
physical multi-monitor behavior, and performance benchmarking are out of scope.

## Host preflight

The host uses the system libvirt connection and default NAT network:

```sh
LC_ALL=C lscpu | rg '^Virtualization:[[:space:]]+AMD-V$'
test -r /dev/kvm
sudo pacman -S --needed qemu-desktop libvirt virt-manager edk2-ovmf dnsmasq
sudo usermod -aG libvirt grmpy
sudo systemctl enable --now libvirtd.socket
```

Log out and back in if `groups` does not include `libvirt`. Then initialize the
network if needed:

```sh
if ! virsh -c qemu:///system net-info default >/dev/null 2>&1; then
  sudo virsh net-define /usr/share/libvirt/networks/default.xml
fi
sudo virsh net-autostart default
if ! virsh -c qemu:///system net-info default | rg -q '^Active:[[:space:]]+yes$'; then
  sudo virsh net-start default
fi
virsh -c qemu:///system list --all
```

Do not weaken libvirt socket permissions or bridge a guest to the LAN.

## Official installation images

Store verified media below `/var/lib/libvirt/boot/overcrow/`.

For Bazzite, use the [official image picker](https://bazzite.gg/#image-picker):

- Desktop or Laptop;
- AMD or Intel GPU;
- KDE Plasma;
- no Steam Gaming Mode.

Download `bazzite-stable-amd64.iso` and its
`bazzite-stable-amd64.iso-CHECKSUM` file. Run
`sha256sum --check bazzite-stable-amd64.iso-CHECKSUM` before copying it.

For CachyOS, use the current official Desktop ISO and checksum from the
[download validation page](https://wiki.cachyos.org/cachyos_basic/download/).
The two CachyOS guests reuse the same verified ISO.

For the DEB build baseline, use the official Xubuntu 24.04 LTS desktop image
and its published SHA256 checksum. Use the official Ubuntu 24.04 LTS desktop
image for GNOME 46. For Plasma 6 validation, use the official Debian 13 KDE
image and checksum. Kubuntu 24.04 is not suitable because it ships Plasma 5,
while OverCrow's KWin bridge targets Plasma 6.

```sh
sudo install -d -m 0755 /var/lib/libvirt/boot/overcrow
sudo install -m 0644 "$HOME/Downloads/bazzite-stable-amd64.iso" \
  /var/lib/libvirt/boot/overcrow/bazzite-stable-amd64.iso
sudo install -m 0644 "$HOME/Downloads/cachyos-desktop-linux-260628.iso" \
  /var/lib/libvirt/boot/overcrow/cachyos-desktop-linux-260628.iso
```

## Common virtual hardware

Every domain uses:

- `qemu:///system`;
- x86_64 UEFI without Secure Boot;
- Q35 machine type;
- host-passthrough CPU;
- VirtIO disk and NAT network;
- SPICE with no external listener;
- VirtIO video with OpenGL and 3D acceleration;
- no host filesystem share.

The original Bazzite and CachyOS `virt-install` definitions are recorded in
[the implementation plan](../plans/2026-07-28-virtual-machine-lab-implementation.md).
The Xubuntu, Ubuntu GNOME, and Debian guests follow the same hardware contract.
Inspect every resulting domain:

```sh
for domain in \
  overcrow-bazzite-kde \
  overcrow-xubuntu-x11 \
  overcrow-ubuntu-gnome46 \
  overcrow-debian-kde \
  overcrow-cachyos-kde \
  overcrow-cachyos-xfce
do
  virsh -c qemu:///system dominfo "$domain"
  virsh -c qemu:///system dumpxml "$domain" | \
    rg "type='spice'|type='virtio'|accel3d='yes'|network='default'"
done
```

## Bazzite KDE guest

Install regular Bazzite KDE Desktop using automatic Btrfs partitioning. Do not
select Steam Gaming Mode or disk encryption for this disposable guest.

After updating:

```sh
printf '%s\n' "$XDG_SESSION_TYPE" "$XDG_CURRENT_DESKTOP"
rpm-ostree status
glxinfo -B | rg 'direct rendering|OpenGL renderer'
```

The session must be Plasma Wayland and the renderer must identify virgl or
VirtIO, not llvmpipe.

Build the native RPM in a Fedora toolbox matching `rpm -E %fedora`. In
addition to the normal build dependencies, the toolbox needs `rpm-build`,
`redhat-rpm-config`, and `zstd`. Install the resulting package as a persistent
layer:

```sh
toolbox run --container overcrow-build sh -lc \
  'cd "$HOME/OverCrow" && ./scripts/build-rpm-package.sh'
rpm_file=$(find "$HOME/OverCrow/dist" -maxdepth 1 -type f \
  -name 'overcrow-*.fc42.x86_64.rpm' -print -quit)
sudo rpm-ostree install "$rpm_file"
systemctl reboot
```

After reboot, `rpm -q overcrow` and `rpm -V --nomtime overcrow` must succeed.
The `--nomtime` exception is limited to timestamps normalized by OSTree; every
content, ownership, permission, and digest check remains enabled. The three
OverCrow user services must remain inactive until the user opts in.

## Xubuntu 24.04 guest

Install Xubuntu 24.04 LTS, update it, and provision Rust stable plus Node.js 22.
Install the native build dependencies without recommended extras:

```sh
sudo apt update
sudo apt install --no-install-recommends \
  build-essential dpkg-dev fontconfig libgl1-mesa-dev \
  libwebkit2gtk-4.1-dev libwayland-dev libx11-dev libx11-xcb-dev \
  libxcb1-dev libxkbcommon-dev pkg-config
cargo install cargo-about --version 0.9.1 --locked
```

Build and inspect the package as the regular desktop user:

```sh
cd "$HOME/OverCrow"
./scripts/build-deb-package.sh
dpkg-deb --info dist/overcrow_0.1.0~pre.alpha.5-1_amd64.deb
sudo apt install ./dist/overcrow_0.1.0~pre.alpha.5-1_amd64.deb
```

The package must install without running or enabling OverCrow. Native-window
tracking, exact geometry, stacking, interactive clicks, shortcuts, and focus
restoration have passed in this guest. Steam/Proton, reboot persistence,
HiDPI, multi-monitor, and physical-GPU checks remain before the Ubuntu/XFCE
path can leave experimental status.

## Ubuntu 24.04 GNOME guest

Install the same Ubuntu-baseline DEB used for release acceptance. Confirm
`XDG_SESSION_TYPE=wayland` and GNOME Shell 46. After the first package
installation, log out and back in once so the running Shell discovers the
system extension, then complete onboarding and run the GNOME section of the
[manual MVP checklist](manual-mvp.md).

## Debian 13 KDE guest

Install the already-built DEB on a clean Debian 13 KDE system. Do not rebuild
it there: the release artifact must remain the one produced on Ubuntu 24.04.
Complete onboarding, Plasma 6 Wayland overlay, upgrade, and removal checks with
that identical file before describing Debian 13 as validated.

## CachyOS KDE guest

Install only KDE Plasma. Add the alternate X11 session and guest tools:

```sh
sudo pacman -Syu
sudo pacman -S --needed qemu-guest-agent spice-vdagent \
  plasma-x11-session kwin-x11 xorg-server mesa-utils steam
sudo systemctl enable --now qemu-guest-agent.service
```

Validate Plasma Wayland, then log out, choose Plasma X11, and repeat:

```sh
printf '%s\n' "$XDG_SESSION_TYPE" "$XDG_CURRENT_DESKTOP"
glxinfo -B | rg 'direct rendering|OpenGL renderer'
```

## CachyOS XFCE guest

Install only XFCE, then:

```sh
sudo pacman -Syu
sudo pacman -S --needed qemu-guest-agent spice-vdagent mesa-utils steam
sudo systemctl enable --now qemu-guest-agent.service
printf '%s\n' "$XDG_SESSION_TYPE" "$XDG_CURRENT_DESKTOP"
glxinfo -B | rg 'direct rendering|OpenGL renderer'
```

The session must be X11/XFCE with a virgl or VirtIO renderer.

## OverCrow test installation

Use the same commit in every guest:

```sh
git clone https://github.com/Valhallab/PlayerVox-OverCrow.git "$HOME/OverCrow"
cd "$HOME/OverCrow"
git pull --ff-only origin master
git rev-parse HEAD
```

On CachyOS:

```sh
sudo pacman -S --needed base-devel rustup nodejs npm zstd \
  webkit2gtk-4.1 libayatana-appindicator
rustup default stable
cargo install cargo-about --version 0.9.1 --locked
./scripts/build-arch-package.sh
sudo pacman -U ./dist/overcrow-bin-*.pkg.tar.zst
```

The Bazzite RPM build is a maintainer test procedure. End users should consume
a signed repository package once Fedora distribution is published.

## Snapshots and reset

Create `clean-os` after the updated system, guest agent, network, and virgl are
working. Do not include Steam credentials.

Create `overcrow-ready` after installing the tested commit but before
onboarding. Power every guest off before taking the snapshot; the Bazzite RPM
layer persists normally across reboots.

```sh
virsh -c qemu:///system snapshot-list overcrow-bazzite-kde
virsh -c qemu:///system snapshot-list overcrow-xubuntu-x11
virsh -c qemu:///system snapshot-list overcrow-debian-kde
virsh -c qemu:///system snapshot-list overcrow-cachyos-kde
virsh -c qemu:///system snapshot-list overcrow-cachyos-xfce
```

Restore `clean-os` for installation tests and `overcrow-ready` for application
tests. Never share a snapshot containing Steam or Twitch credentials.

## Acceptance run

Use Steam App ID `1536610`, OpenTTD, as the lightweight test window:

```sh
steam 'steam://install/1536610'
```

Run its native Linux build, then force the current stable Proton from Steam's
Compatibility settings and run the Windows build. Complete the relevant
[manual MVP checklist](manual-mvp.md) for every session and record the result
in [the VM ledger](vm-lab-results.md).

Required areas are onboarding, game discovery, passive click-through,
interactive input capture, shortcuts, widget and game resizing, virtual
desktops, 100%/150%/200% scaling, game exit, service restart, logout/login, and
diagnostics.

## Troubleshooting

- If the installer cannot start with 3D enabled, install with unaccelerated
  VirtIO, then enable 3D before creating `clean-os`.
- If `glxinfo` reports llvmpipe, do not record overlay behavior as a valid
  accelerated result.
- If a guest cannot reach the network, check
  `virsh -c qemu:///system net-info default`.
- If Bazzite boots a newer broken deployment, select the previous OSTree
  deployment before attributing the failure to OverCrow. Confirm the booted
  base with `rpm-ostree status`, then layer the RPM on that exact base.
- Save only sanitized `overcrowctl logs` and the shortest reproduction
  sequence. Never copy notes, chat, Twitch identity, Steam credentials, or
  private host paths into this repository.
