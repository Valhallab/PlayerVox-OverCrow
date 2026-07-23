# OverCrow Agent Guide

This file applies to the entire repository. Keep agent work focused, secure,
and proportionate to the request. Prefer completing a small task well over
turning it into a large process.

## Sources of truth

- `README.md`: current product behavior, supported platforms, safety model,
  installation, and developer commands.
- `docs/architecture.md`: process boundaries, event flow, display contracts,
  persistence, providers, and security model.
- `docs/troubleshooting.md`: user-facing diagnostics and safe recovery paths.
- `.github/workflows/ci.yml`: authoritative hosted quality checks.
- `docs/testing/manual-mvp.md`: real compositor/game acceptance procedures.
- `CONTRIBUTING.md`, `LICENSE`, and `TRADEMARKS.md`: ownership and contribution
  policy. Do not import third-party code or assets without authorization and a
  documented license review.

## Project map

- `crates/overcrow-protocol`: shared state and stable D-Bus wire contracts.
- `crates/overcrow-config`: validated settings, allowlists, widget profiles,
  private stores, and migration logic.
- `crates/overcrow-core`: game/window authority, telemetry, shortcuts,
  stopwatch state, session lifecycle, and D-Bus service.
- `crates/overcrow-overlay`: egui renderer, widget manager, local providers,
  and Warframe public-data clients.
- `crates/overcrow-hyprland`: Hyprland IPC, placement, focus protection, and
  dynamic shortcut bridge.
- `crates/overcrow-control`: Control Center, Steam discovery, opt-in settings,
  diagnostics, installation integration, and lifecycle authority.
- `crates/overcrowctl`: small D-Bus CLI.
- `integrations/kwin`: Plasma 6 KWin bridge.
- `packaging/`, `scripts/`, and `tests/`: native Arch packaging, managed user
  integration, diagnostics, and smoke tests.

## Supported display backends

- Preserve all three supported paths: X11 through EWMH, Plasma 6 Wayland
  through the KWin bridge, and Hyprland 0.55+ Wayland through its IPC bridge.
  XWayland games are supported through the active compositor integration.
- Changes to window detection, placement, stacking, focus, input regions,
  shortcuts, scaling, or geometry must consider every supported path and add
  the relevant automated coverage or real-machine checklist.
- Generic Wayland is not a supported backend. GNOME Shell/Mutter, Sway/wlroots,
  and other compositors remain unsupported until they have an explicit,
  reviewed bridge that provides OverCrow's full placement and input contract.
  Do not infer support from the ability to create a transparent window alone.
- Windowed and borderless-fullscreen games are in scope. Exclusive fullscreen
  can bypass compositor windows and remains unsupported.
- Unsupported or unrecognized display environments must fail closed without
  modifying compositor configuration or claiming partial compatibility.

## Non-negotiable product boundaries

- OverCrow stays external to the game process. Never add DLL/shared-object
  injection, Vulkan layers, `LD_PRELOAD`, graphics hooks, `ptrace`, memory
  reading, packet interception, input synthesis, or game-file modification.
- The application is disabled by default. Only explicitly selected games may
  authorize runtime services, shortcuts, input capture, or overlays.
- Passive overlays remain read-only and click-through. Interactive behavior
  requires an authorized active game and must retain a reliable close path.
- Fail closed when identity, compositor state, ownership, paths, provider data,
  or protocol values are missing, ambiguous, stale, or malformed.
- Bound external work: bytes, entries, strings, recursion, subprocess time,
  queues, retries, and refresh cadence. Keep HTTPS hosts allowlisted and reject
  redirects or credentials unless an approved design explicitly changes this.
- Preserve stable widget IDs, settings schemas, D-Bus names/signatures, desktop
  IDs, unit names, and persisted keys unless migration and compatibility are
  part of the request.
- Keep user configuration private and transactional. Do not weaken symlink,
  ownership, permission, canonical-path, atomic-write, or durability checks.
- Prefer semantic events and bounded/coalescing channels. Do not introduce
  steady polling, busy loops, unbounded channels, or repeated derived work when
  an existing event/deadline can drive the update.
- Every new external-data widget/provider must emit bounded, deduplicated
  `widget_provider_failed` and `widget_provider_recovered` events through the
  existing non-blocking logger. Use stable widget/provider IDs and categories
  only; never log raw errors, provider payloads, user content, paths, URLs,
  titles, queries, media metadata, notes, checklist text, or keystrokes.

## Efficient workflow

1. Read this file, inspect `git status`, then read only the code and current
   documentation relevant to the request.
2. For questions, audits, and diagnosis, stay read-only unless the user also
   asks for implementation.
3. Handle small, isolated changes directly. Do not create a design document,
   implementation plan, worktree, or subagent workflow unless complexity or
   risk actually justifies it or the user requests it.
4. For bugs, reproduce the symptom, identify the root cause, add the narrowest
   useful regression test, then implement the minimal fix.
5. Use a dedicated branch/worktree for substantial multi-file features,
   architectural changes, or work the user explicitly asks to isolate.
