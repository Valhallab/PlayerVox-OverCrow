# Widget Resize Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove obsolete resize-repair code while preserving the behavior
validated on Hyprland.

**Architecture:** Keep event-batching compatibility inside the shared resize
grip and keep persistent geometry inside `WidgetManager`. Each widget only
provides its painted rectangle and consumes the shared outcome.

**Tech Stack:** Rust 2024, egui 0.35, existing widget profile store.

## Global Constraints

- Preserve the fixed visible top-left throughout a resize.
- Persist the final release delta, size, and normalized position immediately.
- Preserve width-only sizing for horizontal Performance and vertical Warframe
  Status.
- Keep passive mode, X11, Plasma Wayland, and Hyprland contracts unchanged.
- Add no dependency, persisted field, or display-specific branch.

---

### Task 1: Simplify resize state and persistence

**Files:**
- Modify: `crates/overcrow-overlay/src/widgets/manager/mod.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manager/layout.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manager/tests.rs`

**Interfaces:**
- Consumes: `ResizeGripOutcome` frame-local deltas.
- Produces: one `ResizeSession` containing only owner, anchor, size, and whether
  the size changed.

- [ ] Remove the post-release `released` state and restore immediate session
  teardown after profile persistence.
- [ ] Keep normalized placement based on the persisted effective panel size so
  a fresh manager restores the same top-left.
- [ ] Rename the disappearance regression to describe completed persistence,
  then verify focused resize tests pass.
- [ ] Commit the self-contained manager simplification.

### Task 2: Remove per-widget churn and redundant coverage

**Files:**
- Modify: `crates/overcrow-overlay/src/widgets/chrome.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manager/layout.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manager/tests.rs`
- Modify: `crates/overcrow-overlay/src/widgets/notes.rs`
- Modify: `crates/overcrow-overlay/src/widgets/performance.rs`
- Modify: `crates/overcrow-overlay/src/widgets/twitch_chat.rs`
- Modify: `crates/overcrow-overlay/src/widgets/warframe_fissures.rs`
- Modify: `crates/overcrow-overlay/src/widgets/warframe_invasions.rs`
- Modify: `crates/overcrow-overlay/src/widgets/warframe_market.rs`
- Modify: `crates/overcrow-overlay/src/widgets/warframe_sortie.rs`
- Modify: `crates/overcrow-overlay/src/widgets/warframe_status.rs`

**Interfaces:**
- Consumes: the panel outer rectangle after `Frame::show`.
- Produces: one shared grip rectangle and one batched-event-safe outcome.

- [ ] Share the exact grip rectangle with the manager and remove its duplicate
  grip-size constant.
- [ ] Move grip calls back to the uniform post-frame pattern, deleting the
  structural edits copied into every widget.
- [ ] Remove the redundant generic Notes manager pointer test; retain the exact
  Twitch application-order regression, grip-level tests, persistence tests,
  overlap test, and interruption tests.
- [ ] Run focused tests after each deletion and retain only code required by a
  failing regression.
- [ ] Run formatting, workspace Clippy, locked workspace tests, `git diff
  --check`, and a release overlay build.
- [ ] Commit the cleanup without pushing or merging.
