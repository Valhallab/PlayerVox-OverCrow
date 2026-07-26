# Widget Resize Cleanup Design

## Goal

Reduce the resize implementation left by the earlier repair attempts without
changing the behavior confirmed on Hyprland.

## Preserved behavior

- A bottom-right grip resizes every supported resizable widget.
- Press and initial motion may arrive in one egui frame.
- The widget's visible top-left remains fixed during the gesture.
- The final pointer movement is applied on release.
- Width-only widgets never persist a synthetic height.
- Size and normalized position are saved on release.
- Interrupted or stale gestures fail closed.

## Approach

Keep one shared egui compatibility boundary in `chrome.rs` and one resize
session in `WidgetManager`. Remove transient state, per-widget structural
changes, guards, comments, and tests only when the existing end-to-end Twitch
and real-pointer regressions prove them redundant.

The cleanup must not introduce a new abstraction, dependency, persisted field,
or display-backend-specific path.

## Validation

- Focused real-pointer, batched-event, release, interruption, overlap, and
  restart-placement tests.
- Full formatting, Clippy, and workspace tests.
- Final manual Hyprland test by the user because repository tests cannot prove
  compositor behavior.