6. Run focused checks while iterating. Run expensive workspace-wide checks once
   when the implementation is stable, not after every edit.
7. Refactor only code touched by the task. Remove genuine duplication and dead
   code, but do not add abstractions without a concrete second use or measurable
   clarity benefit.
8. Keep the user informed during long work, but avoid approval loops for safe,
   reversible, in-scope decisions. Ask only when a choice materially changes
   behavior, scope, security, or external state.

## Real-machine and environment policy

- Do not use Docker, a VM, nested desktop, or a synthetic black-box environment
  unless it is necessary, explained, and approved by the user.
- Automated tests must not install packages, edit the user's live compositor
  configuration, reserve real shortcuts, or start/stop their active OverCrow
  session unless explicitly requested.
- Private D-Bus/socket tests may fail inside a restricted sandbox with
  `isolated bus omitted its address`. Re-run the unchanged test with normal
  local permissions before treating that as a code failure.
- Repository tests cannot prove live Hyprland, Plasma, X11, Proton, or game
  behavior. Perform bounded automated checks, then give the user a short exact
  real-machine checklist. Never claim live acceptance before they report it.

## Coding rules

- Rust edition 2024; keep `cargo fmt` and Clippy `-D warnings` clean.
- Prefer typed, small interfaces and existing module boundaries. Avoid broad
  public APIs, premature generic helpers, and hidden cross-component state.
- Avoid `unsafe`. If unavoidable, constrain it, document the invariant in
  English, and cover the boundary with focused tests.
- Production paths must not panic on external or runtime data. Validate before
  conversion; use checked or saturating arithmetic where overflow is possible.
- Never block the egui thread or async executors with unbounded I/O or waits.
  Worker and subprocess ownership must have explicit cancellation and bounded
  shutdown behavior.
- Comments are English and explain intent, invariants, or Linux-specific
  constraints—not syntax already obvious from the code.
- Tests should exercise real behavior, boundaries, malformed inputs, and
  fail-closed outcomes. Test-only `unwrap`/`expect` is acceptable when the
  message identifies the violated assumption.
- Shell must pass ShellCheck and `sh -n`, quote expansions, avoid `eval`, use
  fixed command shapes, bound external probes, and clean temporary resources.
- JavaScript in the KWin bridge must remain compatible with Plasma 6's QV4
  runtime; do not assume browser or Node-only APIs.

## Validation ladder

Use the smallest relevant checks during development, then the applicable final
gate. Do not substitute repeated compilation for reasoning or live validation.

For Rust changes:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets --locked
```

For shell, packaging, or integration changes, run the touched smoke tests plus:

```sh
shellcheck scripts/*.sh scripts/lib/*.sh tests/*.sh \
  packaging/arch/*.install packaging/arch/*.sh packaging/aur/*.install \
  packaging/release/*.sh
shellcheck -s bash packaging/aur/PKGBUILD
sh -n scripts/*.sh scripts/lib/*.sh tests/*.sh \
  packaging/arch/*.install packaging/arch/*.sh packaging/aur/*.install \
  packaging/release/*.sh
bash -n packaging/aur/PKGBUILD
```

For KWin changes:

```sh
node --check integrations/kwin/contents/code/main.js
node --test tests/kwin-bridge.test.js
tests/qjsvalue-variant-smoke.sh
```

When CI policy changes, keep `.github/workflows/ci.yml` and
`tests/ci-workflow-smoke.sh` in sync. Hosted CI intentionally runs only the
focused quality gate. Before a public release, also run the dependency policy,
a remapped release build, and every smoke test locally:

```sh
cargo deny --locked check advisories licenses
cargo deny --locked check bans sources
RUSTFLAGS="--remap-path-prefix=$PWD=/usr/src/overcrow" \
  cargo build --workspace --release --locked
for smoke_test in tests/*-smoke.sh; do "$smoke_test"; done
```

Run `./scripts/build-arch-package.sh` only when validating a distributable Arch
artifact, not for unrelated changes.

Always finish with:

```sh
git diff --check
git status --short --branch
```

## Git and delivery

- Preserve user changes and unrelated dirty files. Never reset, discard,
  rewrite, or include them in a commit.
- Keep commits small and intentional. Commit completed logical increments when
  the user expects ongoing commits; squash only when requested.
- Do not push, open a PR, merge to `master`, amend published history, or install
  artifacts without explicit authorization.
- Before integration, report the exact checks run, the commit(s), repository
  status, and any remaining real-machine validation.

## Definition of done

- The requested behavior is implemented with no unrelated expansion.
- Relevant failure, boundary, and compatibility paths are covered.
- Applicable validation commands pass with clean output.
- Security and anti-cheat boundaries remain intact.
- Documentation changes describe user-visible or operator-visible behavior
  without duplicating implementation detail.
- The worktree is clean, commits match the intended scope, and unverified live
  behavior is clearly handed off rather than claimed.

Lead final responses with the outcome. Be concise, name files and commands that
matter, and state blockers or unverified assumptions directly.
