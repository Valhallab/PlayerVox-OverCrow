# First Pre-alpha Release

## Goal

Publish the first public PlayerVox OverCrow release as a deliberately early,
reviewable pre-alpha. Preparation is local and publication always requires
explicit approval.

## Release identity

- Product version: `0.1.0-pre-alpha.1`
- Git tag: `v0.1.0-pre-alpha.1`
- GitHub title: `PlayerVox OverCrow 0.1.0 — Pre-alpha 1`
- GitHub state: draft and pre-release
- Supported release architecture: Linux x86_64
- Arch `pkgver`: `0.1.0prealpha1`

## Distribution

The only binary entry point is the complete native Arch package:

- `overcrow-bin-0.1.0prealpha1-1-x86_64.pkg.tar.zst`
- `SHA256SUMS`

The package contains the Control Center, runtime, overlay, user services, and
supported compositor integrations. Users are never asked to choose between
implementation components.

## Local preparation

`./scripts/prepare-release.sh` requires a clean `master`, runs the release
quality gate, builds and inspects the package, and creates `dist/release`
atomically. It never installs software, modifies the live desktop, starts
OverCrow, tags Git, pushes, or publishes a release.

Automated checks do not prove live compositor or game behavior. The real Arch
system must pass `docs/testing/pre-alpha-release.md` before publication.

## Publication

Tagging, pushing, creating a GitHub prerelease, and publishing to AUR are
separate actions requiring explicit approval. DEB, RPM, ARM64, hosted release
builds, signing, GNOME, Sway, and Gamescope remain out of scope.
