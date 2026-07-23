import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { App } from './App';
import {
  logSnapshot,
  memoryClient,
  memoryStorage,
  snapshot,
} from './test/fixtures';

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
    expect(client.calls).toEqual(['getState']);

    fireEvent.click(screen.getByRole('button', { name: /check my system/i }));
    expect(await screen.findByText('Supported')).toBeVisible();
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
    const checkbox = await screen.findByRole('checkbox');
    fireEvent.click(checkbox);
    await waitFor(() => expect(client.calls).toContain('setGameSelected:4242:true'));
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));
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
        games: [{ app_id: 4242, name: 'Example Game', selected: true }],
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
  it('loads returning users, refreshes discovery, and keeps activation explicit', async () => {
    const storage = memoryStorage();
    storage.setItem('overcrow.onboardingVersion', '1');
    const client = memoryClient(
      snapshot({ games: [{ app_id: 4242, name: 'Example Game', selected: true }] }),
    );
    render(<App client={client} storage={storage} />);

    expect(await screen.findByText('Stopped')).toBeVisible();
    for (const label of ['Overview', 'Games', 'Diagnostics', 'About']) {
      expect(screen.getByRole('button', { name: label }).querySelector('svg')).not.toBeNull();
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
});
