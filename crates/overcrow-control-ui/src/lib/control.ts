import { invoke } from '@tauri-apps/api/core';

export type CompatibilityStatus =
  | 'supported'
  | 'validation_in_progress'
  | 'experimental_for_now'
  | 'not_compatible_for_now'
  | 'unknown';

export type CompatibilityReason =
  | 'hyprland_wayland'
  | 'plasma_wayland'
  | 'generic_x11'
  | 'gnome_wayland'
  | 'sway_wayland'
  | 'gamescope_session'
  | 'xfce_x11'
  | 'other_wayland'
  | 'ambiguous_desktop'
  | 'unknown_session';

export interface ControlSnapshot {
  schema_version: number;
  compatibility: {
    operating_system: string;
    session: 'wayland' | 'x11' | 'unknown';
    desktop:
      | 'hyprland'
      | 'plasma'
      | 'gnome'
      | 'sway'
      | 'xfce'
      | 'gamescope'
      | 'other'
      | 'ambiguous'
      | 'unknown';
    status: CompatibilityStatus;
    reason: CompatibilityReason;
    activation_allowed: boolean;
  };
  lifecycle: 'disabled' | 'enabled' | 'warning' | 'enabling' | 'disabling';
  master_switch_enabled: boolean;
  master_switch_checked: boolean;
  selection_editing_enabled: boolean;
  shortcut: string;
  operations: {
    refresh: boolean;
    picker: boolean;
    lifecycle: boolean;
  };
  games: Array<{ app_id: number; name: string; selected: boolean }>;
  manual_games: Array<{ id: string; name: string; executable: string }>;
  notices: Array<{
    operation: 'selection_save' | 'refresh' | 'picker' | 'lifecycle';
    level: 'warning' | 'error';
    message: string;
  }>;
  diagnostics: Array<{
    label: string;
    detail: string;
    level: 'ok' | 'info' | 'warning' | 'error';
  }>;
}

export interface ControlLogSnapshot {
  schema_version: 1;
  lines: string[];
  truncated: boolean;
}

export interface ControlClient {
  getState(): Promise<ControlSnapshot>;
  getRecentLogs(): Promise<ControlLogSnapshot>;
  refreshGames(): Promise<ControlSnapshot>;
  setGameSelected(appId: number, selected: boolean): Promise<ControlSnapshot>;
  removeManualGame(id: string): Promise<ControlSnapshot>;
  pickManualGame(): Promise<ControlSnapshot>;
  setEnabled(enabled: boolean): Promise<ControlSnapshot>;
}

export const controlClient: ControlClient = {
  getState: () => invoke('get_control_state'),
  getRecentLogs: () => invoke('get_recent_logs'),
  refreshGames: () => invoke('refresh_games'),
  setGameSelected: (appId, selected) =>
    invoke('set_game_selected', { appId, selected }),
  removeManualGame: (id) => invoke('remove_manual_game', { id }),
  pickManualGame: () => invoke('pick_manual_game'),
  setEnabled: (enabled) => invoke('set_overcrow_enabled', { enabled }),
};

export function hasOperationInFlight(snapshot: ControlSnapshot): boolean {
  return Object.values(snapshot.operations).some(Boolean);
}
