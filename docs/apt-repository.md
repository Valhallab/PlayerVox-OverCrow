# Signed APT repository

This document defines the initial APT distribution design for PlayerVox
OverCrow. It is intentionally limited to the existing Ubuntu 24.04-baseline
`amd64` package.

## Scope

- Host a standard signed APT repository on GitHub Pages at
  `https://valhallab.github.io/PlayerVox-OverCrow/`.
- Publish the `overcrow` binary package in the `stable/main` suite.
- Let users install and update through `apt install overcrow` and normal APT
  upgrades.
- Keep GitHub Releases as the immutable source of release artifacts.
- Do not add another hosting account, package service, source package, or
  architecture in this iteration.

## Repository layout

The public `gh-pages` branch is independent from `master` and contains only
generated repository data:

```text
.
├── .nojekyll
├── dists/stable/InRelease
├── dists/stable/Release
├── dists/stable/Release.gpg
├── dists/stable/main/binary-amd64/Packages
├── dists/stable/main/binary-amd64/Packages.gz
├── dists/stable/main/binary-amd64/by-hash/SHA256/...
├── keyrings/playervox-overcrow-archive-keyring.gpg
├── playervox-overcrow.sources
└── pool/main/o/overcrow/overcrow_<version>_amd64.deb
```

`Release` identifies the archive as `PlayerVox OverCrow`, declares only
`stable`, `main`, and `amd64`, enables by-hash retrieval, and includes SHA-256
hashes for every index. It does not expire while the project is in pre-alpha.

## Signing boundary

A dedicated RSA OpenPGP archive key signs `InRelease` and `Release.gpg`.
The key is used only for this APT repository and is kept in the maintainer's
local GnuPG keyring. Its private material, passphrase, and exported secret key
must never enter this repository, GitHub Actions, release assets, logs, or
temporary project files.

The public key is exported in binary form under `keyrings/`. Its full
fingerprint is documented in the README and release instructions so users can
verify it through an independent GitHub page before trusting the repository.
Key rotation requires an overlap release signed by both the old and new keys
and an explicit migration procedure.

APT repository signatures authenticate the package indices and their recorded
package SHA-256 values. The `.deb` does not require a separate embedded
signature.

## Controlled publication

A local maintainer script receives an exact release version and the validated
DEB from `dist/release`. It:

1. verifies the file is regular, non-empty, not a symlink, and inside the
   approved release directory;
2. checks its SHA-256 against `SHA256SUMS` and validates its package name,
   version, architecture, and control metadata;
3. creates an isolated temporary checkout of the current `gh-pages` branch;
4. copies the package into its canonical pool path without replacing a
   different file with the same version;
5. regenerates deterministic `Packages`, compressed and by-hash indices;
6. generates `Release`, then signs `InRelease` and `Release.gpg` with the
   explicitly selected archive-key fingerprint;
7. verifies all hashes and signatures with a clean temporary GnuPG home;
8. creates one publication commit.

The script never invokes `sudo`, edits the live APT configuration, installs
packages, imports secret keys, or pushes implicitly. A separate explicit
`--push` action publishes the already verified commit.

Older packages remain in `pool/` and in the index so users can deliberately
downgrade during pre-alpha testing. Repository growth can be reviewed before
the first stable release.

## User configuration

The repository publishes a Deb822 source definition using:

```text
Types: deb
URIs: https://valhallab.github.io/PlayerVox-OverCrow/
Suites: stable
Components: main
Architectures: amd64
Signed-By: /usr/share/keyrings/playervox-overcrow-archive-keyring.gpg
```

Installation documentation downloads the public key and source definition as
separate files, then runs `apt update` and `apt install overcrow`. It does not
use `apt-key`, disable signature verification, or pipe a remote script into a
privileged shell.

## Failure handling

Publication fails closed when the release artifact, checksum, package
identity, repository history, signing key, signature, generated hash, or
GitHub Pages target is missing or ambiguous. Temporary directories are private
and removed on exit. Existing repository content is never modified in place.

GitHub Pages is enabled once for the root of `gh-pages`. The release process
must verify the public `InRelease`, public key, source definition, package
index, and DEB URL after each push before announcing APT availability.

## Validation

Automated tests cover:

- version and path rejection;
- package identity and checksum enforcement;
- deterministic index generation;
- by-hash consistency;
- isolated signature verification;
- refusal to overwrite conflicting package content;
- absence of private-key material and implicit pushes;
- README and CI command drift.

A final Ubuntu 24.04 VM acceptance test adds the published source with
`Signed-By`, installs OverCrow, runs `apt update`, and confirms that a newer
test repository version is offered as an upgrade. Repository tests do not
modify the host's live APT configuration.
