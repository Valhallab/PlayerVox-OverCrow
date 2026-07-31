# Signed APT Repository Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish the existing Ubuntu 24.04-baseline `amd64` DEB through a
signed GitHub Pages APT repository.

**Architecture:** A deterministic local builder converts validated release
DEBs into a standard `stable/main` archive and signs its release metadata with
a dedicated local OpenPGP key. A separate publisher fetches the current
`gh-pages` archive, builds and verifies a replacement in a private temporary
directory, and pushes only when explicitly invoked with `--push`.

**Tech Stack:** POSIX shell, `ar`, libarchive `bsdtar`, GNU coreutils, GnuPG,
Git, GitHub Pages, Debian APT repository formats.

## Global Constraints

- Publish only package `overcrow`, suite `stable`, component `main`, and
  architecture `amd64`.
- Read the release artifact only from an absolute validated source directory.
- Use SHA-256 repository hashes, `InRelease`, `Release.gpg`, and
  `Acquire-By-Hash: yes`.
- Never commit, export, print, or upload private key material or its
  passphrase.
- Never invoke `sudo`, edit host APT configuration, install dependencies, or
  push without the explicit `--push` argument.
- Preserve older valid OverCrow DEBs for intentional pre-alpha downgrades.
- Fail closed on symlinks, conflicting package versions, ambiguous keys,
  malformed metadata, dirty publication state, and signature failures.

---

### Task 1: Deterministic signed repository builder

**Files:**

- Create: `packaging/apt/playervox-overcrow.sources`
- Create: `packaging/apt/build-repository.sh`
- Create: `tests/apt-repository-smoke.sh`

**Interfaces:**

- Consumes:
  `build-repository.sh VERSION SOURCE_DIR BASE_REPOSITORY OUTPUT_DIR SIGNING_FINGERPRINT`
- Produces: a new, verified APT repository at the absolute `OUTPUT_DIR`.
- Depends on: `scripts/lib/release-version.sh` for exact DEB version
  normalization.

- [ ] **Step 1: Write a failing smoke test**

Create a minimal valid DEB fixture with `ar` and compressed tar members, an
ephemeral signing key in a private test `GNUPGHOME`, and assertions equivalent
to:

```sh
"$builder" 0.1.0-pre-alpha.5 "$release_dir" "$empty_base" \
    "$repository" "$fingerprint"

test -f "$repository/dists/stable/InRelease"
test -f "$repository/dists/stable/Release.gpg"
test -f "$repository/dists/stable/main/binary-amd64/Packages.gz"
grep -Fqx 'Package: overcrow' \
    "$repository/dists/stable/main/binary-amd64/Packages"
grep -Fqx 'Acquire-By-Hash: yes' \
    "$repository/dists/stable/Release"
GNUPGHOME="$verify_home" gpg --batch --verify \
    "$repository/dists/stable/InRelease"
```

The test must also prove rejection of a bad checksum, wrong package name,
wrong architecture, symlinked artifact, conflicting same-version package,
relative path, existing output, and fingerprint without one signing-capable
secret key.

- [ ] **Step 2: Run the smoke test and confirm failure**

Run:

```sh
tests/apt-repository-smoke.sh
```

Expected: failure because `packaging/apt/build-repository.sh` does not exist.

- [ ] **Step 3: Add the Deb822 source definition**

Create exactly:

```text
Types: deb
URIs: https://valhallab.github.io/PlayerVox-OverCrow/
Suites: stable
Components: main
Architectures: amd64
Signed-By: /usr/share/keyrings/playervox-overcrow-archive-keyring.gpg
```

- [ ] **Step 4: Implement the minimal builder**

The builder must:

1. validate all arguments and normalized filenames;
2. verify `SOURCE_DIR/SHA256SUMS`;
3. read DEB control data using `ar p` and `bsdtar`;
4. validate every retained pool DEB as `overcrow`/`amd64`;
5. produce canonical pool paths and `Packages`;
6. create reproducible `Packages.gz` with `gzip -n`;
7. create SHA-256 by-hash copies;
8. generate `Release` with fixed archive fields and index hashes;
9. export only the selected public key;
10. sign `InRelease` and `Release.gpg`;
11. verify hashes and signatures from an isolated public-only `GNUPGHOME`;
12. add a fixed `.overcrow-generated-apt-repository` ownership marker;
13. atomically rename its private working directory to `OUTPUT_DIR`.

No generated pathname may contain data read from package metadata except the
already validated version.

- [ ] **Step 5: Run focused validation**

Run:

