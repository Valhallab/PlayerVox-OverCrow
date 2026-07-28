# OverCrow Virtual Machine Lab Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `subagent-driven-development` or `executing-plans` to implement this plan
> task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create a reproducible three-guest KVM laboratory for functional
OverCrow testing on Bazzite KDE Wayland, CachyOS KDE Wayland/X11, and CachyOS
XFCE X11.

**Architecture:** The Arch/Hyprland workstation hosts isolated libvirt guests
on the default NAT network. Guests use UEFI, sparse qcow2 disks, VirtIO devices,
and virgl acceleration; OverCrow is built from the exact tested commit inside
each guest and restored from clean snapshots between runs.

**Tech Stack:** QEMU/KVM, libvirt, virt-manager/virt-install, qcow2, SPICE,
VirtIO/virgl, Bazzite/rpm-ostree, CachyOS/pacman, Rust, npm, systemd user units.

## Global Constraints

- Keep the existing Arch/Omarchy host as the real Hyprland baseline.
- Run only one guest at a time.
- Use Bazzite KDE Desktop without Steam Gaming Mode.
- Do not use GPU passthrough, bridged networking, or host directory shares.
- Verify official installation images before attaching them to a guest.
- Treat VM results as functional evidence only, never as physical GPU,
  performance, latency, anti-cheat, or exclusive-fullscreen validation.
- Do not push repository commits or install OverCrow on the host.

---

### Task 1: Publish the operator runbook

**Files:**

- Create: `docs/testing/virtual-machine-lab.md`
- Create: `docs/testing/vm-lab-results.md`
- Reference: `docs/testing/manual-mvp.md`
- Reference: `docs/plans/2026-07-28-virtual-machine-lab-design.md`

**Interfaces:**

- Consumes: the approved coverage matrix and acceptance boundaries.
- Produces: one permanent setup guide and one reusable result ledger.

- [ ] **Step 1: Write the laboratory guide**

Create `docs/testing/virtual-machine-lab.md` with these exact sections:

```markdown
# Virtual machine laboratory

## Scope and limitations
## Host preflight
## Official installation images
## Common virtual hardware
## Bazzite KDE guest
## CachyOS KDE guest
## CachyOS XFCE guest
## OverCrow test installation
## Snapshots and reset
## Acceptance run
## Troubleshooting
```

Include the validated resource table, exact VM names, host and guest commands
from Tasks 2–6, official image sources, checksum verification, and links back
to `manual-mvp.md`. State prominently that Gamescope and GPU-driver validation
remain out of scope.

- [ ] **Step 2: Write the result ledger**

Create `docs/testing/vm-lab-results.md` with an empty row for each guest/session:

```markdown
| Environment | Session | Commit | Native game | Proton game | Overlay | Input | Scaling | Recovery | Result |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Bazzite KDE | Plasma Wayland | | | | | | | | |
| CachyOS KDE | Plasma Wayland | | | | | | | | |
| CachyOS KDE | Plasma X11 | | | | | | | | |
| CachyOS XFCE | XFCE X11 | | | | | | | | |
```

Below the table, define the only accepted result values as `PASS`, `FAIL`, and
`BLOCKED`, plus fields for the sanitized diagnostic report and minimal
reproduction sequence.

- [ ] **Step 3: Validate the documentation**

Run:

```sh
test -s docs/testing/virtual-machine-lab.md
test -s docs/testing/vm-lab-results.md
rg -n 'Bazzite KDE|CachyOS KDE|CachyOS XFCE' \
  docs/testing/virtual-machine-lab.md docs/testing/vm-lab-results.md
git diff --check
```

Expected: both files are non-empty, every guest is present, and
`git diff --check` prints nothing.

- [ ] **Step 4: Commit the runbook**

```sh
git add docs/testing/virtual-machine-lab.md docs/testing/vm-lab-results.md
git commit -m "docs(testing): add virtual machine lab runbook"
```

### Task 2: Bootstrap the host hypervisor

**Files:**

- Modify: host package, group, socket, and libvirt network state only.
- Do not modify: OverCrow services or host compositor configuration.

**Interfaces:**

- Consumes: the host's AMD-V support and installed Arch packages.
- Produces: a working unprivileged `qemu:///system` libvirt connection and
  default NAT network.

- [ ] **Step 1: Confirm virtualization support**

Run:

