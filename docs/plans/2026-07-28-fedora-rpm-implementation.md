# Fedora RPM Packaging Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build one native Fedora 42 RPM and validate its persistent installation
on Bazzite Plasma Wayland.

**Architecture:** Reuse the canonical release stage already consumed by Arch
packaging. A small renderer pins version and payload digest into an RPM spec;
a bounded build script compiles in Fedora, verifies the staged manifest, builds
one binary RPM in a temporary rpmbuild tree, inspects it, and atomically
publishes it to `dist/`.

**Tech Stack:** POSIX shell, RPM spec/rpmbuild, Fedora 42 Distrobox,
rpm-ostree, Rust 2024, npm.

## Global Constraints

- Produce exactly one `overcrow-<version>-1.fc42.x86_64.rpm`.
- Install only files listed in `packaging/release/manifest.txt`.
- Installation must not enable or start OverCrow.
- Do not add debug/source RPMs to `dist/`.
- Do not delete user configuration or state during package removal.
- Preserve X11, Plasma Wayland, and Hyprland runtime payloads.
- Build as a regular user and publish atomically.

---

### Task 1: Render and validate the RPM spec

**Files:**
- Create: `packaging/rpm/overcrow.spec.in`
- Create: `packaging/rpm/render-spec.sh`
- Create: `tests/rpm-package-smoke.sh`

**Interfaces:**
- Consumes: version accepted by `overcrow_version_is_valid`, canonical release
  archive path, and its SHA-256.
- Produces: `render-spec.sh VERSION BUNDLE OUTPUT`, a complete RPM spec with
  fixed package identity and source digest.

- [ ] **Step 1: Write the failing smoke test**

The test creates a minimal source bundle, calls the renderer, and asserts:

```sh
"$renderer" '0.1.0-pre-alpha.3' "$bundle" "$spec"
grep -Fq 'Name:           overcrow' "$spec"
grep -Fq 'Version:        0.1.0~pre_alpha.3' "$spec"
grep -Fq 'Release:        1%{?dist}' "$spec"
grep -Fq '%global debug_package %{nil}' "$spec"
grep -Fq 'PlayerVox OverCrow was installed inertly.' "$spec"
grep -Fq '%{_bindir}/overcrow-control' "$spec"
```

It also rejects invalid versions, symlinked/non-regular bundles, extra
arguments, and pre-existing output paths.

- [ ] **Step 2: Confirm the test fails**

Run: `tests/rpm-package-smoke.sh`

Expected: failure because `packaging/rpm/render-spec.sh` does not exist.

- [ ] **Step 3: Implement the renderer and spec**

The renderer sources `scripts/lib/release-version.sh`, converts
`0.1.0-pre-alpha.3` to RPM-safe `0.1.0~pre_alpha.3`, validates an absolute
regular bundle and absent output, calculates SHA-256, replaces fixed tokens,
and publishes with `install` plus `mv -T -n`.

The spec must contain:

```spec
Name:           overcrow
Version:        @RPM_VERSION@
Release:        1%{?dist}
Summary:        Opt-in external Linux game overlay by PlayerVox
License:        AGPL-3.0-only
URL:            https://github.com/Valhallab/PlayerVox-OverCrow
Source0:        @BUNDLE_NAME@
Source1:        @BUNDLE_NAME@.sha256
BuildArch:      x86_64
Requires:       systemd
Requires:       xdg-desktop-portal
%global debug_package %{nil}
```

`%prep` validates `Source0` against `Source1`; `%install` extracts the canonical
`usr/` tree into `%{buildroot}`. `%files` lists every canonical payload path and
marks license files with `%license`. `%post` prints only inert onboarding copy.

- [ ] **Step 4: Confirm the smoke test passes**

Run: `tests/rpm-package-smoke.sh`

Expected: `RPM package smoke test passed`.

---

### Task 2: Build and inspect one RPM

**Files:**
- Create: `scripts/build-rpm-package.sh`
- Create: `tests/build-rpm-package-smoke.sh`
- Modify: `AGENTS.md`

**Interfaces:**
- Consumes: `packaging/release/stage.sh`,
  `packaging/release/manifest.txt`, and Task 1 renderer.
- Produces: `dist/overcrow-<rpm-version>-1.fc42.x86_64.rpm`.

- [ ] **Step 1: Write the failing build-policy smoke test**

Assert that the script:

