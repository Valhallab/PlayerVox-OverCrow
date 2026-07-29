# Debian Package Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build one validated Ubuntu 24.04-baseline `amd64` DEB and include it
in OverCrow release candidates.

**Architecture:** Reuse the existing release stage as the only runtime payload
source. A small Debian renderer creates reviewed control metadata, while the
builder derives ELF library dependencies with `dpkg-shlibdeps`, constructs the
root-owned archive with `dpkg-deb`, and verifies it before atomic publication.

**Tech Stack:** POSIX shell, Debian `dpkg-dev`, existing Rust/npm release
pipeline, ShellCheck.

## Global Constraints

- Build only as an unprivileged `x86_64` user on Ubuntu 24.04.
- Produce exactly `overcrow_<package-version>_amd64.deb`, where the package
  version already includes Debian revision `-1`.
- Install inertly: no maintainer scripts, triggers, service activation, or
  compositor mutation.
- Preserve the shared release manifest; only
  `usr/share/doc/overcrow/copyright` is Debian-specific.
- Do not publish, install, tag, push, or create an APT repository.

---

### Task 1: Debian version normalization

**Files:**
- Modify: `scripts/lib/release-version.sh`
- Create: `tests/deb-package-smoke.sh`

**Interfaces:**
- Produces: `overcrow_deb_upstream_version VERSION`
- Produces: `overcrow_deb_package_version VERSION`

- [ ] **Step 1: Add failing normalization tests**

The smoke test sources the version helper and requires:

```sh
test "$(overcrow_deb_upstream_version 0.1.0-pre-alpha.4)" = \
    '0.1.0~pre.alpha.4'
test "$(overcrow_deb_package_version 0.1.0-pre-alpha.4)" = \
    '0.1.0~pre.alpha.4-1'
test "$(overcrow_deb_package_version 1.2.3)" = '1.2.3-1'
! overcrow_deb_package_version '01.2.3' >/dev/null 2>&1
```

- [ ] **Step 2: Verify RED**

Run: `tests/deb-package-smoke.sh`

Expected: failure because `overcrow_deb_upstream_version` is unavailable.

- [ ] **Step 3: Implement the two helpers**

Use the existing semantic-version validator. For pre-releases, replace `-`
inside the prerelease with `.` and prefix the complete prerelease with `~`.
Append Debian revision `-1` only in the package-version helper.

- [ ] **Step 4: Verify GREEN**

Run: `tests/deb-package-smoke.sh`

Expected: version assertions pass and the test advances to the next missing
packaging file.

### Task 2: Reviewed Debian metadata

**Files:**
- Create: `packaging/deb/control.in`
- Create: `packaging/deb/copyright`
- Create: `packaging/deb/render-control.sh`
- Modify: `tests/deb-package-smoke.sh`

**Interfaces:**
- Consumes: validated Cargo version, dependency line, installed size, output.
- Produces: one mode-`0644` `DEBIAN/control` file.

- [ ] **Step 1: Add failing renderer tests**

Require exact package identity, `amd64`, Valhallab contact, PlayerVox
homepage, AGPL license metadata, fixed description, exact token replacement,
and rejection of invalid versions, dependency lines, sizes, relative output
paths, symlink outputs, and unresolved tokens.

- [ ] **Step 2: Verify RED**

Run: `tests/deb-package-smoke.sh`

Expected: failure because `packaging/deb/render-control.sh` is missing.

- [ ] **Step 3: Add the fixed template and renderer**

The control template contains:

```text
Package: overcrow
Version: @DEB_VERSION@
Section: games
Priority: optional
Architecture: amd64
Maintainer: Valhallab <contact@valhallab.com>
Installed-Size: @INSTALLED_SIZE@
Depends: @DEPENDS@
Homepage: https://overcrow.playervox.com
Description: Opt-in external Linux game overlay by PlayerVox
 PlayerVox OverCrow provides movable widgets without injecting code into
 game processes. It remains disabled until the user selects a game and starts
 the runtime from the Control Center.
```

The renderer validates every input before replacing the three fixed tokens and
publishes through a private temporary file in the destination directory.

- [ ] **Step 4: Verify GREEN**

Run: `tests/deb-package-smoke.sh`

Expected: metadata and rejection-path assertions pass.

### Task 3: Bounded DEB builder and package inspection

**Files:**
- Create: `scripts/build-deb-package.sh`
- Modify: `tests/deb-package-smoke.sh`
- Modify: `AGENTS.md`

**Interfaces:**
- Consumes: the Cargo workspace, npm UI, shared release stage, Debian renderer.
- Produces: `dist/overcrow_<version>-1_amd64.deb`.

- [ ] **Step 1: Add failing static builder checks**

