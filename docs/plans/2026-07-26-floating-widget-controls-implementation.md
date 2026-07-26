# Floating Widget Controls Implementation Plan

> **For Codex:** Use the executing-plans workflow and implement each task test-first.

**Goal:** Put compact controls and options back on each rendered widget, simplify
the widget library, preserve widget top-left placement across automatic size
changes, and complete the requested media, performance, notes, Twitch, and
per-widget scaling behavior.

**Architecture:** The widget manager records the rectangles rendered during the
current interactive frame and paints one shared foreground toolbar for them.
Common settings remain transactional through the existing profile action path;
provider-specific menus keep their existing action types. Runtime absolute
anchors stabilize placement without rewriting persisted normalized positions
unless the user actually drags or resizes a widget.

**Tech Stack:** Rust 2024, eframe/egui 0.35, existing `overcrow-config`
transactional persistence, existing provider action routers.

---

## Task 1: Extend transactional widget settings actions

**Files:**
- Modify: `crates/overcrow-overlay/src/widgets/catalog.rs`
- Modify: `crates/overcrow-overlay/src/app_tests.rs`

1. Add failing tests for applying a bounded content scale and for resetting a
   widget position without changing unrelated settings.
2. Add `SetScale(WidgetId, f32)` to the existing transactional action enum.
3. Reuse profile validation for the 0.75–1.75 bound and retain rollback,
   post-commit warning, logging, and stopwatch reload behavior.
4. Run the focused catalog/action tests.

## Task 2: Stabilize the runtime top-left anchor

**Files:**
- Modify: `crates/overcrow-overlay/src/widgets/manager/mod.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manager/layout.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manager/tests.rs`

1. Replace the old test that expected bottom-anchored mode changes to move the
   widget with tests proving that measured size and passive/interactive changes
   preserve the last visible top-left.
2. Add one runtime anchor per widget, update it after paint/drag/resize, clear it
   when no game is active, and clamp it only when the current widget would leave
   the safe viewport.
3. Keep normalized persistence limited to completed user drag/resize gestures.
4. Run manager tests.

## Task 3: Add the shared foreground widget toolbar

**Files:**
- Modify: `crates/overcrow-overlay/src/widgets/chrome.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manager/mod.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manager/layout.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manager/tests.rs`
- Modify: `crates/overcrow-overlay/src/widgets/mod.rs`

1. Add tests for toolbar placement above a widget, safe top-edge folding, hover
   visibility, and custom icon hit targets.
2. Record visible widget rectangles for the current frame.
3. Paint a foreground toolbar on hover with the exact order: passive eye,
   options ellipsis, disable cross.
4. Put transparent background, content scale, reset content size, and reset
   position in the common options menu; expose a callback for widget-specific
   menu contents.
5. Ensure the toolbar is absent in passive mode and is never included in widget
   size measurements.
6. Run focused chrome and manager tests.

## Task 4: Make the widget library compact and click-to-toggle

**Files:**
- Modify: `crates/overcrow-overlay/src/widgets/catalog.rs`
- Modify: `crates/overcrow-overlay/src/app_tests.rs`
- Modify: `crates/overcrow-overlay/src/app.rs`

1. Add UI tests proving a card has one enable/disable action and no passive,
   transparent, or more-options controls.
2. Replace the current 126 px card with a compact clickable card containing the
   glyph, name, short description, and non-interactive ON/OFF state.
3. Remove the catalog options callback; clicking anywhere on a card toggles the
   widget.
4. Keep categories, responsive columns, and enabled count.
5. Run catalog UI tests.

## Task 5: Wire common and widget-specific controls in the app

**Files:**
- Modify: `crates/overcrow-overlay/src/app.rs`
- Modify: `crates/overcrow-overlay/src/app_tests.rs`
- Modify: `crates/overcrow-overlay/src/widgets/catalog.rs`
- Modify: `crates/overcrow-overlay/src/widgets/mod.rs`

1. Add focused routing tests for common toolbar actions.
2. Begin and finish the manager frame around widget rendering.
3. Route eye, transparency, scale, reset, and disable through transactional
   profile persistence.
4. Render Clock, Performance, Notes, Twitch, and Warframe-specific options from
   the relevant widget toolbar menu, preserving provider availability gates.
5. Show bounded settings failures/warnings in the bottom overlay controls even
   when the library is closed.
6. Run app tests.

## Task 6: Apply content scale and requested widget refinements

**Files:**
- Modify: `crates/overcrow-overlay/src/widgets/session.rs`
- Modify: `crates/overcrow-overlay/src/widgets/clock.rs`
- Modify: `crates/overcrow-overlay/src/widgets/performance.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manual_stopwatch.rs`
- Modify: `crates/overcrow-overlay/src/widgets/media.rs`
- Modify: `crates/overcrow-overlay/src/widgets/notes.rs`
- Modify: `crates/overcrow-overlay/src/widgets/manager/builtin.rs`
- Modify: relevant widget test modules

1. Add failing tests for bounded media width, horizontal performance width
   layout, compact checklist removal, and scaled compact widget content.
2. Pass persisted scale into every compact widget and apply it only to content.
3. Make media automatically size between 240 and 560 logical px and wrap at the
   maximum.
4. Add horizontal-only performance resizing from 300 to 900 logical px; use one
   row when it fits and a 2x2 fallback below the content threshold.
5. Make checklist removal a 22 px shared compact icon button.
6. Ensure reset content size restores each widget's effective defaults.
7. Run all overlay widget tests.

## Task 7: Relocate Twitch favorite mutation

**Files:**
- Modify: `crates/overcrow-overlay/src/widgets/twitch_chat.rs`
- Modify: `crates/overcrow-overlay/src/widgets/twitch_chat_tests.rs`
- Modify: `crates/overcrow-overlay/src/app.rs`

1. Add a failing test proving the channel header star is non-interactive while
   the toolbar options can add/remove the current channel from favorites.
2. Remove the favorite mutation control from the widget body.
3. Keep a static star next to the channel name when it is a favorite.
4. Preserve credential safety, action bounds, and existing provider routing.
5. Run Twitch widget/provider tests.

## Task 8: Cleanup and verification

**Files:**
- Modify only touched files and documentation if user-visible behavior needs it.

1. Remove obsolete catalog-only option helpers and genuinely dead imports/code.
2. Run:

   ```sh
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace --all-targets --locked
   git diff --check
   git status --short --branch
   ```

3. Review the final diff for input safety, compositor neutrality, persistence
   rollback, redraw behavior, and unnecessary abstractions.
4. Commit logical implementation increments. Do not push or merge.
5. Hand off a concise real-machine checklist covering hover controls, passive
   click-through, mode transitions, resize anchoring, provider menus, and
   X11/Plasma/Hyprland behavior that automated tests cannot prove.
