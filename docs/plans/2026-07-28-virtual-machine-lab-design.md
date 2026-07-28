# OverCrow Virtual Machine Lab Design

## Purpose

Build a small, repeatable virtual-machine laboratory for functional validation
of OverCrow across its supported display paths. The laboratory complements, but
does not replace, testing on physical gaming hardware.

The initial scope validates OverCrow from source. Native RPM and DEB packaging,
GPU passthrough, GNOME, Sway, and Gamescope support are not part of this work.

## Coverage matrix

| Environment | Role | Display path |
| --- | --- | --- |
| Existing Arch/Omarchy host | Primary real-machine baseline | Hyprland Wayland |
| Bazzite KDE Desktop VM | Mandatory gaming-distribution and atomic-system case | Plasma Wayland through the KWin bridge |
| CachyOS KDE VM | Arch gaming distribution and alternate Plasma session | Plasma Wayland and Plasma X11 |
| CachyOS XFCE VM | Independent EWMH window-manager case | XFCE X11 |

Bazzite uses its regular KDE Desktop image. The Deck image is excluded because
its Steam Gaming Mode uses Gamescope, which OverCrow does not currently support.
The X11 session in the CachyOS KDE guest is installed explicitly and selected
at login.

Fedora KDE and Debian XFCE are deferred until native RPM and DEB packaging is in
scope. Their current display coverage would mostly duplicate the selected
guests while adding package and build-system variables unrelated to this pass.

## Host architecture

Use the existing QEMU/KVM, libvirt, and virt-manager installation through the
`qemu:///system` connection. The host has AMD-V, 16 logical CPUs, 32 GiB of
memory, and sufficient disk space for the proposed guests.

Only one guest should run at a time.

| Guest | vCPUs | Memory | Sparse qcow2 disk |
| --- | ---: | ---: | ---: |
| Bazzite KDE | 6 | 10 GiB | 100 GiB |
| CachyOS KDE | 4 | 8 GiB | 80 GiB |
| CachyOS XFCE | 4 | 6 GiB | 60 GiB |

Each guest uses:

- x86_64 UEFI firmware without Secure Boot for the initial pass;
- a VirtIO disk and network adapter;
- a VirtIO video device with 3D acceleration and an OpenGL-enabled SPICE
  display;
- libvirt's default NAT network;
- no host home-directory or repository share;
- a clean clone pinned to the exact commit under test.

Installation images must come from the distribution's official source and have
their published checksum or signature verified before use.

## Installation strategy

### CachyOS

Build OverCrow inside each guest, assemble the existing Arch package, and
install that local package. This exercises the normal installed layout without
requiring publication to the AUR for every test commit.

### Bazzite

Build against the matching Fedora userspace in a local development container,
then stage the resulting release layout on the Bazzite host. Use
`rpm-ostree usroverlay` to make `/usr` writable only for the current boot.
Rebooting discards the OverCrow files, which keeps this development-only
installation reversible.

Required runtime libraries are checked explicitly before launching OverCrow.
Missing host libraries must be installed through Bazzite's supported
`rpm-ostree` package-layering mechanism, not copied from another distribution.

The Bazzite procedure is strictly a test installation. A native RPM remains the
proper future distribution mechanism.

## Snapshot policy

Create two named snapshots for every guest:

- `clean-os`: updated distribution, guest tools, working network, and verified
  3D acceleration, before OverCrow dependencies or files;
- `overcrow-ready`: dependencies and the selected OverCrow commit installed,
  before application configuration.

Settings and logs remain inside the guest. Restore `overcrow-ready` before a
repeat test and `clean-os` before testing installation behavior. Never reuse a
configured user home as a clean-install baseline.

The Bazzite `overcrow-ready` snapshot is a running full-system snapshot. It
captures guest memory as well as the qcow2 disk because the transient `/usr`
overlay intentionally disappears on reboot. The two CachyOS ready snapshots
are conventional powered-off snapshots.

## Acceptance workflow

Record the guest identity, session type, desktop, OverCrow commit, and result
for every run. Each relevant case is marked `PASS`, `FAIL`, or `BLOCKED`.

The common workflow covers:

1. launch and first-run onboarding;
2. explicit game selection and lifecycle start/stop;
3. Steam discovery with one native game and one Proton game;
4. passive visibility and click-through;
5. interactive input capture, shortcut handling, and the emergency close path;
6. game and widget move/resize behavior;
7. virtual-desktop transitions;
8. 100%, 150%, and 200% display scaling;
9. game exit, service restart, logout, and login recovery;
10. diagnostics, bounded logs, and absence of crash or restart loops.

Display-specific checks continue to use `docs/testing/manual-mvp.md`. Failures
include the sanitized diagnostic output and the smallest reproducible action
sequence; no private Steam, Twitch, note, or chat content is copied into the
repository.

## Boundaries

VirtIO/virgl is suitable for functional compositor and application testing but
cannot validate:

- AMD or NVIDIA proprietary-driver behavior;
- physical GPU temperatures, performance, latency, or frame pacing;
- exclusive fullscreen;
- real multi-monitor topology and hardware-specific fractional scaling;
- anti-cheat behavior under production hardware conditions.

Those remain real-machine acceptance items. A VM result must not be presented
as proof of physical GPU or gaming performance compatibility.

Unsupported sessions must fail closed. GNOME, Sway, other generic Wayland
compositors, and Gamescope are not promoted by this laboratory.

## Recovery and safety

- Use libvirt NAT rather than a bridged interface.
- Do not mount host user data into a guest.
- Do not store Steam credentials in snapshots intended for sharing.
- Stop a guest cleanly before restoring a snapshot.
- If graphics acceleration prevents an installer from starting, temporarily
  use unaccelerated VirtIO for installation, then re-enable 3D before creating
  `clean-os`.
- A Bazzite reboot removes the transient `/usr` overlay. Persistent layered
  dependencies are removed by restoring `clean-os`.
- Host changes are limited to libvirt access, its default NAT network, and VM
  storage. No OverCrow service or compositor integration is installed on the
  host by the VM setup.

## Deliverables

The implementation pass will provide:

- the host libvirt bootstrap and verification commands;
- verified download locations for the selected current ISO images;
- exact virt-manager settings for all three guests;
- guest-specific source-build and temporary-install commands;
- a compact reusable result matrix tied to the manual MVP checklist.