Require the builder to:

- reject root, non-`x86_64`, and non-Ubuntu-24.04 hosts;
- require `dpkg-deb` and `dpkg-shlibdeps`;
- use `npm ci`, locked Cargo fetch/build, path remapping, and
  `SOURCE_DATE_EPOCH`;
- stage through `packaging/release/stage.sh`;
- compare the payload to the release manifest plus the copyright path;
- derive dependencies using all five shipped ELF binaries;
- include the dynamically loaded display/tray libraries plus `systemd` and
  `xdg-desktop-portal`;
- call `dpkg-deb --root-owner-group`;
- reject maintainer scripts, symlinks, writable payloads, wrong metadata,
  unexpected files, and multiple artifacts;
- publish through a private temporary file without installing anything.

- [ ] **Step 2: Verify RED**

Run: `tests/deb-package-smoke.sh`

Expected: failure because `scripts/build-deb-package.sh` is missing.

- [ ] **Step 3: Implement the builder**

Mirror the proven Arch/RPM build stages, but construct a `DEBIAN` directory,
render the control file from generated dependencies, build one XZ-compressed
archive, inspect it with `dpkg-deb`, and atomically publish it to `dist/`.
Keep all temporary state under one mode-`0700` directory with signal cleanup.

- [ ] **Step 4: Verify GREEN and shell quality**

Run:

```sh
tests/deb-package-smoke.sh
shellcheck scripts/build-deb-package.sh packaging/deb/render-control.sh \
  tests/deb-package-smoke.sh
sh -n scripts/build-deb-package.sh packaging/deb/render-control.sh \
  tests/deb-package-smoke.sh
```

Expected: all commands exit zero.

### Task 4: Release and public documentation integration

**Files:**
- Modify: `packaging/release/assemble.sh`
- Modify: `packaging/release/inspect.sh`
- Modify: `scripts/prepare-release.sh`
- Modify: `.github/workflows/ci.yml`
- Modify: `tests/ci-workflow-smoke.sh`
- Modify: `tests/native-only-distribution-smoke.sh`
- Modify: `tests/public-docs-smoke.sh`
- Modify: `README.md`
- Modify: `docs/testing/pre-alpha-release.md`
- Modify: `docs/testing/virtual-machine-lab.md`
- Modify: `packaging/release/RELEASE_NOTES-pre-alpha.md`

**Interfaces:**
- Consumes: one already validated DEB in `dist/`.
- Produces: a release directory containing Arch, RPM, DEB, and checksums.

- [ ] **Step 1: Extend smoke tests first**

Require the release scripts to normalize the Debian version, reject a missing,
empty, symlinked, wrongly named, wrongly versioned, or wrong-architecture DEB,
copy exactly one DEB, and include it in `SHA256SUMS`. Require hosted CI to run
the cheap static DEB smoke test without building release binaries.

- [ ] **Step 2: Verify RED**

Run:

```sh
tests/deb-package-smoke.sh
tests/ci-workflow-smoke.sh
tests/native-only-distribution-smoke.sh
tests/public-docs-smoke.sh
```

Expected: failures on missing release and documentation integration.

- [ ] **Step 3: Implement release integration and concise documentation**

Update the release assembler, inspector, preparation gate, CI command lists,
quick-start text, release checklist, VM build instructions, and pre-alpha
limitations. Document direct `.deb` installation only; do not claim an APT
repository or verified Debian-family compatibility before live acceptance.

- [ ] **Step 4: Run focused validation**

Run all four smoke tests, ShellCheck, `sh -n`, and `git diff --check`.

Expected: all commands exit zero.

### Task 5: Final verification and Ubuntu handoff

**Files:**
- No production changes unless verification exposes a defect.

- [ ] **Step 1: Run repository gates**

Run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets --locked
shellcheck scripts/*.sh scripts/lib/*.sh tests/*.sh \
  packaging/arch/*.install packaging/arch/*.sh packaging/aur/*.install \
  packaging/deb/*.sh packaging/release/*.sh packaging/rpm/*.sh
sh -n scripts/*.sh scripts/lib/*.sh tests/*.sh \
  packaging/arch/*.install packaging/arch/*.sh packaging/aur/*.install \
  packaging/deb/*.sh packaging/release/*.sh packaging/rpm/*.sh
for smoke_test in tests/*-smoke.sh; do "$smoke_test"; done
git diff --check
git status --short --branch
```

- [ ] **Step 2: Provide the exact Ubuntu 24.04 build command**

The real package build must run in the user's Ubuntu 24.04 VM:

```sh
./scripts/build-deb-package.sh
```

Do not claim package acceptance, alter the host installation, publish a
release, or create the APT repository before the user reports the VM result.
