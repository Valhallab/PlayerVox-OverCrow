# Notes Workspace and Passive Sizing Implementation Plan

**Goal:** Turn the Notes widget into a bounded multi-note workspace and make
resizable widgets use their measured content height in passive mode.

**Architecture:** Notes remain in one private, atomically replaced local
document. Schema v2 stores up to eight titled pages with stable IDs and
per-page checklists; the repository migrates schema v1 in memory without
rewriting it until the next user mutation. Display preferences stay in the
widget profile, while the renderer keeps separate interactive and passive
measurements so content-fit rendering never changes saved geometry.

**Tech Stack:** Rust 2024, serde/serde_json, egui/eframe, existing bounded notes
worker and transactional configuration stores.

## Global Constraints

- Preserve existing v1 note and checklist content during migration.
- Keep notes and widget configuration private, bounded, validated, and atomic.
- Render plain text only and never log note titles, bodies, or checklist text.
- Passive mode remains click-through and cannot produce mutating commands.
- Interactive configured height remains user-controlled and persisted.
- Passive measured height is transient and capped to the game viewport.
- No change to X11, Plasma Wayland, or Hyprland compositor contracts.

---

### Task 1: Versioned Multi-Note Domain

**Files:**
- Modify: `crates/overcrow-overlay/src/notes/model.rs`
- Modify: `crates/overcrow-overlay/src/notes/store.rs`
- Modify: `crates/overcrow-overlay/src/notes/mod.rs`
- Test: `crates/overcrow-overlay/src/notes/tests.rs`

**Interfaces:**
- Produce `NotePage { id, title, body, items }`.
- Produce schema-v2 `NotesDocument { next_note_id, next_item_id,
  active_note_id, notes }`.
- Produce page-scoped atomic mutation methods and a bounded v1-to-v2 loader.

- [x] Write tests proving default identity, page/item limits, stable monotonic
  IDs, last-page protection, active-page fallback, UTF-8 limits, unknown-field
  rejection, and exact v1 content migration.
- [x] Run `cargo test -p overcrow-overlay notes::tests --locked` and confirm the
  new tests fail because schema v2 and migration do not exist.
- [x] Implement the schema-v2 model with eight pages maximum, 96-byte titles,
  8-KiB bodies, 64 checklist entries per page, stable global item IDs, and
  checked arithmetic.
- [x] Make the repository inspect the bounded `schema_version`, deserialize
  only v1 or v2, migrate v1 to a `General` page, and reject unknown versions.
- [x] Re-run the focused notes tests and confirm they pass.

### Task 2: Multi-Note UX and Independent Visibility

**Files:**
- Modify: `crates/overcrow-config/src/widgets.rs`
- Modify: `crates/overcrow-config/src/widget_store.rs`
- Modify: `crates/overcrow-config/src/lib.rs`
- Modify: `crates/overcrow-config/src/widget_store_tests.rs`
- Modify: `crates/overcrow-overlay/src/notes/service.rs`
- Modify: `crates/overcrow-overlay/src/widgets/notes.rs`
- Modify: `crates/overcrow-overlay/src/widgets/notes_tests.rs`
- Modify: `crates/overcrow-overlay/src/widgets/catalog.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manager/mod.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manager/builtin.rs`
- Modify: `crates/overcrow-overlay/src/app.rs`
- Test: `crates/overcrow-overlay/src/app_tests.rs`

**Interfaces:**
- Produce page-scoped `NotesCommand` variants.
- Produce `NotesDisplaySettings { show_note, show_checklist }`, defaulting both
  to true and rejecting a configuration that hides both.
- Produce horizontal page selection, bounded title/body drafts, safe
  add/delete flows, and checked rows sorted after open rows.

- [x] Write failing tests for schema-v1 widget-profile migration, visibility
  validation, page-scoped optimistic updates, dirty-draft preservation,
  passive mutation rejection, checked-row ordering, and content visibility.
- [x] Run the focused config and overlay tests and verify they fail for the
  missing commands/settings.
- [x] Add backward-compatible in-memory widget-profile migration and catalog
  actions for `Show note` and `Show checklist`.
- [x] Replace the single-note UI with horizontally scrollable titled tabs,
  explicit save state, add/delete controls, and independent content sections.
- [x] Draw the checklist control with egui primitives, strike completed text,
  and render completed entries after active entries while preserving order.
- [x] Use neutral copy: `Add something…`, `Nothing here yet`, `Note title`, and
  `Write anything…`.
- [x] Re-run focused tests and confirm they pass.

### Task 3: Mode-Aware Content-Fit Sizing

**Files:**
- Modify: `crates/overcrow-overlay/src/widgets/chrome.rs`
- Modify: `crates/overcrow-overlay/src/widgets/notes.rs`
- Modify: `crates/overcrow-overlay/src/widgets/warframe_fissures.rs`
- Modify: `crates/overcrow-overlay/src/widgets/warframe_market.rs`
- Modify: `crates/overcrow-overlay/src/widgets/warframe_sortie.rs`
- Modify: `crates/overcrow-overlay/src/widgets/warframe_invasions.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manager/mod.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manager/layout.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manager/builtin.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manager/warframe.rs`
- Test: `crates/overcrow-overlay/src/widgets/manager/tests.rs`
- Test: `crates/overcrow-overlay/src/widgets/presentation_tests.rs`

**Interfaces:**
- Keep distinct transient measurements for `OverlayMode::Interactive` and
  `OverlayMode::Passive`.
- Produce shared panel constraints that fix configured size while editing and
  use measured content height, capped to the safe viewport, while passive.

- [x] Write failing tests proving passive measurement cannot replace the
  interactive cache or persisted height, and passive panels report content
  height below the interactive minimum.
- [x] Run focused manager/presentation tests and verify the new assertions fail.
- [x] Add mode-aware measurement and placement APIs, updating every caller.
- [x] Apply content-fit passive constraints to Notes, Fissures, Market, Sortie,
  and Invasions; leave already auto-sized widgets unchanged.
- [x] Request one follow-up repaint when a mode-specific measured size changes
  so normalized bottom/right placement settles without polling.
- [x] Re-run focused tests and confirm they pass.

### Task 4: Documentation and Final Gate

**Files:**
- Modify: `README.md`
- Modify: `docs/architecture.md`

- [x] Document titled notes, independent note/checklist visibility, local
  persistence, v1 migration, and passive content-fit behavior.
- [x] Run `cargo fmt --all -- --check`.
- [x] Run `cargo clippy --workspace --all-targets -- -D warnings`.
- [x] Run `cargo test --workspace --all-targets --locked`.
- [x] Run `git diff --check` and `git status --short --branch`.
- [x] Review the diff for note-content logging, unbounded allocations,
  accidental schema breakage, unrelated edits, and display-backend changes.