```sh
tests/apt-repository-smoke.sh
shellcheck packaging/apt/build-repository.sh \
  tests/apt-repository-smoke.sh
sh -n packaging/apt/build-repository.sh tests/apt-repository-smoke.sh
```

Expected: all commands succeed.

- [ ] **Step 6: Commit**

```sh
git add packaging/apt tests/apt-repository-smoke.sh
git commit -m "feat(packaging): build signed APT repositories"
```

---

### Task 2: Explicit GitHub Pages publisher

**Files:**

- Create: `scripts/publish-apt-repository.sh`
- Modify: `tests/apt-repository-smoke.sh`

**Interfaces:**

- Consumes:
  `publish-apt-repository.sh VERSION SIGNING_FINGERPRINT [--push]`
- Produces without `--push`: `dist/apt-repository/`.
- Produces with `--push`: one verified commit on remote `gh-pages`.
- Calls: `packaging/apt/build-repository.sh`.

- [ ] **Step 1: Extend the smoke test with a local bare Git remote**

The test must create a local `gh-pages` branch containing one older valid DEB,
then assert:

```sh
"$publisher" 0.1.0-pre-alpha.5 "$fingerprint"
test -d "$project_root/dist/apt-repository"
test "$(git --git-dir="$remote" rev-parse refs/heads/gh-pages)" = \
    "$original_remote_commit"

"$publisher" 0.1.0-pre-alpha.5 "$fingerprint" --push
test "$(git --git-dir="$remote" rev-parse refs/heads/gh-pages)" != \
    "$original_remote_commit"
```

Inject the local remote through an `OVERCROW_APT_REMOTE_URL` test-only
environment variable. Production rejects any override unless the URL is a
local absolute path and `OVERCROW_APT_TEST_MODE=1`.

- [ ] **Step 2: Run the new test and confirm failure**

Run:

```sh
tests/apt-repository-smoke.sh
```

Expected: failure because `scripts/publish-apt-repository.sh` is missing.

- [ ] **Step 3: Implement bounded publication**

The publisher must use:

```sh
remote_url='git@github.com:Valhallab/PlayerVox-OverCrow.git'
branch=gh-pages
```

It clones only that branch into a mode-`0700` temporary directory, treats a
missing branch as an empty initial archive, calls the builder, verifies the
candidate, and writes the candidate to `dist/apt-repository`.

An existing local candidate may be replaced only when it is a real directory,
not a symlink, and contains the exact generated-repository ownership marker.
Any other existing path fails closed.

With `--push`, it replaces only generated archive content in the temporary
checkout, commits as `Valhallab <contact@valhallab.com>`, and pushes
`HEAD:gh-pages`. Git commands use `GIT_TERMINAL_PROMPT=0` and bounded
`timeout`. Without `--push`, no commit or remote write occurs.

- [ ] **Step 4: Run focused validation**

Run:

```sh
tests/apt-repository-smoke.sh
shellcheck scripts/publish-apt-repository.sh \
  packaging/apt/build-repository.sh tests/apt-repository-smoke.sh
sh -n scripts/publish-apt-repository.sh \
  packaging/apt/build-repository.sh tests/apt-repository-smoke.sh
```

Expected: all commands succeed.

- [ ] **Step 5: Commit**

```sh
git add scripts/publish-apt-repository.sh tests/apt-repository-smoke.sh
git commit -m "feat(packaging): publish APT repository explicitly"
```

---

### Task 3: CI and user documentation

**Files:**

- Modify: `.github/workflows/ci.yml`
- Modify: `README.md`
- Modify: `docs/testing/pre-alpha-release.md`
- Modify: `AGENTS.md`
- Modify: `tests/ci-workflow-smoke.sh`
- Modify: `tests/public-docs-smoke.sh`

**Interfaces:**

- CI invokes `tests/apt-repository-smoke.sh`.
- README publishes the exact archive-key fingerprint and installation
  commands.

- [ ] **Step 1: Add failing documentation and CI assertions**

Require the workflow to install `libarchive-tools` and `gnupg`, lint
`packaging/apt/*.sh`, and run `tests/apt-repository-smoke.sh`. Require public
documentation to contain:

```text
https://valhallab.github.io/PlayerVox-OverCrow/
playervox-overcrow-archive-keyring.gpg
sudo apt install overcrow
```

- [ ] **Step 2: Run tests and confirm failure**

Run:

```sh
tests/ci-workflow-smoke.sh
tests/public-docs-smoke.sh
```

Expected: failure because CI and README do not yet expose APT support.

- [ ] **Step 3: Update CI and repository validation guidance**

