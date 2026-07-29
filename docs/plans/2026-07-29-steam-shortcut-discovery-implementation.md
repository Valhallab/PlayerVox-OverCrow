# Steam Shortcut Discovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Discover Proton games added to Steam, hide positively identified
non-game Steam applications, preserve ambiguous games with a warning, and
present the result alphabetically.

**Architecture:** A focused iterative binary-VDF reader extracts only shortcut
identity and official application type. Existing text manifest discovery joins
that metadata into typed `SteamGame` records before Control Center
serialization. Core continues to authorize the existing 32-bit Steam app ID.

**Tech Stack:** Rust 2024, bounded filesystem I/O, Steam KeyValues1 binary
records, serde snapshots, React/TypeScript, Vitest.

## Global Constraints

- No executable-path or Wine process heuristics.
- Keep `selected_steam_app_ids: BTreeSet<u32>` unchanged.
- Reject ambiguous identities and positively classified non-games.
- Preserve X11, Plasma Wayland, and Hyprland behavior.
- Bound files, profiles, records, nesting, strings, and warnings.
- Do not retain shortcut command lines, paths, tags, icons, or account IDs.

---

### Task 1: Bounded binary Steam metadata reader

**Files:**
- Create: `crates/overcrow-control/src/steam_binary.rs`
- Create: `crates/overcrow-control/src/steam_binary_tests.rs`
- Modify: `crates/overcrow-control/src/lib.rs`

**Interfaces:**
- Produces:
  `read_binary_metadata(root: &Path) -> BinaryMetadataReport`
- Produces:
  `BinaryMetadataReport { app_types: BTreeMap<u32, SteamAppType>,
  shortcuts: Vec<SteamShortcut>, warnings: Vec<String> }`
- Produces:
  `SteamShortcut { app_id: u32, name: String }`

- [ ] **Step 1: Write failing binary-reader tests**

Add small byte-building fixture helpers and tests proving:

```rust
#[test]
fn reads_shortcut_identity_without_retaining_launch_metadata() {
    let report = read_binary_metadata(&root);
    assert_eq!(
        report.shortcuts,
        [SteamShortcut {
            app_id: 2_369_324_441,
            name: "Soulframe".into(),
        }]
    );
}

#[test]
fn reads_game_and_tool_types_from_v41_appinfo() {
    assert_eq!(report.app_types[&10], SteamAppType::Game);
    assert_eq!(report.app_types[&20], SteamAppType::NonGame);
}
```

Also cover strict UTF-8, zero IDs, empty/oversized names, oversized files,
profile/entry/depth limits, truncated scalar values, invalid string-table
indices, unknown type bytes, symlink escape, and partial-profile recovery.

- [ ] **Step 2: Verify the tests fail for missing reader**

Run:

```sh
cargo test -p overcrow-control steam_binary_tests --locked
```

Expected: compilation fails because `steam_binary` and its interfaces do not
exist.

- [ ] **Step 3: Implement the minimal iterative reader**

Use a cursor over `&[u8]` with checked offset arithmetic. Maintain an explicit
object-path stack capped at `MAX_BINARY_VDF_DEPTH`; never recurse. Support
object start/end, string, signed 32-bit integer, float/pointer/color skip,
UTF-16 skip, and unsigned 64-bit skip. Reject unknown types and unterminated
values.

For app-info, validate v40/v41 headers and every entry size before slicing.
Bound and decode the v41 string table before walking records. Extract only
`common/type`. For shortcuts, scan at most `MAX_STEAM_PROFILES` canonical
numeric account directories and extract only `appid` and `appname`.

- [ ] **Step 4: Verify the focused reader suite passes**

Run:

```sh
cargo test -p overcrow-control steam_binary_tests --locked
```

Expected: all `steam_binary_tests` pass without warnings.

### Task 2: Join metadata into safe sorted game discovery

**Files:**
- Modify: `crates/overcrow-control/src/steam.rs`
- Modify: `crates/overcrow-control/src/steam_tests.rs`
- Modify: `crates/overcrow-control/src/lib.rs`
- Update fixtures constructing `SteamGame` in
  `crates/overcrow-control/src/*_tests.rs` and `crates/overcrow-control/src/commands.rs`

**Interfaces:**
- Produces:
  `SteamGameKind::{SteamGame, SteamShortcut, Unverified}`
- Changes:
  `SteamGame.install_dir` to `Option<PathBuf>` so shortcuts do not invent a
  filesystem identity.

- [ ] **Step 1: Write failing discovery tests**

Add tests proving that:

```rust
assert_eq!(
    names_and_kinds(&report.games),
    [
        ("alpha", SteamGameKind::SteamShortcut),
        ("Portal 2", SteamGameKind::SteamGame),
        ("Unknown", SteamGameKind::Unverified),
    ]
);
```