```sh
LC_ALL=C lscpu | rg '^Virtualization:[[:space:]]+AMD-V$'
test -r /dev/kvm
```

Expected: the AMD-V line is printed and `/dev/kvm` is readable.

- [ ] **Step 2: Install only missing host packages**

Run:

```sh
sudo pacman -S --needed qemu-desktop libvirt virt-manager edk2-ovmf dnsmasq
```

Expected: pacman completes without replacing unrelated packages.

- [ ] **Step 3: Enable libvirt access**

Run:

```sh
sudo usermod -aG libvirt grmpy
sudo systemctl enable --now libvirtd.socket
```

If the current shell does not report `libvirt`, log out and back in before
continuing. Do not use a world-writable libvirt socket.

- [ ] **Step 4: Start the default NAT network**

Run:

```sh
if ! virsh -c qemu:///system net-info default >/dev/null 2>&1; then
  sudo virsh net-define /usr/share/libvirt/networks/default.xml
fi
sudo virsh net-autostart default
if ! virsh -c qemu:///system net-info default | rg -q '^Active:[[:space:]]+yes$'; then
  sudo virsh net-start default
fi
```

Expected:

```sh
virsh -c qemu:///system net-info default
```

prints `Active: yes` and `Autostart: yes`.

- [ ] **Step 5: Confirm the system connection**

Run:

```sh
virsh -c qemu:///system list --all
```

Expected: a domain table, including an empty table, without an authentication
or socket error.

### Task 3: Acquire and verify installation media

**Files:**

- Create outside repository: `/var/lib/libvirt/boot/overcrow/`
- Download outside repository: one Bazzite ISO and one CachyOS desktop ISO.

**Interfaces:**

- Consumes: official Bazzite and CachyOS download services.
- Produces: verified read-only installation media accessible to libvirt.

- [ ] **Step 1: Prepare the ISO directory**

Run:

```sh
sudo install -d -m 0755 /var/lib/libvirt/boot/overcrow
```

- [ ] **Step 2: Download Bazzite from the official selector**

Open `https://bazzite.gg/#image-picker` and select:

```text
Hardware: Desktop or Laptop
GPU: AMD or Intel
Desktop environment: KDE Plasma
Steam Gaming Mode: No
```

Download `bazzite-stable.iso` and record the SHA-256 value displayed by the
official selector. Verify the downloaded file before installing it:

```sh
sha256sum "$HOME/Downloads/bazzite-stable.iso"
```

Continue only if it exactly matches the official value, then run:

```sh
sudo install -m 0644 "$HOME/Downloads/bazzite-stable.iso" \
  /var/lib/libvirt/boot/overcrow/bazzite-stable.iso
```

- [ ] **Step 3: Download and verify CachyOS**

Run:

```sh
curl --fail --location --output /tmp/cachyos-desktop-linux-260628.iso \
  https://iso.cachyos.org/desktop/260628/cachyos-desktop-linux-260628.iso
printf '%s  %s\n' \
  '136c84942eacdc6deed205fe7018c69fe7b70757f2f9b4010936ee05e060f336' \
  '/tmp/cachyos-desktop-linux-260628.iso' | sha256sum --check
sudo install -m 0644 /tmp/cachyos-desktop-linux-260628.iso \
  /var/lib/libvirt/boot/overcrow/cachyos-desktop-linux-260628.iso
rm -f /tmp/cachyos-desktop-linux-260628.iso
```

Expected: `cachyos-desktop-linux-260628.iso: OK`.

- [ ] **Step 4: Confirm media ownership and hashes**

Run:

```sh
sudo sha256sum /var/lib/libvirt/boot/overcrow/*.iso
sudo find /var/lib/libvirt/boot/overcrow -maxdepth 1 -type f \
  -name '*.iso' -printf '%f %m %u:%g\n'
```

Expected: two regular ISO files with mode `644`.

### Task 4: Define the three virtual machines

**Files:**

- Create outside repository:
  `/var/lib/libvirt/images/overcrow-bazzite-kde.qcow2`
- Create outside repository:
  `/var/lib/libvirt/images/overcrow-cachyos-kde.qcow2`
- Create outside repository:
  `/var/lib/libvirt/images/overcrow-cachyos-xfce.qcow2`

**Interfaces:**

- Consumes: active libvirt NAT, UEFI firmware, and verified ISOs.
- Produces: three stopped libvirt domains with identical display contracts and
  isolated disks.

