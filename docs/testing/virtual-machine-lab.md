# Virtual machine laboratory

This laboratory exercises OverCrow's supported display contracts on isolated
Linux guests. It supplements the real Arch/Hyprland baseline; it does not prove
physical GPU performance or driver compatibility.

## Scope and limitations

| Environment | Display path | Resources |
| --- | --- | --- |
| Bazzite KDE Desktop | Plasma Wayland | 6 vCPU, 10 GiB RAM, 100 GiB qcow2 |
| CachyOS KDE | Plasma Wayland and Plasma X11 | 4 vCPU, 8 GiB RAM, 80 GiB qcow2 |
| CachyOS XFCE | XFCE X11 | 4 vCPU, 6 GiB RAM, 60 GiB qcow2 |

Run one guest at a time. GNOME, Sway, Gamescope, exclusive fullscreen, GPU
passthrough, physical multi-monitor behavior, and performance benchmarking are
out of scope.

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

The exact `virt-install` definitions are recorded in
[the implementation plan](../plans/2026-07-28-virtual-machine-lab-implementation.md).
Inspect every resulting domain:

```sh
for domain in \
  overcrow-bazzite-kde \
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

OverCrow is built in a Fedora toolbox matching `rpm -E %fedora`. Runtime files
are staged after `sudo rpm-ostree usroverlay`; the installation disappears on
reboot. Keep the guest running when creating its `overcrow-ready` full-system
snapshot so the transient mount is captured. After staging desktop files,
refresh the transient application metadata before starting OverCrow:

```sh
sudo update-desktop-database /usr/share/applications
systemctl --user restart \
  plasma-xdg-desktop-portal-kde.service \
  xdg-desktop-portal.service
```

This refresh is specific to the development-only `/usr` overlay. A native
package installation runs the distribution's normal desktop-database hooks.

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

The Bazzite toolbox and transient staging commands are intentionally kept in
the implementation plan because they are a development-only procedure, not a
supported user installation.

## Snapshots and reset

Create `clean-os` after the updated system, guest agent, network, and virgl are
working. Do not include Steam credentials.

Create `overcrow-ready` after installing the tested commit but before
onboarding. CachyOS snapshots are powered off. Bazzite uses a running
full-system snapshot to preserve its transient `/usr` overlay.

```sh
virsh -c qemu:///system snapshot-list overcrow-bazzite-kde
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
- If a Bazzite reboot removes OverCrow, restore its running
  `overcrow-ready` snapshot or repeat the transient staging procedure.
- Save only sanitized `overcrowctl logs` and the shortest reproduction
  sequence. Never copy notes, chat, Twitch identity, Steam credentials, or
  private host paths into this repository.
