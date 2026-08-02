import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { App } from './App';
import {
  logSnapshot,
  memoryClient,
  memoryStorage,
  snapshot,
  updateState,
} from './test/fixtures';
import { APP_VERSION } from './version';

describe('Control Center onboarding', () => {
  it('starts disabled and does not scan games before the user starts setup', async () => {
    const client = memoryClient(snapshot());
    render(<App client={client} storage={memoryStorage()} />);

    expect(await screen.findByText('Your games. Your overlay. Your control.')).toBeVisible();
    const brand = screen.getByLabelText('PlayerVox OverCrow');
    const player = within(brand).getByText('Player');
    const vox = within(brand).getByText('Vox');
    expect(player).toBeVisible();
    expect(vox).toBeVisible();
    expect(vox).toHaveClass('brand__accent');
    expect(within(brand).getByText('OverCrow')).toBeVisible();
    expect(brand).toHaveAccessibleName('PlayerVox OverCrow');
    expect(screen.queryByText('by PlayerVox')).not.toBeInTheDocument();
    expect(client.calls).toEqual([
      'subscribe',
      'subscribeUpdates',
      'getState',
      'getUpdateState',
      'checkForUpdates:false',
    ]);

    fireEvent.click(screen.getByRole('button', { name: /check my system/i }));
    expect(await screen.findByText('Supported')).toBeVisible();
    expect(screen.getByText('Graphics: Intel + NVIDIA')).toBeVisible();
    expect(client.calls).toContain('refreshGames');
    expect(client.calls).not.toContain('setEnabled:true');
  });

  it('shows for-now wording and blocks setup on unsupported desktops', async () => {
    const client = memoryClient(
      snapshot({
        compatibility: {
          operating_system: 'Fedora Linux',
          session: 'wayland',
          desktop: 'gnome',
          status: 'not_compatible_for_now',
          reason: 'gnome_wayland',
          activation_allowed: false,
          graphics: ['nvidia'],
        },
        master_switch_enabled: false,
      }),
    );
    render(<App client={client} storage={memoryStorage()} />);

    fireEvent.click(await screen.findByRole('button', { name: /check my system/i }));
    expect(await screen.findByText('Not compatible — for now')).toBeVisible();
    expect(screen.getByRole('button', { name: 'Continue' })).toBeDisabled();
    expect(screen.getByText(/support is a work in progress/i)).toBeVisible();
  });

  it('persists onboarding only after explicit game selection and completion', async () => {
    const client = memoryClient(snapshot());
    const storage = memoryStorage();
    render(<App client={client} storage={storage} />);

    fireEvent.click(await screen.findByRole('button', { name: /check my system/i }));
    fireEvent.click(await screen.findByRole('button', { name: 'Continue' }));
    expect(
      screen.getByRole('button', { name: 'Add a native game' })
        .querySelector('.lucide-plus'),
    ).not.toBeNull();
    expect(
      screen.getByRole('button', { name: 'Continue' })
        .querySelector('.lucide-arrow-right'),
    ).not.toBeNull();
    const checkbox = await screen.findByRole('checkbox');
    fireEvent.click(checkbox);
    await waitFor(() => expect(client.calls).toContain('setGameSelected:4242:true'));
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));
    expect(document.querySelector('.ready-mark .lucide-check')).not.toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /finish with overcrow off/i }));

    expect(await screen.findByText('Stopped')).toBeVisible();
    expect(storage.getItem('overcrow.onboardingVersion')).toBe('1');
    expect(client.calls).not.toContain('setEnabled:true');
  });

  it('honors finish-off when onboarding state was reset while OverCrow remained enabled', async () => {
    const client = memoryClient(
      snapshot({
        lifecycle: 'enabled',
        master_switch_checked: true,
        selection_editing_enabled: false,
        games: [{ app_id: 4242, name: 'Example Game', kind: 'steam_game', selected: true }],
      }),
    );
    const storage = memoryStorage();
    render(<App client={client} storage={storage} />);

    fireEvent.click(await screen.findByRole('button', { name: /check my system/i }));
    fireEvent.click(await screen.findByRole('button', { name: 'Continue' }));
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));
    fireEvent.click(screen.getByRole('button', { name: /finish with overcrow off/i }));

    await waitFor(() => expect(client.calls).toContain('setEnabled:false'));
    expect(await screen.findByText('Stopped')).toBeVisible();
  });
});

