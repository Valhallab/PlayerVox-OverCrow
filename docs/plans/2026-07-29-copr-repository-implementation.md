# COPR Repository Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Produce a validated OverCrow SRPM and publish it through the
`grmpy/playervox-overcrow` COPR repository.

**Architecture:** Extend the existing Fedora RPM build once so the same
canonical bundle produces both the validated user RPM and a maintainer SRPM.
Keep repository publication explicit and keep COPR credentials outside Git.

**Tech Stack:** POSIX shell, RPM/rpmbuild, Fedora COPR, DNF/rpm-ostree.

## Global Constraints

- Preserve the existing Fedora 42 binary RPM and GitHub release layout.
- Do not compile OverCrow again inside COPR during this pre-alpha.
- Never read, copy, print, commit, or upload `~/.config/copr`.
- Do not publish until local packaging checks pass.

---

### Task 1: Specify the SRPM contract

**Files:**
- Modify: `tests/build-rpm-package-smoke.sh`
- Modify: `tests/rpm-package-smoke.sh`

**Interfaces:**
- Consumes: `scripts/build-rpm-package.sh`,
  `packaging/rpm/overcrow.spec.in`
- Produces: smoke-test requirements for a source RPM built from the canonical
  bundle and checksum

- [ ] Add assertions requiring `rpmbuild -ba`, SRPM identity inspection, source
  enumeration, and an explicit SRPM output message.
- [ ] Run both smoke tests and verify they fail because SRPM production is
  absent.

### Task 2: Produce and validate the SRPM

**Files:**
- Modify: `scripts/build-rpm-package.sh`

**Interfaces:**
- Consumes: the rendered RPM spec and canonical release bundle
- Produces: `dist/overcrow-<artifact-version>-1.src.rpm`

- [ ] Replace the binary-only rpmbuild invocation with a binary-and-source
  build.
- [ ] Validate SRPM identity and enumerate its spec, bundle, and checksum.
- [ ] Publish the SRPM atomically beside the user-facing RPM.
- [ ] Run the focused shell checks and both RPM smoke tests.

### Task 3: Document repository installation

**Files:**
- Modify: `README.md`
- Modify: `docs/testing/pre-alpha-release.md`

**Interfaces:**
- Consumes: COPR project `grmpy/playervox-overcrow`
- Produces: Fedora and Bazzite installation/update instructions

- [ ] Replace direct-download-first Fedora instructions with COPR commands,
  retaining the release RPM as a documented fallback.
- [ ] Add the explicit maintainer COPR publication and remote verification
  steps to the release checklist.
- [ ] Run documentation and packaging smoke tests.

### Task 4: Build and publish

**Files:**
- No repository files

**Interfaces:**
- Consumes: validated `dist/*.src.rpm` and local `~/.config/copr`
- Produces: signed COPR repository package `overcrow`

- [ ] Build the Fedora artifacts in the approved Fedora 42 packaging
  environment.
- [ ] Create `grmpy/playervox-overcrow` for the current supported Fedora x86_64
  chroot if it does not exist.
- [ ] Submit the SRPM and wait for the build to finish.
- [ ] Query COPR publicly and verify the expected version is available.
- [ ] Finish with `git diff --check` and `git status --short --branch`.
