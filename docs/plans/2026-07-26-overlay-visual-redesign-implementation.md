# Overlay Visual Redesign Implementation Plan

> Scope: presentation only. Preserve all runtime, provider, persistence,
> placement, and input contracts.

## 1. Establish the presentation system

**Files**

- Modify: `crates/overcrow-overlay/src/branding.rs`
- Modify: `crates/overcrow-overlay/src/widgets/chrome.rs`
- Test: `crates/overcrow-overlay/src/widgets/presentation_tests.rs`

Add PlayerVox color and spacing tokens, install the existing bundled Noto Sans
faces as the overlay body fonts, define branded egui visuals, and provide small
shared paint helpers for headers, pills, metric cards, section labels, buttons,
and panel frames. Write token/invariant tests first.

## 2. Add categorized widget metadata

**Files**

- Modify: `crates/overcrow-overlay/src/widgets/registry.rs`
- Modify: `crates/overcrow-overlay/src/widgets/catalog.rs`
- Modify: `crates/overcrow-overlay/src/widgets/mod.rs`
- Test: `crates/overcrow-overlay/src/app_tests.rs`
- Test: `crates/overcrow-overlay/src/widgets/manager/tests.rs`

Add stable presentation-only category and visual-mark metadata to every
descriptor. Test complete, unique grouping and stable order. Replace the list
with responsive category sections and widget cards while preserving exact
`CatalogAction` behavior.

## 3. Redesign the overlay shell

**Files**

- Modify: `crates/overcrow-overlay/src/app.rs`
- Test: `crates/overcrow-overlay/src/app_tests.rs`

Apply the overlay theme once at startup. Restyle the existing bottom bar,
catalog container, and about surface without altering visibility, toggle, or
close behavior.

## 4. Redesign general widgets

**Files**

- Modify: `crates/overcrow-overlay/src/widgets/session.rs`
- Modify: `crates/overcrow-overlay/src/widgets/clock.rs`
- Modify: `crates/overcrow-overlay/src/widgets/performance.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manual_stopwatch.rs`
- Modify: `crates/overcrow-overlay/src/widgets/media.rs`
- Modify: `crates/overcrow-overlay/src/widgets/notes.rs`
- Modify: `crates/overcrow-overlay/src/widgets/twitch_chat.rs`

Use the shared header and card hierarchy, improving scanning and controls while
retaining all presentation models, action routing, bounds, passive behavior,
and auto-height reporting.

## 5. Redesign Warframe widgets

**Files**

- Modify: `crates/overcrow-overlay/src/widgets/warframe_status.rs`
- Modify: `crates/overcrow-overlay/src/widgets/warframe_fissures.rs`
- Modify: `crates/overcrow-overlay/src/widgets/warframe_market.rs`
- Modify: `crates/overcrow-overlay/src/widgets/warframe_sortie.rs`
- Modify: `crates/overcrow-overlay/src/widgets/warframe_invasions.rs`

Apply consistent headers, filter chips, section cards, rows, progress visuals,
and status accents without changing derived data, filters, actions, scrolling,
or resize calculations.

## 6. Verify and clean

Run focused tests during each step, then:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets --locked
git diff --check
git status --short --branch
```

Review the diff for dead code, duplicated style literals, accidental behavior
changes, per-frame cloning, and unnecessary abstractions. Commit logical,
reviewable increments on the dedicated branch. Do not merge or push.

## 7. Persist common-widget display options

**Files**

- Modify: `crates/overcrow-config/src/widgets.rs`
- Modify: `crates/overcrow-config/src/widget_store_tests.rs`

Add defaulted clock-date and performance-orientation settings without changing
stable widget IDs or invalidating existing version-3 profiles. Test legacy
deserialization, defaults, validation, and round trips before adding the fields.

## 8. Unify compact controls

**Files**

- Modify: `crates/overcrow-overlay/src/widgets/chrome.rs`
- Modify: `crates/overcrow-overlay/src/widgets/presentation_tests.rs`

Define one compact control height and shared badge/icon-button helpers. Add
layout tests for natural badge height, square icon controls, and consistent
single-line control sizing before migrating widget callers.

## 9. Refine common widget bodies

**Files**

- Modify: `crates/overcrow-overlay/src/widgets/session.rs`
- Modify: `crates/overcrow-overlay/src/widgets/clock.rs`
- Modify: `crates/overcrow-overlay/src/widgets/performance.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manual_stopwatch.rs`
- Modify: `crates/overcrow-overlay/src/widgets/media.rs`
- Modify: `crates/overcrow-overlay/src/widgets/notes.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manager/builtin.rs`
- Test: colocated unit tests and `crates/overcrow-overlay/src/widgets/manager/tests.rs`

First add failing presentation/layout tests. Then remove redundant common-widget
headers, normalize game CPU against bounded logical capacity, implement full-row
and full-column performance layouts with a bounded narrow-viewport fallback,
use conventional media symbols, place the media state beside the artist, and
make the note add action part of the tab strip. Preserve passive auto-height,
dragging, resizing, and action routing.

## 10. Move widget options into the library

**Files**

- Modify: `crates/overcrow-overlay/src/widgets/catalog.rs`
- Modify: `crates/overcrow-overlay/src/widgets/notes.rs`
- Modify: `crates/overcrow-overlay/src/widgets/twitch_chat.rs`
- Modify: `crates/overcrow-overlay/src/widgets/warframe_status.rs`
- Modify: `crates/overcrow-overlay/src/widgets/warframe_fissures.rs`
- Modify: `crates/overcrow-overlay/src/widgets/warframe_sortie.rs`
- Modify: `crates/overcrow-overlay/src/widgets/warframe_invasions.rs`
- Modify: `crates/overcrow-overlay/src/warframe/controller.rs`
- Modify: `crates/overcrow-overlay/src/app.rs`
- Test: `crates/overcrow-overlay/src/app_tests.rs`
- Test: `crates/overcrow-overlay/src/warframe/controller_tests.rs`

Add a shared `More options` menu to cards and a small typed library-action
router. Reuse widget-owned option painters, but render them from the catalog.
Route general, Twitch, and Warframe actions through their existing validation,
authorization, transactional save, and logging paths. Remove every in-widget
settings gear. Test option presence, typed routing, interaction gating, and
rollback behavior before implementation.

## 11. Final acceptance for the feedback pass

Run focused tests after each red-green step, then the full Rust gate:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets --locked
git diff --check
git status --short --branch
```

Hand off a short live test covering both performance orientations, date
visibility, all catalog option menus, passive auto-height, and media/notes
interaction. Do not claim compositor/game acceptance before the user reports
it.
