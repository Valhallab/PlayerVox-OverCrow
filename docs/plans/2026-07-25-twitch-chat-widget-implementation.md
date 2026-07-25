# Twitch EventSub and Lifecycle Hardening Implementation Plan

> **For agentic workers:** Implement each task with test-first red/green cycles.

**Goal:** Make the current Twitch branch safe to test by fixing display
authority regressions and replacing IRC with Twitch's supported EventSub and
Helix chat path.

**Architecture:** Keep the existing renderer-owned Twitch worker, bounded
commands, coalesced snapshots, OAuth machine, and widget UI. Replace only the
provider protocol boundary and remove dead IRC/asset code. Make display
authority fail-safe independently of provider state.

**Tech stack:** Rust 2024, Tokio, tokio-tungstenite, ureq, serde, egui, D-Bus,
Hyprland IPC, systemd user units.

## Global constraints

- Preserve X11 EWMH, Plasma 6 KWin, and Hyprland 0.55+ paths.
- Keep OverCrow external to game processes.
- Never log Twitch private content or credentials.
- Bound every queue, payload, deadline, retry, and deduplication collection.
- Do not push or install artifacts without user authorization.

### Task 1: Hyprland passive-focus continuity

**Files:**
- Modify: `crates/overcrow-hyprland/src/model.rs`
- Modify: `crates/overcrow-hyprland/src/reconcile.rs`
- Modify: `crates/overcrow-hyprland/src/bridge.rs`

- [x] Add failing tests proving an empty focus retains a mapped remembered game
      only on the focused monitor's active workspace.
- [x] Add failing tests proving workspace changes and invalid active windows
      clear the remembered game.
- [x] Extend the bounded monitor model with focused active-workspace identity.
- [x] Implement explicit active-window classification and workspace validation.
- [x] Run `cargo test -p overcrow-hyprland --all-targets --locked`.

### Task 2: Renderer fail-safe authority and restart

**Files:**
- Modify: `crates/overcrow-overlay/src/app.rs`
- Modify: `crates/overcrow-overlay/src/app_tests.rs`
- Modify: `crates/overcrow-overlay/src/main.rs`
- Modify: `packaging/systemd/overcrow-overlay.service.in`
- Modify: relevant systemd smoke test

- [x] Add failing tests proving authority loss forces effective Passive and
      mouse passthrough even when the retained snapshot is Interactive.
- [x] Implement the effective presentation/input mode without mutating Core
      state.
- [x] Add a failing test for unexpected successful event-loop termination.
- [x] Make unexpected termination return failure and retain bounded systemd
      restart behavior.
- [x] Run focused overlay and packaging tests.

### Task 3: OAuth scope and Secret Service recovery

**Files:**
- Modify: `crates/overcrow-overlay/src/twitch/auth.rs`
- Modify: `crates/overcrow-overlay/src/twitch/auth_tests.rs`
- Modify: `crates/overcrow-overlay/src/twitch/http.rs`

- [x] Add failing tests for the new exact scopes.
- [x] Add a failing test for transient credential-store restore failure.
- [x] Store authenticated user ID alongside login.
- [x] Schedule bounded restore retries and permit recovery without restart.
- [x] Run focused authentication tests.

### Task 4: EventSub protocol and Helix HTTP boundary

**Files:**
- Create: `crates/overcrow-overlay/src/twitch/eventsub.rs`
- Create: `crates/overcrow-overlay/src/twitch/eventsub_tests.rs`
- Modify: `crates/overcrow-overlay/src/twitch/http.rs`
- Modify: `crates/overcrow-overlay/src/twitch/mod.rs`
- Delete: `crates/overcrow-overlay/src/twitch/irc.rs`
- Delete: `crates/overcrow-overlay/src/twitch/tests.rs`
- Delete: `crates/overcrow-overlay/src/twitch/rate_limit.rs`

- [x] Add failing parser tests for welcome, keepalive, reconnect, revocation,
      chat message, clear, delete, duplicate IDs, malformed and oversized data.
- [x] Implement strictly bounded EventSub envelopes and payload models.
- [x] Add failing HTTP tests for channel resolution, subscription creation, and
      accepted/rejected Send Chat Message responses.
- [x] Implement fixed-shape Helix calls with exact host, status, body, and
      identifier validation.
- [x] Remove the unused IRC and local IRC rate-limit implementation.
- [x] Run focused protocol and HTTP tests.

### Task 5: EventSub worker

**Files:**
- Modify: `crates/overcrow-overlay/src/twitch/client.rs`
- Modify: `crates/overcrow-overlay/src/twitch/client_tests.rs`
- Modify: `crates/overcrow-overlay/src/twitch/model.rs`

- [x] Add failing end-to-end worker tests for connect/subscribe, arbitrary
      public channel, duplicate delivery, reconnect URL, channel switching,
      lifecycle loss, accepted send, rejected send, and bounded shutdown.
- [x] Replace IRC handshake/frame/send state with EventSub session and Helix
      command state.
- [x] Deduplicate notifications and reconcile optimistic sends by Twitch
      message ID only.
- [x] Coalesce snapshot publication so high-traffic chat does not clone and
      repaint once per provider frame.
- [x] Run all Twitch tests.

### Task 6: UI and durable settings

**Files:**
- Modify: `crates/overcrow-overlay/src/widgets/twitch_chat.rs`
- Modify: `crates/overcrow-overlay/src/widgets/twitch_chat_tests.rs`
- Modify: `crates/overcrow-overlay/src/app.rs`
- Modify: `crates/overcrow-config/src/twitch_prefs.rs`

- [x] Add a failing widget test proving usernames use strong text and no icon
      layout.
- [x] Render usernames in bold while preserving configured name color.
- [x] Reset pause state when the user intentionally selects another channel.
- [x] Move Twitch preference durability work off the egui callback path or use
      the existing bounded settings worker pattern.
- [x] Reuse existing private transactional storage primitives where practical;
      add failure-path tests for any retained specialized code.
- [x] Run config and widget tests.

### Task 7: Product truth and final verification

**Files:**
- Modify: `README.md`
- Modify: `docs/architecture.md`
- Modify: `docs/testing/manual-mvp.md`
- Modify: `docs/troubleshooting.md`
- Modify: `AGENTS.md` if provider constraints changed

- [x] Remove every promise of rendered badges/emotes and document bold
      text-only usernames.
- [x] Document EventSub/Helix, reauthorization, recovery, and safe lifecycle.
- [x] Run formatting, Clippy, workspace tests, dependency policy, relevant
      shell/KWin smoke tests, `git diff --check`, and repository status.
- [x] Report exact automated results and a short real-machine Hyprland/Twitch
      checklist without claiming unperformed live acceptance.
