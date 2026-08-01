# Launchpad PPA Design

## Goal

Publish PlayerVox OverCrow from `ppa:valhallab/overcrow` as a genuine Ubuntu
source package so Ubuntu users need only `add-apt-repository` and `apt install`.
The existing Arch, RPM, standalone DEB, and signed APT artifacts remain
unchanged.

## Build boundary

Launchpad receives source, never an OverCrow binary. The source package contains
the tracked repository, the locked Cargo dependency sources, generated frontend
assets, and generated third-party notices. The frontend assets and Rust plus
production-JavaScript notices are prepared locally from checked-in lockfiles
because Launchpad builders do not have Internet access. Launchpad recompiles
every Rust binary from the vendored sources with Cargo offline.

Ubuntu 24.04 does not yet carry the Rust 1.92 toolchain required by eframe 0.35
in its normal archive. The PPA therefore uses Canonical's
`ppa:rust-toolchain/staging` only as a build dependency. This keeps the current
renderer intact and does not add that PPA to end-user systems. The generated
package explicitly invokes `/usr/bin/cargo-1.92` and
`/usr/bin/rustc-1.92`.

## Source-package structure

- `scripts/build-ppa-source.sh` creates, validates, optionally signs, and
  atomically publishes one source upload set. It never uploads or installs.
- `packaging/ppa/debian/` contains the reviewed Debian source metadata and
  offline build rules.
- The existing `packaging/release/stage.sh` and release manifest remain the
  only runtime-payload authority.
- `scripts/lib/release-version.sh` owns PPA version normalization. For example,
  pre-alpha 5 revision 1 becomes
  `0.1.0~pre.alpha.5+ppa1-1~noble1`, which upgrades the existing standalone
  `0.1.0~pre.alpha.5-1` package. Including the PPA revision in the upstream
  version also gives each immutable Launchpad original archive a unique name.
- Signed source artifacts use the dedicated Valhallab Launchpad upload key
  `6425BB0DBE7933E086EE420B2789BF4BF0C19541`. Development smoke builds may be
  explicitly unsigned.

## Safety and reproducibility

The source builder requires a clean checkout, derives timestamps from the
exported commit, fixes its file-creation mask, uses `npm ci`,
`cargo vendor --locked`, deterministic tar metadata, fixed package identities,
and a private temporary directory. It rejects symlinked or missing generated
inputs and never deletes unrelated artifacts. Upload remains a separate
explicit `dput` action.

The binary package starts and enables no OverCrow service during installation.
It reuses the same payload and runtime dependencies as the validated standalone
DEB. Its Debian rules suppress Debhelper's automatic user-service hook so the
Control Center remains the only lifecycle authority. `Rules-Requires-Root: no`
keeps compilation unprivileged.

## Validation and publication

Fast smoke tests validate version ordering, metadata, offline Cargo use,
canonical staging, fixed signing identity, and absence of installation or
upload commands. Hosted CI runs only those inexpensive checks. Before upload,
the complete source set is built and inspected on the Ubuntu 24.04 packaging
guest, then uploaded manually to Launchpad. The README switches its primary
Ubuntu instructions to the PPA only after Launchpad publishes a successful
amd64 build.
