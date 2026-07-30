import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

const CONTROL_STATE_EVENT = 'overcrow-control-state';
const UPDATE_STATE_EVENT = 'overcrow-update-state';

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
    graphics: Array<'amd' | 'intel' | 'nvidia' | 'other'>;
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
  games: Array<{
    app_id: number;
    name: string;
    kind: 'steam_game' | 'steam_shortcut' | 'unverified';
    selected: boolean;
  }>;
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

export interface ControlSupportReport {
  schema_version: 1;
  report_id: string;
  created_at: string;
  content: string;
  logs_included: boolean;
}

export interface ControlSupportReceipt {
  reference: string;
  received_at: string;
}

export type UpdatePhase =
  | 'idle'
  | 'checking'
  | 'up_to_date'
  | 'available'
  | 'downloading'
  | 'installing'
  | 'installed'
  | 'restart_required'
  | 'manual'
  | 'failed';

export type UpdateInstallKind =
  | 'arch'
  | 'rpm'
  | 'deb'
  | 'rpm_ostree'
  | 'manual';

export type UpdateErrorCode =
  | 'busy'
  | 'unavailable'
  | 'network'
  | 'invalid_response'
  | 'download'
  | 'verification'
  | 'runtime_stop'
  | 'authorization_cancelled'
  | 'installation'
  | 'timeout'
  | 'open_page';

export interface ControlUpdateState {
  schema_version: 1;
  phase: UpdatePhase;
  current_version: string;
  latest_version: string | null;
  install_kind: UpdateInstallKind;
  last_checked_at: string | null;
  error: UpdateErrorCode | null;
}

export interface ControlClient {
  subscribe(listener: (snapshot: ControlSnapshot) => void): Promise<() => void>;
  subscribeUpdates(
    listener: (state: ControlUpdateState) => void,
  ): Promise<() => void>;
  getState(): Promise<ControlSnapshot>;
  getUpdateState(): Promise<ControlUpdateState>;
  checkForUpdates(force: boolean): Promise<ControlUpdateState>;
  installAvailableUpdate(): Promise<ControlUpdateState>;
  openUpdatePage(): Promise<void>;
  restartControlCenter(): Promise<void>;
  getRecentLogs(): Promise<ControlLogSnapshot>;
  prepareSupportReport(
    description: string,
    includeLogs: boolean,
  ): Promise<ControlSupportReport>;
  submitSupportReport(reportId: string): Promise<ControlSupportReceipt>;
  refreshGames(): Promise<ControlSnapshot>;
  setGameSelected(appId: number, selected: boolean): Promise<ControlSnapshot>;
  removeManualGame(id: string): Promise<ControlSnapshot>;
  pickManualGame(): Promise<ControlSnapshot>;
  setEnabled(enabled: boolean): Promise<ControlSnapshot>;
}

export const controlClient: ControlClient = {
  subscribe: (listener) =>
    listen<ControlSnapshot>(CONTROL_STATE_EVENT, (event) => listener(event.payload)),
  subscribeUpdates: (listener) =>
    listen<ControlUpdateState>(UPDATE_STATE_EVENT, (event) =>
      listener(event.payload),
    ),
  getState: () => invoke('get_control_state'),
  getUpdateState: () => invoke('get_update_state'),
  checkForUpdates: (force) => invoke('check_for_updates', { force }),
  installAvailableUpdate: () => invoke('install_available_update'),
  openUpdatePage: () => invoke('open_update_page'),
  restartControlCenter: () => invoke('restart_control_center'),
  getRecentLogs: () => invoke('get_recent_logs'),
  prepareSupportReport: (description, includeLogs) =>
    invoke('prepare_support_report', { description, includeLogs }),
  submitSupportReport: (reportId) =>
    invoke('submit_support_report', { reportId }),
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
