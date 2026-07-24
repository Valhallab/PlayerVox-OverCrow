# Private Support Report Submission Plan

**Goal:** Let users review a bounded diagnostic report and explicitly send it
to PlayerVox support without a GitHub account or a local report file.

**Architecture:** `overcrow-control` prepares the exact in-memory Markdown and
owns a fixed, bounded HTTPS client. The Tauri host keeps only the latest
prepared report in memory, submits it off the UI thread, and returns the server
reference. React provides description, log consent, preview, copy fallback, and
the explicit send action.

## Constraints

- Never upload before the user clicks `Send report`.
- POST only to `https://api.playervox.com/api/v1/overcrow/reports`.
- Do not follow redirects or send credentials.
- Bound description, request, response, timeout, and concurrent submissions.
- Never log the report body, description, response body, or transport details.
- Keep no local support-report file.
- Treat malformed or unexpected server responses as failure.

## Implementation

1. Replace file persistence with a pure bounded report builder and random
   client report ID. Add request/receipt models and a hardened HTTP client.
2. Add focused tests for payload shape, no redirects, response bounds,
   malformed responses, conflict, rate limits, and timeout/error mapping.
3. Keep the latest prepared report in Tauri-managed memory. Submit only a
   matching report ID through `spawn_blocking`, with one request in flight.
4. Replace `Show file` and `Open GitHub issue` with `Send report`. Keep the
   exact preview and `Copy report`; show the returned support reference.
5. Update user documentation and smoke assertions, then run the relevant Rust,
   frontend, security, and repository checks.