- [ ] **Step 1: Define Bazzite KDE**

Run:

```sh
virt-install --connect qemu:///system \
  --name overcrow-bazzite-kde \
  --memory 10240 \
  --vcpus 6 \
  --cpu host-passthrough \
  --boot uefi \
  --disk path=/var/lib/libvirt/images/overcrow-bazzite-kde.qcow2,size=100,format=qcow2,bus=virtio,sparse=yes \
  --network network=default,model=virtio \
  --graphics spice,listen=none,gl.enable=yes \
  --video virtio,model.acceleration.accel3d=yes \
  --channel spicevmc \
  --cdrom /var/lib/libvirt/boot/overcrow/bazzite-stable.iso \
  --osinfo detect=on,require=off \
  --noautoconsole
```

- [ ] **Step 2: Define CachyOS KDE**

Run:

```sh
virt-install --connect qemu:///system \
  --name overcrow-cachyos-kde \
  --memory 8192 \
  --vcpus 4 \
  --cpu host-passthrough \
  --boot uefi \
  --disk path=/var/lib/libvirt/images/overcrow-cachyos-kde.qcow2,size=80,format=qcow2,bus=virtio,sparse=yes \
  --network network=default,model=virtio \
  --graphics spice,listen=none,gl.enable=yes \
  --video virtio,model.acceleration.accel3d=yes \
  --channel spicevmc \
  --cdrom /var/lib/libvirt/boot/overcrow/cachyos-desktop-linux-260628.iso \
  --osinfo detect=on,require=off \
  --noautoconsole
```

- [ ] **Step 3: Define CachyOS XFCE**

Run:

```sh
virt-install --connect qemu:///system \
  --name overcrow-cachyos-xfce \
  --memory 6144 \
  --vcpus 4 \
  --cpu host-passthrough \
  --boot uefi \
  --disk path=/var/lib/libvirt/images/overcrow-cachyos-xfce.qcow2,size=60,format=qcow2,bus=virtio,sparse=yes \
  --network network=default,model=virtio \
  --graphics spice,listen=none,gl.enable=yes \
  --video virtio,model.acceleration.accel3d=yes \
  --channel spicevmc \
  --cdrom /var/lib/libvirt/boot/overcrow/cachyos-desktop-linux-260628.iso \
  --osinfo detect=on,require=off \
  --noautoconsole
```

- [ ] **Step 4: Verify the definitions**

Run:

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

Expected: all domains exist and their XML contains SPICE, VirtIO, 3D
acceleration, and the default network.

### Task 5: Install and baseline the guests

**Files:**

- Modify: guest disks only.
- Create: libvirt snapshots named `clean-os`.

**Interfaces:**

- Consumes: the three bootable domains.
- Produces: updated KDE/Wayland, KDE/Wayland+X11, and XFCE/X11 guests with
  working virgl.

- [ ] **Step 1: Install Bazzite interactively**

Open `overcrow-bazzite-kde` in virt-manager. Use automatic partitioning, Btrfs,
KDE Plasma, and no disk encryption. After reboot, confirm:

```sh
printf '%s\n' "$XDG_SESSION_TYPE" "$XDG_CURRENT_DESKTOP"
rpm-ostree status
```

Expected: `wayland`, Plasma/KDE, and a signed `bazzite:stable` deployment.

- [ ] **Step 2: Install CachyOS KDE interactively**

Install only KDE Plasma with automatic partitioning. After updating, install
the alternate X11 session:

```sh
sudo pacman -Syu
sudo pacman -S --needed qemu-guest-agent spice-vdagent \
  plasma-x11-session kwin-x11 xorg-server mesa-utils steam
sudo systemctl enable --now qemu-guest-agent.service
```

Confirm Plasma Wayland first, then log out and confirm Plasma X11:

```sh
printf '%s\n' "$XDG_SESSION_TYPE" "$XDG_CURRENT_DESKTOP"
```

- [ ] **Step 3: Install CachyOS XFCE interactively**

Install only XFCE with automatic partitioning, then run:

```sh
sudo pacman -Syu
sudo pacman -S --needed qemu-guest-agent spice-vdagent mesa-utils steam
sudo systemctl enable --now qemu-guest-agent.service
printf '%s\n' "$XDG_SESSION_TYPE" "$XDG_CURRENT_DESKTOP"
```