describe('Control Center dashboard', () => {
  it('shows an available native update and installs only after an explicit click', async () => {
    const storage = memoryStorage();
    storage.setItem('overcrow.onboardingVersion', '1');
    const client = memoryClient(
      snapshot(),
      updateState({
        phase: 'available',
        latest_version: '0.1.0-pre-alpha.5',
        install_kind: 'arch',
      }),
    );
    render(<App client={client} storage={storage} />);

    expect(
      await screen.findByText('PlayerVox OverCrow 0.1.0-pre-alpha.5 is available'),
    ).toBeVisible();
    expect(
      screen
        .getByText('PlayerVox OverCrow 0.1.0-pre-alpha.5 is available')
        .closest('.update-panel')
        ?.querySelector('.lucide-download'),
    ).not.toBeNull();
    expect(client.calls).not.toContain('installAvailableUpdate');
    fireEvent.click(screen.getByRole('button', { name: 'Update now' }));
    await waitFor(() =>
      expect(client.calls).toContain('installAvailableUpdate'),
    );
    expect(await screen.findByText('Update installed')).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Restart now' }));
    expect(client.calls).toContain('restartControlCenter');
  });

  it('keeps unknown package layouts manual and checks again from About', async () => {
    const storage = memoryStorage();
    storage.setItem('overcrow.onboardingVersion', '1');
    const client = memoryClient(
      snapshot(),
      updateState({
        phase: 'manual',
        latest_version: '0.1.0-pre-alpha.5',
        install_kind: 'manual',
      }),
    );
    render(<App client={client} storage={storage} />);

    expect(
      await screen.findByRole('button', { name: 'Open release page' }),
    ).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Open release page' }));
    await waitFor(() => expect(client.calls).toContain('openUpdatePage'));
    fireEvent.click(screen.getByRole('button', { name: 'About' }));
    fireEvent.click(screen.getByRole('button', { name: 'Check for updates' }));
    await waitFor(() =>
      expect(client.calls).toContain('checkForUpdates:true'),
    );
  });

  it('stages rpm-ostree updates with explicit reboot guidance', async () => {
    const storage = memoryStorage();
    storage.setItem('overcrow.onboardingVersion', '1');
    const client = memoryClient(
      snapshot(),
      updateState({
        phase: 'available',
        latest_version: '0.1.0-pre-alpha.5',
        install_kind: 'rpm_ostree',
      }),
    );
    render(<App client={client} storage={storage} />);

    fireEvent.click(
      await screen.findByRole('button', { name: 'Update now' }),
    );
    expect(
      await screen.findByText('Update staged — restart required'),
    ).toBeVisible();
    expect(screen.getByText(/Bazzite has prepared the next deployment/i)).toBeVisible();
    expect(
      screen.queryByRole('button', { name: 'Restart now' }),
    ).not.toBeInTheDocument();
  });

  it('subscribes before update reads and schedules one six-hour deadline', async () => {
    const storage = memoryStorage();
    storage.setItem('overcrow.onboardingVersion', '1');
    const client = memoryClient(snapshot());
    const timeout = vi.spyOn(window, 'setTimeout');
    const view = render(<App client={client} storage={storage} />);

    await waitFor(() =>
      expect(client.calls).toContain('checkForUpdates:false'),
    );
    expect(client.calls.indexOf('subscribeUpdates')).toBeLessThan(
      client.calls.indexOf('getUpdateState'),
    );
    expect(timeout).toHaveBeenCalledWith(expect.any(Function), 6 * 60 * 60 * 1_000);

    view.unmount();
    timeout.mockRestore();
  });

  it('registers native state events before loading the baseline', async () => {
    const client = memoryClient(snapshot());
    let finishSubscription: ((unsubscribe: () => void) => void) | undefined;
    client.subscribe = () => {
      client.calls.push('subscribe');
      return new Promise((resolve) => {
        finishSubscription = resolve;
      });
    };

    render(<App client={client} storage={memoryStorage()} />);

    await waitFor(() =>
      expect(
        client.calls.filter((call) => call === 'subscribe' || call === 'getState'),
      ).toEqual(['subscribe']),
    );
    expect(client.calls).not.toContain('getState');
    await act(async () => {
      finishSubscription?.(() => {});
    });
    expect(await screen.findByText('Your games. Your overlay. Your control.')).toBeVisible();
    expect(
      client.calls.filter((call) => call === 'subscribe' || call === 'getState'),
    ).toEqual(['subscribe', 'getState']);
  });

  it('loads returning users, refreshes discovery, and keeps activation explicit', async () => {
    const storage = memoryStorage();
    storage.setItem('overcrow.onboardingVersion', '1');
    const client = memoryClient(
      snapshot({ games: [{ app_id: 4242, name: 'Example Game', kind: 'steam_game', selected: true }] }),
    );
    render(<App client={client} storage={storage} />);

    expect(await screen.findByText('Stopped')).toBeVisible();
    for (const [label, icon] of [
      ['Overview', 'layout-grid'],
      ['Games', 'gamepad-2'],
      ['Diagnostics', 'activity'],
      ['About', 'info'],
    ]) {
      expect(
        screen
          .getByRole('button', { name: label })
          .querySelector(`.lucide-${icon}`),
      ).not.toBeNull();
    }
    await waitFor(() => expect(client.calls).toContain('refreshGames'));
    expect(
      screen.queryByRole('checkbox', { name: 'Enable OverCrow globally' }),
    ).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Start OverCrow' }));
    await waitFor(() => expect(client.calls).toContain('setEnabled:true'));
    expect(await screen.findByText('Running')).toBeVisible();

    fireEvent.click(screen.getByRole('button', { name: 'Stop OverCrow' }));
    await waitFor(() => expect(client.calls).toContain('setEnabled:false'));
    expect(await screen.findByText('Stopped')).toBeVisible();

    fireEvent.click(screen.getByRole('button', { name: 'Games' }));
    expect(screen.getByRole('checkbox', { name: /Example Game.*Steam · App 4242/ })).toBeChecked();
    expect(
      screen.getByRole('button', { name: /Add a native game/ })
        .querySelector('.lucide-plus'),
    ).not.toBeNull();
  });

  it('labels Steam shortcuts and unverified application types', async () => {
    const storage = memoryStorage();
    storage.setItem('overcrow.onboardingVersion', '1');
    const client = memoryClient(
      snapshot({
        games: [
          { app_id: 101, name: 'Shortcut Game', kind: 'steam_shortcut', selected: false },
          { app_id: 202, name: 'Unverified Game', kind: 'unverified', selected: false },
        ],
      }),
    );
    render(<App client={client} storage={storage} />);

    fireEvent.click(await screen.findByRole('button', { name: 'Games' }));

    expect(screen.getByText('Steam shortcut · App 101')).toBeVisible();
    expect(screen.getByText('Type unverified · App 202')).toBeVisible();
  });

  it('shows the release version below compatibility and on the About page', async () => {
    const storage = memoryStorage();
    storage.setItem('overcrow.onboardingVersion', '1');
    render(<App client={memoryClient(snapshot())} storage={storage} />);

    expect(await screen.findByText(`v${APP_VERSION}`)).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'About' }));
    expect(screen.getByText(`Version ${APP_VERSION}`)).toBeVisible();
    expect(
      screen.getByText('Commercial licenses for proprietary use are available from Valhallab.'),
    ).toBeVisible();
    expect(screen.getByText('contact@valhallab.com')).toBeVisible();
  });

  it('locks the lifecycle action while OverCrow is starting', async () => {
    const storage = memoryStorage();
    storage.setItem('overcrow.onboardingVersion', '1');
    const client = memoryClient(
      snapshot({
        lifecycle: 'enabling',
        master_switch_enabled: false,
        master_switch_checked: true,
        operations: { refresh: false, picker: false, lifecycle: true },
      }),
    );
    render(<App client={client} storage={storage} />);

    expect(await screen.findByRole('button', { name: 'Starting…' })).toBeDisabled();
  });

  it('applies lifecycle changes initiated from the system tray', async () => {
    const storage = memoryStorage();
    storage.setItem('overcrow.onboardingVersion', '1');
    const initial = snapshot({
      games: [{ app_id: 4242, name: 'Example Game', kind: 'steam_game', selected: true }],
    });
    const client = memoryClient(initial);
    render(<App client={client} storage={storage} />);

    expect(await screen.findByText('Stopped')).toBeVisible();
    client.emitState({
      ...initial,
      lifecycle: 'enabled',
      master_switch_checked: true,
      selection_editing_enabled: false,
    });

    expect(await screen.findByText('Running')).toBeVisible();
  });

  it('translates command failures into bounded friendly copy', async () => {
    const storage = memoryStorage();
    storage.setItem('overcrow.onboardingVersion', '1');
    const client = memoryClient(snapshot());
    client.refreshGames = async () => Promise.reject('state_unavailable');
    render(<App client={client} storage={storage} />);

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'The Control Center state is temporarily unavailable.',
    );
  });

  it('loads, filters, refreshes, and copies logs only from the Logs tab', async () => {
    const storage = memoryStorage();
    storage.setItem('overcrow.onboardingVersion', '1');
    const client = memoryClient(snapshot());
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    });
    render(<App client={client} storage={storage} />);

    fireEvent.click(await screen.findByRole('button', { name: 'Diagnostics' }));
    expect(screen.getByRole('tab', { name: 'Overview' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    expect(screen.getByText('Desktop session')).toBeVisible();
    expect(client.calls).not.toContain('getRecentLogs');

    fireEvent.click(screen.getByRole('tab', { name: 'Logs' }));
    await waitFor(() =>
      expect(client.calls.filter((call) => call === 'getRecentLogs')).toHaveLength(1),
    );
    expect(screen.getByText(/game_detected/)).toBeVisible();
    expect(screen.getByText(/frame_late/)).toBeVisible();

    fireEvent.change(screen.getByLabelText('Component'), {
      target: { value: 'overlay' },
    });
    expect(screen.queryByText(/game_detected/)).not.toBeInTheDocument();
    expect(screen.getByText(/frame_late/)).toBeVisible();

    fireEvent.change(screen.getByLabelText('Level'), {
      target: { value: 'WARN' },
    });
    fireEvent.change(screen.getByLabelText('Search logs'), {
      target: { value: 'frame_late' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Copy visible logs' }));
    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith(
        '2026-07-23T10:00:01.000Z WARN overlay frame_late count=1',
      ),
    );

    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() =>
      expect(client.calls.filter((call) => call === 'getRecentLogs')).toHaveLength(2),
    );
  });

  it('keeps the last successful logs when a manual refresh fails', async () => {
    const storage = memoryStorage();
    storage.setItem('overcrow.onboardingVersion', '1');
    const client = memoryClient(snapshot());
    let attempts = 0;
    client.getRecentLogs = async () => {
      client.calls.push('getRecentLogs');
      attempts += 1;
      if (attempts === 1) return structuredClone(logSnapshot());
      return Promise.reject('logs_unavailable');
    };
    render(<App client={client} storage={storage} />);

    fireEvent.click(await screen.findByRole('button', { name: 'Diagnostics' }));
    fireEvent.click(screen.getByRole('tab', { name: 'Logs' }));
    expect(await screen.findByText(/game_detected/)).toBeVisible();

    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    expect(await screen.findByText('Logs could not be refreshed.')).toBeVisible();
    expect(screen.getByText(/game_detected/)).toBeVisible();
    expect(
      screen.queryByText('OverCrow could not complete that action'),
    ).not.toBeInTheDocument();
  });

  it('previews, copies, and explicitly sends a private support report', async () => {
    const storage = memoryStorage();
    storage.setItem('overcrow.onboardingVersion', '1');
    const supportReport = {
      schema_version: 1 as const,
      report_id: 'oc-20260724t100000000z-0000',
      created_at: '2026-07-24T10:00:00.000Z',
      content: '# PlayerVox OverCrow support report\n\nExact preview.',
      logs_included: true,
    };
    const client = Object.assign(memoryClient(snapshot()), {
      prepareSupportReport: vi.fn().mockResolvedValue(supportReport),
      submitSupportReport: vi.fn().mockResolvedValue({
        reference: 'bfaf03ce-5471-4739-a145-1ca24f215f1b',
        received_at: '2026-07-25T10:00:00Z',
      }),
    });
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    });
    render(<App client={client} storage={storage} />);

    fireEvent.click(await screen.findByRole('button', { name: 'Diagnostics' }));
    fireEvent.click(screen.getByRole('button', { name: 'Report a problem' }));
    const dialog = screen.getByRole('dialog', { name: 'Report a problem' });
    expect(
      dialog.querySelector('.report-dialog__close .lucide-x'),
    ).not.toBeNull();
    const prepare = within(dialog).getByRole('button', { name: 'Prepare report' });
    expect(prepare).toBeDisabled();
    expect(
      within(dialog).getByRole('checkbox', { name: 'Include sanitized logs' }),
    ).toBeChecked();

    fireEvent.change(within(dialog).getByLabelText('What happened?'), {
      target: { value: 'The overlay stopped responding.' },
    });
    fireEvent.click(prepare);
    await waitFor(() =>
      expect(client.prepareSupportReport).toHaveBeenCalledWith(
        'The overlay stopped responding.',
        true,
      ),
    );
    expect(await within(dialog).findByText(/Exact preview\./)).toBeVisible();

    fireEvent.click(within(dialog).getByRole('button', { name: 'Copy report' }));
    fireEvent.click(within(dialog).getByRole('button', { name: 'Send report' }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith(supportReport.content);
      expect(client.submitSupportReport).toHaveBeenCalledWith(
        supportReport.report_id,
      );
    });
    expect(
      within(dialog).getByText(/bfaf03ce-5471-4739-a145-1ca24f215f1b/),
    ).toBeVisible();

    fireEvent.change(within(dialog).getByLabelText('What happened?'), {
      target: { value: 'A different problem.' },
    });
    expect(within(dialog).queryByText(/Exact preview\./)).not.toBeInTheDocument();
  });

  it('enforces the support description byte limit before invoking native code', async () => {
    const storage = memoryStorage();
    storage.setItem('overcrow.onboardingVersion', '1');
    const client = Object.assign(memoryClient(snapshot()), {
      prepareSupportReport: vi.fn(),
      submitSupportReport: vi.fn(),
    });
    render(<App client={client} storage={storage} />);

    fireEvent.click(await screen.findByRole('button', { name: 'Diagnostics' }));
    fireEvent.click(screen.getByRole('button', { name: 'Report a problem' }));
    const dialog = screen.getByRole('dialog', { name: 'Report a problem' });
    fireEvent.change(within(dialog).getByLabelText('What happened?'), {
      target: { value: 'é'.repeat(1_001) },
    });

    expect(within(dialog).getByText('2,002 / 2,000 bytes')).toBeVisible();
    expect(
      within(dialog).getByRole('button', { name: 'Prepare report' }),
    ).toBeDisabled();
    expect(client.prepareSupportReport).not.toHaveBeenCalled();
  });

  it('keeps the preview available when submission fails', async () => {
    const storage = memoryStorage();
    storage.setItem('overcrow.onboardingVersion', '1');
    const client = Object.assign(memoryClient(snapshot()), {
      prepareSupportReport: vi.fn().mockResolvedValue({
        schema_version: 1,
        report_id: 'oc-test',
        created_at: '2026-07-24T10:00:00.000Z',
        content: '# PlayerVox OverCrow support report',
        logs_included: false,
      }),
      submitSupportReport: vi.fn().mockRejectedValue(
        'support_network_unavailable',
      ),
    });
    render(<App client={client} storage={storage} />);

    fireEvent.click(await screen.findByRole('button', { name: 'Diagnostics' }));
    fireEvent.click(screen.getByRole('button', { name: 'Report a problem' }));
    const dialog = screen.getByRole('dialog', { name: 'Report a problem' });
    fireEvent.change(within(dialog).getByLabelText('What happened?'), {
      target: { value: 'The overlay stopped responding.' },
    });
    fireEvent.click(
      within(dialog).getByRole('button', { name: 'Prepare report' }),
    );

    fireEvent.click(
      await within(dialog).findByRole('button', { name: 'Send report' }),
    );
    expect(
      await within(dialog).findByText(
        'The report could not be sent. Check your connection and try again.',
      ),
    ).toBeVisible();
    expect(
      within(dialog).getByRole('button', { name: 'Copy report' }),
    ).toBeEnabled();
  });
});
