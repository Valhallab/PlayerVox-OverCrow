# Control Center Lifecycle Action

## Goal

Make OverCrow's two authorization levels immediately understandable:

- game selection defines where OverCrow may run;
- the lifecycle action defines whether OverCrow is currently running.

## Interface

Remove the global toggle from the dashboard header. The overview status card
becomes the single place that displays and changes the lifecycle state.

The card exposes one explicit action:

- `Start OverCrow` while stopped;
- `Stop OverCrow` while running.

Stopping OverCrow disables its runtime services, overlay, and shortcut while
preserving allowed games and all user settings. Starting it uses the existing
allowlist and never authorizes an unchecked game.

The card displays `Running`, `Stopped`, `Starting…`, or `Needs attention` with
short explanatory copy. Its action is unavailable while another lifecycle
operation is running or when the installed integration cannot safely start.

## Scope

This is a presentation change over the existing `setEnabled` lifecycle
operation. It does not change persistence, installation, D-Bus contracts,
game authorization, or the fail-closed security model.

## Verification

Component tests cover the absence of the header toggle, the correct Start/Stop
action for each lifecycle state, preserved game selections, and disabled
actions during transitions or unsupported states.
