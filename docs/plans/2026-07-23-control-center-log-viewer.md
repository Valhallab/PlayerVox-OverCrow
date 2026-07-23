# Control Center Log Viewer

## Goal

Let users inspect OverCrow's existing diagnostic files from the native Control
Center without enabling automatic data transfer.

## Data boundary

The webview never receives filesystem access and never chooses a path. Its
typed Rust command reads only OverCrow's existing private log directory through
the hardened reader in `overcrow-logging`.

Each response is limited to the newest 500 lines and 256 KiB after JSON
serialization. Unsafe directories, symlinks, permissions, oversized files, or
malformed responses fail closed. The request accepts no user-controlled
arguments. Logs remain local and are never uploaded automatically.

The versioned response contains the retained sanitized lines and whether older
entries were omitted. Existing line sanitization and 1 KiB per-line bounds
remain authoritative.

## Interface

The Diagnostics page gains two local tabs:

- `Overview` retains the current diagnostic cards;
- `Logs` loads recent entries once when opened.

The Logs tab provides:

- a `Refresh` button with no timer or live tail;
- component, severity, and text filters applied locally;
- a bounded scrollable monospaced list with severity colors;
- `Copy visible logs`;
- explicit loading, empty, and friendly failure states.

Filters are populated from the components present in the response rather than
claiming support for components that do not currently emit file logs. React
renders every line as text, never as markup.

## Error handling

Log-loading errors do not replace or invalidate the Control Center snapshot.
The existing Overview diagnostics remain usable. A failed refresh preserves
the last successful result and shows a bounded error beside the refresh
control.

## Compatibility and testing

Tests cover response bounds, unsafe log files, load-on-open behavior, manual
refresh, filters, copying, and independent error handling.
