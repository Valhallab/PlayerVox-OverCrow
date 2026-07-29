# Fedora RPM packaging design

## Goal

Produce one native x86_64 RPM that installs the complete PlayerVox OverCrow
runtime on Fedora 42 and Bazzite. Validate it on the existing Bazzite Plasma
Wayland VM through a persistent `rpm-ostree` deployment.

## Package

- Name: `overcrow`
- Version: the exact Cargo workspace version
- Architecture: `x86_64`
- License: `AGPL-3.0-only`
- Payload: the existing reviewed `packaging/release/stage.sh` `/usr` tree
- Installation: inert; no service is enabled or started by RPM scriptlets
- Output: one application RPM, without separate debug or source artifacts in
  the user-facing `dist/` directory

RPM automatic ELF dependency discovery supplies library requirements. The spec
adds only non-library runtime requirements that the application directly
expects. Plasma and Hyprland remain optional desktop integrations.

## Build flow

`scripts/build-rpm-package.sh` mirrors the existing Arch build flow:

1. validate tools, version, and non-root execution;
2. build the Control Center frontend and locked Rust workspace inside the
   matching Fedora environment;
3. generate third-party notices;
4. create the canonical release stage and verify its checked-in manifest;
5. render a bounded RPM spec and build it in a temporary rpmbuild tree;
6. verify the RPM identity, architecture, payload, dependencies, and file
   permissions;
7. atomically publish exactly one RPM into `dist/`.

No build step installs, starts, or modifies the live OverCrow session.

## Bazzite acceptance

1. Stop OverCrow and remove its temporary KWin integration.
2. Reboot, which discards the development-only `rpm-ostree usroverlay`.
3. Confirm that no temporary OverCrow executable or service file remains.
4. Build the RPM in the Fedora 42 Distrobox.
5. Run `sudo rpm-ostree install <absolute-rpm-path>` and reboot.
6. Confirm `rpm -q overcrow`, launch the Control Center from Plasma, and repeat
   the game, focus, shortcut, resize, widget, and Warframe-catalog checks.
7. Reboot once more and verify that the installed application remains usable.

User configuration and local state are preserved during package removal. This
matches standard package-manager behavior and also validates upgrades from an
existing OverCrow profile. A separate explicit settings reset can test first
launch later without coupling private data deletion to RPM lifecycle scripts.

## Safety and rollback

- The RPM contains only paths present in the canonical release manifest.
- It never writes user configuration or compositor files during installation.
- Existing integration security checks remain the only path that installs a
  per-user KWin or Hyprland bridge.
- Rollback is `sudo rpm-ostree uninstall overcrow`, followed by a reboot.
- A failed RPM validation is never published into `dist/`.