Expected: `x11` and XFCE.

- [ ] **Step 4: Verify virtual 3D in every guest**

Run inside each guest:

```sh
glxinfo -B | rg 'direct rendering|OpenGL renderer'
```

Expected: direct rendering is enabled and the renderer identifies virgl or
VirtIO rather than llvmpipe.

- [ ] **Step 5: Create powered-off clean snapshots**

Shut down all guests, then run on the host:

```sh
for domain in \
  overcrow-bazzite-kde \
  overcrow-cachyos-kde \
  overcrow-cachyos-xfce
do
  virsh -c qemu:///system snapshot-create-as \
    "$domain" clean-os 'Updated OS before OverCrow installation'
done
```

Expected: `virsh -c qemu:///system snapshot-list DOMAIN` shows `clean-os`.

### Task 6: Install OverCrow and execute acceptance

**Files:**

- Modify: guest disks and guest user configuration only.
- Create: libvirt snapshots named `overcrow-ready`.
- Update: `docs/testing/vm-lab-results.md`

**Interfaces:**

- Consumes: clean guests and the exact public OverCrow commit.
- Produces: one acceptance result per supported guest/session.

- [ ] **Step 1: Pin the same source commit in every guest**

Inside each guest:

```sh
git clone https://github.com/Valhallab/PlayerVox-OverCrow.git "$HOME/OverCrow"
cd "$HOME/OverCrow"
git fetch origin master
git checkout master
git pull --ff-only origin master
git rev-parse HEAD
```

Record the printed commit in `docs/testing/vm-lab-results.md`. Do not test
different commits across guests.

- [ ] **Step 2: Build and install on both CachyOS guests**

Inside each CachyOS guest:

```sh
sudo pacman -S --needed base-devel rustup nodejs npm zstd \
  webkit2gtk-4.1 libayatana-appindicator
rustup default stable
cargo install cargo-about --version 0.9.1 --locked
cd "$HOME/OverCrow"
./scripts/build-arch-package.sh
sudo pacman -U ./dist/overcrow-bin-*.pkg.tar.zst
```

Expected: `overcrow-control` starts from `/usr/bin/overcrow-control`.

- [ ] **Step 3: Build and stage temporarily on Bazzite**

Inside Bazzite, create a Fedora toolbox matching the booted base:

```sh
if ! rpm -q webkit2gtk4.1 libayatana-appindicator-gtk3 glx-utils; then
  sudo rpm-ostree install webkit2gtk4.1 \
    libayatana-appindicator-gtk3 glx-utils
  systemctl reboot
fi
FEDORA_VERSION=$(rpm -E %fedora)
toolbox create --container overcrow-build \
  --image "registry.fedoraproject.org/fedora-toolbox:${FEDORA_VERSION}"
toolbox run --container overcrow-build sudo dnf install -y \
  cargo rust nodejs npm gcc gcc-c++ pkgconf-pkg-config \
  gtk3-devel webkit2gtk4.1-devel \
  libayatana-appindicator-gtk3-devel \
  libX11-devel libxcb-devel libxkbcommon-devel wayland-devel \
  openssl-devel alsa-lib-devel
toolbox run --container overcrow-build sh -lc \
  'cd "$HOME/OverCrow/crates/overcrow-control-ui" &&
   npm ci --ignore-scripts --no-audit --no-fund &&
   npm run build'
toolbox run --container overcrow-build sh -lc \
  'cd "$HOME/OverCrow" && cargo build --workspace --release --locked'
sudo rpm-ostree usroverlay
```

Install the built binaries and integration resources into the transient system
layout:

```sh
cd "$HOME/OverCrow"
for binary in \
  overcrow-control overcrow-core overcrow-hyprland overcrow-overlay overcrowctl
do
  sudo install -Dm0755 "target/release/$binary" "/usr/bin/$binary"
done

sudo install -Dm0755 scripts/integrate-user.sh \
  /usr/lib/overcrow/overcrow-integrate
sudo install -Dm0644 scripts/lib/hyprland-config.sh \
  /usr/lib/overcrow/hyprland-config.sh

unit_stage=$(mktemp -d)
for unit in overcrow-core overcrow-hyprland overcrow-overlay
do
  sed 's|@OVERCROW_BINDIR@|/usr/bin|g' \
    "packaging/systemd/$unit.service.in" > "$unit_stage/$unit.service"
  sudo install -Dm0644 "$unit_stage/$unit.service" \
    "/usr/lib/systemd/user/$unit.service"
done
rm -rf -- "$unit_stage"

sudo install -Dm0644 packaging/applications/com.playervox.OverCrow.desktop \
  /usr/share/applications/com.playervox.OverCrow.desktop
sudo install -Dm0644 packaging/metainfo/com.playervox.OverCrow.metainfo.xml \
  /usr/share/metainfo/com.playervox.OverCrow.metainfo.xml
sudo install -Dm0644 \
  crates/overcrow-control-ui/src-tauri/icons/icon.png \
  /usr/share/icons/hicolor/512x512/apps/com.playervox.OverCrow.png

sudo install -Dm0644 integrations/kwin/metadata.json \
  /usr/share/overcrow/integrations/kwin/metadata.json
sudo install -Dm0644 integrations/kwin/contents/code/main.js \
  /usr/share/overcrow/integrations/kwin/contents/code/main.js
sudo install -Dm0644 integrations/hyprland/overcrow.conf.in \
  /usr/share/overcrow/integrations/hyprland/overcrow.conf.in
sudo install -Dm0644 integrations/hyprland/overcrow.lua.in \
  /usr/share/overcrow/integrations/hyprland/overcrow.lua.in
sudo install -Dm0644 TRADEMARKS.md \
  /usr/share/overcrow/TRADEMARKS.md

sudo restorecon -RF \
  /usr/bin/overcrow-control \
  /usr/bin/overcrow-core \
  /usr/bin/overcrow-hyprland \
  /usr/bin/overcrow-overlay \
  /usr/bin/overcrowctl \
  /usr/lib/overcrow \
  /usr/lib/systemd/user/overcrow-core.service \
  /usr/lib/systemd/user/overcrow-hyprland.service \
  /usr/lib/systemd/user/overcrow-overlay.service \
  /usr/share/overcrow

if ldd "$HOME/OverCrow/target/release/overcrow-control" | rg 'not found'; then
  exit 1
fi
test -x /usr/bin/overcrow-control
test -x /usr/bin/overcrow-overlay
test -x /usr/lib/overcrow/overcrow-integrate
```

Expected: no missing library and all three installed executables exist. A
Bazzite reboot intentionally discards these transient `/usr` files.

- [ ] **Step 4: Create ready snapshots**

After dependency and OverCrow installation but before onboarding, leave
Bazzite running so its transient `/usr` mount is preserved. Create an internal
full-system snapshot from the host:

```sh
virsh -c qemu:///system snapshot-create-as \
  overcrow-bazzite-kde overcrow-ready \
  'OverCrow transient install before onboarding'
```

Shut down the two CachyOS guests and run:

```sh
for domain in \
  overcrow-cachyos-kde \
  overcrow-cachyos-xfce
do
  virsh -c qemu:///system snapshot-create-as \
    "$domain" overcrow-ready 'OverCrow installed before onboarding'
done
```

Expected: every domain's snapshot list contains `overcrow-ready`. Reverting
the Bazzite snapshot restores the running guest and its transient mount.

- [ ] **Step 5: Run the acceptance checklist**

For each row in `docs/testing/vm-lab-results.md`, execute the corresponding
section of `docs/testing/manual-mvp.md`, including native Steam, Proton,
passive/interactive input, resize, virtual desktop, scaling, restart, logout,
and diagnostics. Use only `PASS`, `FAIL`, or `BLOCKED`.

Use the free Steam version of OpenTTD, App ID `1536610`, for a reproducible
lightweight game window:

```sh
steam 'steam://install/1536610'
```

Run it once with its native Linux build. Then use Steam Properties →
Compatibility → **Force the use of a specific Steam Play compatibility tool**
and run the Windows build through the current stable Proton. Record both runs
separately. Steam credentials are entered only after `overcrow-ready` exists
and are never included in a clean snapshot intended for sharing.

- [ ] **Step 6: Validate and commit results**

Run on the development host:

```sh
rg -n 'PASS|FAIL|BLOCKED' docs/testing/vm-lab-results.md
git diff --check
git status --short --branch
```

Commit only after at least one real result is recorded:

```sh
git add docs/testing/vm-lab-results.md
git commit -m "test(display): record virtual machine acceptance"
```

Do not push without explicit user authorization.
