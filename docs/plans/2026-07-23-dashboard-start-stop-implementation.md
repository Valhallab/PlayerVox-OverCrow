# Dashboard Start/Stop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the ambiguous dashboard-wide toggle with an explicit
Start/Stop action inside the OverCrow status card.

**Architecture:** Keep the existing `ControlClient.setEnabled(boolean)`
lifecycle boundary unchanged. Only the React presentation, English copy,
styles, and component tests change.

**Tech Stack:** React, TypeScript, CSS, Vitest, Testing Library

## Global Constraints

- Allowed-game selections remain the only authorization scope.
- Stopping OverCrow preserves game selections and settings.
- Lifecycle operations and unsupported configurations remain fail-closed.
- Do not change native services, D-Bus contracts, or persistence.

---

### Task 1: Move lifecycle control into the status card

**Files:**
- Modify: `crates/overcrow-control-ui/src/App.test.tsx`
- Modify: `crates/overcrow-control-ui/src/components/Dashboard.tsx`
- Modify: `crates/overcrow-control-ui/src/i18n/en.ts`
- Modify: `crates/overcrow-control-ui/src/styles.css`

**Interfaces:**
- Consumes: `ControlSnapshot.lifecycle`,
  `ControlSnapshot.master_switch_checked`,
  `ControlSnapshot.master_switch_enabled`, and
  `DashboardProps.onEnable(enabled: boolean)`.
- Produces: the `Start OverCrow` / `Stop OverCrow` status-card action without
  changing any public interface.

- [ ] **Step 1: Replace the toggle assertion with the desired actions**

In the returning-user dashboard test, assert that the global checkbox is gone,
start OverCrow, then stop it:

```tsx
expect(
  screen.queryByRole('checkbox', { name: 'Enable OverCrow globally' }),
).not.toBeInTheDocument();

fireEvent.click(screen.getByRole('button', { name: 'Start OverCrow' }));
await waitFor(() => expect(client.calls).toContain('setEnabled:true'));
expect(await screen.findByText('Running')).toBeVisible();

fireEvent.click(screen.getByRole('button', { name: 'Stop OverCrow' }));
await waitFor(() => expect(client.calls).toContain('setEnabled:false'));
expect(await screen.findByText('Stopped')).toBeVisible();
```

Add a second dashboard test using `lifecycle: 'enabling'`,
`operations.lifecycle: true`, and `master_switch_enabled: false`; assert that
the `Starting…` action is disabled.

- [ ] **Step 2: Run the frontend test and verify failure**

Run:

```sh
npm --prefix crates/overcrow-control-ui test -- --run
```

Expected: FAIL because the dashboard still exposes the global checkbox and no
status-card Start/Stop action.

- [ ] **Step 3: Implement the minimal presentation change**

Remove the `master-toggle` label from `Dashboard`'s header. Derive lifecycle
copy from the existing snapshot:

```tsx
const lifecycleTitle =
  snapshot.lifecycle === 'enabling'
    ? en.dashboard.starting
    : snapshot.lifecycle === 'disabling'
      ? en.dashboard.stopping
      : snapshot.lifecycle === 'warning'
        ? en.dashboard.warning
        : snapshot.master_switch_checked
          ? en.dashboard.running
          : en.dashboard.stopped;

const lifecycleAction =
  snapshot.lifecycle === 'enabling'
    ? en.dashboard.starting
    : snapshot.lifecycle === 'disabling'
      ? en.dashboard.stopping
      : snapshot.master_switch_checked
        ? en.dashboard.stop
        : en.dashboard.start;
```

Inside `status-hero`, render:

```tsx
<button
  className={`button ${snapshot.master_switch_checked ? 'button--secondary' : 'button--primary'} status-hero__action`}
  disabled={busy || !snapshot.master_switch_enabled}
  onClick={() => props.onEnable(!snapshot.master_switch_checked)}
>
  {lifecycleAction}
</button>
```

Replace the dashboard lifecycle strings with:

```ts
running: 'Running',
stopped: 'Stopped',
starting: 'Starting…',
stopping: 'Stopping…',
warning: 'Needs attention',
start: 'Start OverCrow',
stop: 'Stop OverCrow',
systemStatus: 'OverCrow status',
```

Delete `masterToggle` and the unused `.master-toggle` styles. Add
`.status-hero__action { min-width: 140px; margin-left: auto; }` and stack the
action below the copy in the existing `max-width: 860px` media query.

- [ ] **Step 4: Verify tests and the production build**

Run:

```sh
npm --prefix crates/overcrow-control-ui test
npm --prefix crates/overcrow-control-ui run build
git diff --check
```

Expected: all frontend tests pass, the production build succeeds, and
`git diff --check` emits no output.

- [ ] **Step 5: Commit the complete UI change**

```sh
git add crates/overcrow-control-ui/src/App.test.tsx \
  crates/overcrow-control-ui/src/components/Dashboard.tsx \
  crates/overcrow-control-ui/src/i18n/en.ts \
  crates/overcrow-control-ui/src/styles.css
git commit -m "refactor(control-ui): clarify lifecycle control"
```
