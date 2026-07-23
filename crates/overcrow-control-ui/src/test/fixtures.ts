import type {
  ControlClient,
  ControlLogSnapshot,
  ControlSnapshot,
} from '../lib/control';

export function snapshot(
  overrides: Partial<ControlSnapshot> = {},
): ControlSnapshot {
  return {
    schema_version: 1,
    compatibility: {
      operating_system: 'Arch Linux',
      session: 'wayland',
      desktop: 'hyprland',
      status: 'supported',
      reason: 'hyprland_wayland',
      activation_allowed: true,
    },
    lifecycle: 'disabled',
    master_switch_enabled: true,
    master_switch_checked: false,
    selection_editing_enabled: true,
    shortcut: 'SUPER+ALT+O',
    operations: { refresh: false, picker: false, lifecycle: false },
    games: [{ app_id: 4242, name: 'Example Game', selected: false }],
    manual_games: [],
    notices: [],
    diagnostics: [{ label: 'Desktop session', detail: 'Wayland — Hyprland detected.', level: 'ok' }],
    ...overrides,
  };
}

export function logSnapshot(
  overrides: Partial<ControlLogSnapshot> = {},
): ControlLogSnapshot {
  return {
    schema_version: 1,
    lines: [
      '2026-07-23T10:00:00.000Z INFO core game_detected app_id=4242',
      '2026-07-23T10:00:01.000Z WARN overlay frame_late count=1',
    ],
    truncated: false,
    ...overrides,
  };
}

export function memoryClient(initial: ControlSnapshot): ControlClient & {
  calls: string[];
  emitState(snapshot: ControlSnapshot): void;
} {
  let current = structuredClone(initial);
  const logs = logSnapshot();
  const calls: string[] = [];
  const listeners = new Set<(snapshot: ControlSnapshot) => void>();
  return {
    calls,
    async subscribe(listener) {
      calls.push('subscribe');
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    emitState(snapshot) {
      current = structuredClone(snapshot);
      for (const listener of listeners) listener(structuredClone(current));
    },
    async getState() {
      calls.push('getState');
      return structuredClone(current);
    },
    async getRecentLogs() {
      calls.push('getRecentLogs');
      return structuredClone(logs);
    },
    async refreshGames() {
      calls.push('refreshGames');
      return structuredClone(current);
    },
    async setGameSelected(appId, selected) {
      calls.push(`setGameSelected:${appId}:${selected}`);
      current = {
        ...current,
        games: current.games.map((game) =>
          game.app_id === appId ? { ...game, selected } : game,
        ),
      };
      return structuredClone(current);
    },
    async removeManualGame(id) {
      calls.push(`removeManualGame:${id}`);
      current = {
        ...current,
        manual_games: current.manual_games.filter((game) => game.id !== id),
      };
      return structuredClone(current);
    },
    async pickManualGame() {
      calls.push('pickManualGame');
      return structuredClone(current);
    },
    async setEnabled(enabled) {
      calls.push(`setEnabled:${enabled}`);
      current = {
        ...current,
        lifecycle: enabled ? 'enabled' : 'disabled',
        master_switch_checked: enabled,
      };
      return structuredClone(current);
    },
  };
}

export function memoryStorage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() {
      return values.size;
    },
    clear: () => values.clear(),
    getItem: (key) => values.get(key) ?? null,
    key: (index) => [...values.keys()][index] ?? null,
    removeItem: (key) => {
      values.delete(key);
    },
    setItem: (key, value) => {
      values.set(key, value);
    },
  };
}
