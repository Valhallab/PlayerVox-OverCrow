# Debian Package Design

## Goal

Produce one native `amd64` DEB for PlayerVox OverCrow that can be installed on
Ubuntu 24.04 or newer, Linux Mint 22 or newer, and Debian 13 or newer. The
Ubuntu 24.04 build environment is the compatibility baseline.

The first delivery is a direct GitHub Release artifact. A signed APT repository
is intentionally deferred until the same package passes real-machine
acceptance on Ubuntu and Debian-family systems.

## Package contract

- Package name: `overcrow`
- Architecture: `amd64`
- Maintainer: `Valhallab <contact@valhallab.com>`
- Homepage: `https://overcrow.playervox.com`
- Source: `https://github.com/Valhallab/PlayerVox-OverCrow`
- Version mapping: `0.1.0-pre-alpha.4` becomes
  `0.1.0~pre.alpha.4-1`; Debian's `~` keeps pre-releases below `0.1.0`.
- Artifact name: `overcrow_0.1.0~pre.alpha.4-1_amd64.deb`

The package installs the same reviewed runtime payload used by Arch and RPM.
It adds only Debian control metadata and the Debian copyright document. It
contains no maintainer scripts, triggers, service activation, compositor
mutation, or user configuration.

## Build

`scripts/build-deb-package.sh` runs only as an unprivileged user on an exact
Ubuntu 24.04 x86_64 build host. It:

1. validates required Debian tools and the workspace version;
2. builds the frontend and release Rust binaries from the checked-out source;
3. stages the shared release payload through `packaging/release/stage.sh`;
4. verifies the payload against `packaging/release/manifest.txt`;
5. derives shared-library dependencies with `dpkg-shlibdeps`;
6. adds the explicit libraries loaded dynamically by the tray and display
   backends, plus the `systemd` and `xdg-desktop-portal` runtime services;
7. creates the package with root ownership using `dpkg-deb`;
8. inspects package identity, payload, permissions, scripts, and dependencies;
9. atomically publishes exactly one DEB into `dist/`.

The build uses `SOURCE_DATE_EPOCH` when provided and never runs package
installation commands.

## Dependencies

Direct shared-library dependencies are generated on the Ubuntu 24.04 baseline.
Libraries loaded with `dlopen` cannot be discovered from ELF metadata, so the
Ayatana tray, EGL/GL, Wayland, X11/XCB, and xkbcommon runtimes remain explicit.
The systemd-backed D-Bus user session, `systemd`, and `xdg-desktop-portal` are
also explicit because OverCrow invokes their services without directly linking
their libraries.

Hyprland and Plasma packages are not hard dependencies: only one compositor is
used at runtime, and the Control Center already fails closed when its required
bridge is unavailable.

## Release integration

The release assembler and inspector require exactly:

- one Arch package;
- one Fedora 42 RPM;
- one Ubuntu-baseline DEB;
- `SHA256SUMS`.

`scripts/prepare-release.sh` accepts only a previously validated DEB with the
expected version, architecture, and filename, matching the existing
cross-distribution RPM workflow.

## Validation

Automated coverage checks:

- Cargo-to-Debian version normalization and ordering;
- exact package metadata and architecture;
- exact shared payload plus the approved Debian copyright path;
- absence of maintainer scripts and unsafe permissions;
- dependency generation and required runtime dependencies;
- deterministic artifact naming and bounded atomic publication;
- release assembly, documentation, ShellCheck, and syntax integration.

Real-machine acceptance remains mandatory before publication:

1. clean Xubuntu 24.04 X11 build, installation, and removal;
2. Control Center onboarding and inert-by-default state;
3. Debian 13 Plasma 6 Wayland overlay activation for an explicitly selected
   Steam game;
4. upgrade from the prior package while preserving user settings;
5. honest compatibility status on unsupported Debian-family desktops.

The package will not be described as validated for those distributions until
these checks are reported successful.