The same suite must prove known tools are absent, missing app-info preserves
manifests as `Unverified`, conflicting IDs are omitted, identical duplicates
coalesce, and ordering is Unicode-case-folded with app ID as the tie-breaker.

- [ ] **Step 2: Verify the new discovery tests fail**

Run:

```sh
cargo test -p overcrow-control steam_tests --locked
```

Expected: failures for missing kinds, shortcut joining, filtering, and
alphabetical ordering.

- [ ] **Step 3: Implement the metadata join**

Read binary metadata once per canonical root. Classify official manifests using
an allowlist:

```rust
match normalized_type {
    "game" | "demo" => SteamGameKind::SteamGame,
    "advertising" | "application" | "config" | "dlc" | "guide"
    | "hardware" | "music" | "series" | "tool" | "video" => omit,
    _ => SteamGameKind::Unverified,
}
```

Merge shortcuts by app ID without retaining their executable. Reject a source
collision rather than authorizing an ambiguous ID. Sort with
`unicode_casefold::UnicodeCaseFold`, then app ID. Keep warning count and byte
bounds through the existing warning helper.

- [ ] **Step 4: Verify all Control Center Rust tests**

Run:

```sh
cargo test -p overcrow-control --locked
```

Expected: all tests pass.

### Task 3: Typed Control Center presentation and picker guidance

**Files:**
- Modify: `crates/overcrow-control/src/presentation.rs`
- Modify: `crates/overcrow-control/src/presentation_tests.rs`
- Modify: `crates/overcrow-control/src/app.rs`
- Modify: `crates/overcrow-control/src/model.rs`
- Modify: `crates/overcrow-control/src/model_tests.rs`
- Modify: `crates/overcrow-control-ui/src/lib/control.ts`
- Modify: `crates/overcrow-control-ui/src/test/fixtures.ts`
- Modify: `crates/overcrow-control-ui/src/components/Dashboard.tsx`
- Modify: `crates/overcrow-control-ui/src/i18n/en.ts`
- Modify: `crates/overcrow-control-ui/src/App.test.tsx`

**Interfaces:**
- Adds `kind: "steam_game" | "steam_shortcut" | "unverified"` to each
  serialized game.
- Increments `CONTROL_SNAPSHOT_SCHEMA_VERSION` from `2` to `3`.

- [ ] **Step 1: Write failing Rust and UI tests**

Rust tests assert schema `3`, kind serialization, and this `.exe` guidance:

```text
Windows executables cannot be selected directly. Add the game to Steam,
force a Proton compatibility tool, launch it once, then refresh this list.
```

Vitest renders `Steam shortcut` and `Type unverified`, while ordinary rows
remain `Steam · App <id>`.

- [ ] **Step 2: Verify both focused suites fail**

Run:

```sh
cargo test -p overcrow-control presentation_tests --locked
cargo test -p overcrow-control model_tests --locked
npm test -- --run src/App.test.tsx
```

Run the npm command from `crates/overcrow-control-ui`. Expected: schema,
property, label, and guidance assertions fail.

- [ ] **Step 3: Implement the schema and UI copy**

Add a serde snake-case presentation enum, include it in wire-bound checks and
snapshot mapping, update the TypeScript union and fixtures, and render the
appropriate secondary label. Keep the row interaction and persisted IDs
unchanged.

- [ ] **Step 4: Verify focused Rust and UI suites pass**

Run:

```sh
cargo test -p overcrow-control --locked
npm test
npm run build
```

Run npm commands from `crates/overcrow-control-ui`. Expected: all commands exit
zero.

### Task 4: Documentation and final validation

**Files:**
- Modify: `README.md`
- Modify: `docs/troubleshooting.md`

**Interfaces:**
- Documents the supported non-Steam shortcut path and native-only manual
  executable picker.

- [ ] **Step 1: Update user documentation**

Explain that Windows games must be added as Steam shortcuts, forced through
Proton, launched once, and selected after Refresh. Explain `Type unverified`
without implying incompatibility.

- [ ] **Step 2: Run the full relevant gate**

Run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets --locked
```

Then from `crates/overcrow-control-ui`:

```sh
npm test
npm run build
```

Expected: every command exits zero.

- [ ] **Step 3: Run final repository checks**

Run:

```sh
git diff --check
git status --short --branch
```

Expected: only intentional feature files are modified on
`feature/steam-shortcuts-discovery`.

- [ ] **Step 4: Commit the implementation**

Create one intentional implementation commit after all gates pass:

```sh
git add README.md docs/troubleshooting.md crates/overcrow-control \
  crates/overcrow-control-ui
git commit -m "feat(control): discover Steam shortcuts safely"
```
