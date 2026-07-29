# COPR repository design

## Goal

Publish the validated PlayerVox OverCrow Fedora package through
`grmpy/playervox-overcrow` so Fedora and Bazzite users can install and update it
through their native package tooling.

## Package source

The existing Fedora 42 builder remains the only payload builder. In addition to
the validated binary RPM, it produces a source RPM containing the exact
canonical release bundle, its checksum, and the reviewed RPM spec.

This pre-alpha source RPM deliberately repackages the already validated binary
bundle. It does not introduce a second Rust/npm compilation environment inside
COPR. A fully source-built COPR package can replace it later without changing
the repository or package name.

## Publication

COPR owns repository metadata and package signing. Publication is an explicit
maintainer action using the local COPR credential file; credentials never enter
the repository, scripts, logs, or GitHub Actions.

The project identity is:

- owner: `grmpy`
- project: `playervox-overcrow`
- package: `overcrow`
- targets: Fedora 43 and 44 x86_64 chroots

GitHub release contents remain unchanged: one Arch package, one user-facing RPM,
and `SHA256SUMS`. The SRPM is a maintainer artifact for COPR.

## Validation

The builder must fail closed unless the source RPM has the expected
name/version/release, contains only the rendered spec and its two bounded
sources, and is a direct non-empty regular file. Existing binary RPM payload,
dependency, permission, and inert-scriptlet checks remain mandatory.

Public acceptance requires successful COPR builds for both chroots and a clean
Fedora repository installation that returns the expected OverCrow version.
Live Bazzite installation remains a separate real-machine check.