```sh
grep -Fq 'cargo build --workspace --release --locked' "$builder"
grep -Fq 'packaging/release/stage.sh' "$builder"
grep -Fq 'packaging/release/manifest.txt' "$builder"
grep -Fq 'rpmbuild -bb' "$builder"
grep -Fq 'rpm -qp' "$builder"
grep -Fq 'rpm -qpl' "$builder"
grep -Fq 'Nothing was installed or started.' "$builder"
```

The test must also fail if the builder references `sudo`, `rpm-ostree`,
`systemctl`, a repository upload, or multiple `dist/` artifacts.

- [ ] **Step 2: Confirm the policy test fails**

Run: `tests/build-rpm-package-smoke.sh`

Expected: failure because `scripts/build-rpm-package.sh` does not exist.

- [ ] **Step 3: Implement the build script**

Follow `scripts/build-arch-package.sh` for tool validation, temporary-directory
cleanup, frontend/Rust builds, notices, canonical staging, manifest comparison,
and atomic publication. Build a deterministic `.tar.zst` payload, render the
spec, and call:

```sh
rpmbuild -bb \
  --define "_topdir $rpm_root" \
  --define "_sourcedir $rpm_root/SOURCES" \
  "$rpm_root/SPECS/overcrow.spec"
```

Require exactly one non-source x86_64 RPM. Before publication, use `rpm -qp`,
`rpm -qpl`, and `rpm -qpR` to verify name, architecture, payload paths, required
dependencies, absence of forbidden scriptlet commands, and the exact release
manifest.

- [ ] **Step 4: Extend repository validation**

Add RPM scripts/spec checks to `AGENTS.md`:

```sh
shellcheck packaging/rpm/*.sh scripts/build-rpm-package.sh \
  tests/rpm-package-smoke.sh tests/build-rpm-package-smoke.sh
sh -n packaging/rpm/*.sh scripts/build-rpm-package.sh \
  tests/rpm-package-smoke.sh tests/build-rpm-package-smoke.sh
```

- [ ] **Step 5: Run focused validation**

Run:

```sh
tests/rpm-package-smoke.sh
tests/build-rpm-package-smoke.sh
shellcheck packaging/rpm/*.sh scripts/build-rpm-package.sh \
  tests/rpm-package-smoke.sh tests/build-rpm-package-smoke.sh
```

Expected: all exit zero.

---

### Task 3: Build and validate on Bazzite

**Files:**
- Modify: `docs/testing/virtual-machine-lab.md`
- Modify: `docs/testing/vm-lab-results.md`

**Interfaces:**
- Consumes: Task 2 RPM and the existing Bazzite Fedora 42 Distrobox.
- Produces: a persistent booted rpm-ostree deployment containing `overcrow`.

- [ ] **Step 1: Run the complete local gate**

Run the Rust, shell, KWin, smoke, diff, and status commands from `AGENTS.md`.
Expected: all exit zero and no unrelated worktree changes.

- [ ] **Step 2: Build the RPM in Fedora 42**

Copy the committed source into the Bazzite VM and run:

```sh
distrobox enter overcrow-build -- \
  bash -lc 'cd /home/bazzite/OverCrow && ./scripts/build-rpm-package.sh'
```

Expected: exactly one `dist/overcrow-*.fc42.x86_64.rpm`.

- [ ] **Step 3: Remove the temporary installation**

Stop OverCrow, remove its temporary KWin package, and reboot. The existing
`rpm-ostree usroverlay` disappears at reboot. Confirm `/usr/bin/overcrow-control`
and `/usr/lib/overcrow/overcrow-integrate` are absent before installing the RPM.

- [ ] **Step 4: Install the persistent RPM**

Run:

```sh
sudo rpm-ostree install /home/bazzite/OverCrow/dist/overcrow-*.fc42.x86_64.rpm
systemctl reboot
```

Expected: the next deployment reports the layered local `overcrow` package.

- [ ] **Step 5: Perform real-machine acceptance**

Confirm:

```sh
rpm -q overcrow
rpm -V overcrow
systemctl --user is-enabled overcrow-core.service
```

Expected: package verification succeeds and services are not enabled before
the user opts in. Launch from Plasma, enable OverCrow, start Rune Dice, and
repeat focus, shortcut, resize, widget placement, Warframe catalog, and reboot
persistence checks. Record only the pass/fail matrix in
`docs/testing/vm-lab-results.md`.

- [ ] **Step 6: Commit the tested implementation**

Commit packaging, tests, and documentation locally. Do not push, publish a
repository, tag, or release.
