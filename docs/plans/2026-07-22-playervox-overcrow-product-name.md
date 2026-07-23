# PlayerVox OverCrow Product Name Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every user-facing application identity read `PlayerVox OverCrow` without changing compatibility identifiers.

**Architecture:** Keep technical IDs stable and update only presentation constants, desktop metadata, package copy, documentation, and their focused tests. Preserve the intentionally legacy desktop fixture used by migration tests.

**Tech Stack:** Rust, React/TypeScript, Tauri configuration, freedesktop desktop/AppStream metadata, POSIX shell tests.

## Global Constraints

- The exact user-facing product name is `PlayerVox OverCrow`.
- Do not render a separate `by PlayerVox` byline.
- Keep `com.playervox.OverCrow`, `io.github.overcrow.*`, package names, binary names, service names, configuration paths, and persisted keys unchanged.
- Do not modify `tests/fixtures/legacy-overlay.desktop`; it deliberately represents an old installation.
- Do not push.

---

### Task 1: Replace and enforce the visible product name

**Files:**
- Modify: `tests/public-license-policy-smoke.sh`
- Modify: `crates/overcrow-control-ui/src/App.test.tsx`
- Modify: `crates/overcrow-control/src/app_tests.rs`
- Modify: `crates/overcrow-control-ui/src/i18n/en.ts`
- Modify: `crates/overcrow-control-ui/src/components/Brand.tsx`
- Modify: `crates/overcrow-control-ui/index.html`
- Modify: `crates/overcrow-control-ui/src-tauri/tauri.conf.json`
- Modify: `crates/overcrow-control/src/app.rs`
- Modify: `packaging/applications/com.playervox.OverCrow.desktop`
- Modify: `packaging/metainfo/com.playervox.OverCrow.metainfo.xml`
- Modify: `packaging/arch/overcrow.install`
- Modify: `README.md`
- Modify: `docs/testing/manual-mvp.md`

**Interfaces:**
- Consumes: the stable IDs listed under Global Constraints.
- Produces: the exact visible title `PlayerVox OverCrow` on every application identity surface.

- [ ] **Step 1: Write failing identity tests**

Change the Rust assertion to:

```rust
assert_eq!(APPLICATION_TITLE, "PlayerVox OverCrow");
```

Add a UI assertion after the first screen appears:

```typescript
expect(screen.getByText('PlayerVox OverCrow')).toBeVisible();
expect(screen.queryByText('by PlayerVox')).not.toBeInTheDocument();
```

Make the shell policy require the new launcher and reject the legacy title on
all production presentation files:

```sh
grep -Fq 'Name=PlayerVox OverCrow' \
    packaging/applications/com.playervox.OverCrow.desktop

if grep -Fq 'OverCrow by PlayerVox' \
        README.md \
        docs/testing/manual-mvp.md \
        packaging/arch/overcrow.install \
        packaging/applications/com.playervox.OverCrow.desktop \
        packaging/metainfo/com.playervox.OverCrow.metainfo.xml \
        crates/overcrow-control-ui/index.html \
        crates/overcrow-control-ui/src-tauri/tauri.conf.json; then
    printf '%s\n' 'legacy visible product name remains' >&2
    exit 1
fi
```

- [ ] **Step 2: Run the tests and verify they fail**

Run:

```sh
cargo test -p overcrow-control shell_metadata_has_the_exact_application_identity_and_title
npm --prefix crates/overcrow-control-ui test -- --run
tests/public-license-policy-smoke.sh
```

Expected: each relevant test fails because production copy still uses the old title.

- [ ] **Step 3: Apply the minimal presentation changes**

Use one brand value and remove the separate byline:

```typescript
brand: {
  product: 'PlayerVox OverCrow',
},
```

```tsx
<strong>{en.brand.product}</strong>
```

Set Rust and configuration titles exactly:

```rust
pub const APPLICATION_TITLE: &str = "PlayerVox OverCrow";
```

```json
"title": "PlayerVox OverCrow"
```

Set the HTML title, desktop `Name`, AppStream `name`, install message, README,
and manual launcher instruction to `PlayerVox OverCrow`. Do not change prose
that simply refers to the project as OverCrow or explains the PlayerVox brand.

- [ ] **Step 4: Run focused and policy validation**

Run:

```sh
cargo fmt --all -- --check
cargo test -p overcrow-control shell_metadata_has_the_exact_application_identity_and_title
npm --prefix crates/overcrow-control-ui test -- --run
npm --prefix crates/overcrow-control-ui run build
tests/public-license-policy-smoke.sh
appstreamcli validate --strict --no-net packaging/metainfo/com.playervox.OverCrow.metainfo.xml
desktop-file-validate packaging/applications/com.playervox.OverCrow.desktop
shellcheck packaging/arch/*.install tests/public-license-policy-smoke.sh
git diff --check
```

Expected: all commands pass; AppStream may retain its known pedantic uppercase-ID note.

- [ ] **Step 5: Commit the implementation**

```sh
git add README.md docs/testing/manual-mvp.md \
  crates/overcrow-control crates/overcrow-control-ui \
  packaging/applications packaging/arch packaging/metainfo \
  tests/public-license-policy-smoke.sh
git commit -m "fix(branding): use the PlayerVox OverCrow product name"
```