Add only the two required Ubuntu packages and one APT smoke invocation to the
existing quality job. Extend shell glob validation to `packaging/apt/*.sh`.
Document the focused and release validation commands in `AGENTS.md`.

- [ ] **Step 4: Update public installation and release documentation**

Replace the direct-DEB-only limitation with:

```sh
curl -fsSLo /tmp/playervox-overcrow-archive-keyring.gpg \
  https://valhallab.github.io/PlayerVox-OverCrow/keyrings/playervox-overcrow-archive-keyring.gpg
sudo install -m 0644 /tmp/playervox-overcrow-archive-keyring.gpg \
  /usr/share/keyrings/playervox-overcrow-archive-keyring.gpg
curl -fsSLo /tmp/playervox-overcrow.sources \
  https://valhallab.github.io/PlayerVox-OverCrow/playervox-overcrow.sources
sudo install -m 0644 /tmp/playervox-overcrow.sources \
  /etc/apt/sources.list.d/playervox-overcrow.sources
sudo apt update
sudo apt install overcrow
```

Place the actual full signing fingerprint directly above these commands.
Keep direct GitHub Release installation as a documented fallback.

- [ ] **Step 5: Run focused validation**

```sh
tests/ci-workflow-smoke.sh
tests/public-docs-smoke.sh
tests/apt-repository-smoke.sh
git diff --check
```

Expected: all commands succeed.

- [ ] **Step 6: Commit**

```sh
git add .github/workflows/ci.yml README.md AGENTS.md \
  docs/testing/pre-alpha-release.md tests/ci-workflow-smoke.sh \
  tests/public-docs-smoke.sh
git commit -m "docs(packaging): document signed APT installation"
```

---

### Task 4: Create the archive key and publish pre-alpha 5

**Files:**

- Modify with the generated public fingerprint:
  `README.md`
- Generated and ignored: `dist/apt-repository/`
- Remote-only: `gh-pages`

**Interfaces:**

- Uses: the validated
  `dist/release/overcrow_0.1.0~pre.alpha.5-1_amd64.deb`.
- Publishes:
  `https://valhallab.github.io/PlayerVox-OverCrow/dists/stable/InRelease`.

- [ ] **Step 1: Generate a dedicated protected key**

Run interactively in the maintainer's existing GnuPG home:

```sh
gpg --quick-generate-key \
  'PlayerVox OverCrow APT Repository <contact@valhallab.com>' \
  rsa4096 sign 3y
```

Record the one full fingerprint returned by:

```sh
gpg --batch --with-colons --fingerprint \
  'PlayerVox OverCrow APT Repository <contact@valhallab.com>'
```

- [ ] **Step 2: Insert the exact public fingerprint and re-run checks**

Update README assertions with the full uppercase fingerprint. Run focused
documentation, APT, shell, and diff checks, then commit the fingerprint.

- [ ] **Step 3: Run the complete local quality gate**

Run the full applicable commands from `AGENTS.md`, including Rust, frontend,
shell, dependency policy, all smoke tests, and the remapped release build.
Expected: all succeed.

- [ ] **Step 4: Push source and require green hosted CI**

Push `master`, wait for the exact HEAD workflow run, and require
`completed/success` before publishing `gh-pages`.

- [ ] **Step 5: Build and publish the archive**

```sh
./scripts/publish-apt-repository.sh \
  0.1.0-pre-alpha.5 "$fingerprint"

./scripts/publish-apt-repository.sh \
  0.1.0-pre-alpha.5 "$fingerprint" --push
```

- [ ] **Step 6: Enable GitHub Pages from `gh-pages`**

Use the repository Pages setting with branch `gh-pages` and path `/`. Do not
grant a write-capable Actions workflow or store the signing key on GitHub.

- [ ] **Step 7: Verify the public archive**

Download the public key, source definition, `InRelease`, `Packages.gz`, and
DEB. Verify the key fingerprint, `InRelease` signature, release hashes,
by-hash object, package SHA-256, and exact version.

- [ ] **Step 8: Run Ubuntu 24.04 acceptance**

In the existing disposable Ubuntu VM, install the public key and `.sources`,
then run:

```sh
sudo apt update
apt-cache policy overcrow
sudo apt install overcrow
```

Expected: APT authenticates the archive, selects
`0.1.0~pre.alpha.5-1`, and installs the existing validated DEB.

- [ ] **Step 9: Final state report**

Run:

```sh
git diff --check
git status --short --branch
```

Report the source commits, hosted CI URL, APT URL, signing-key fingerprint,
public package version, and any remaining physical-machine validation.
