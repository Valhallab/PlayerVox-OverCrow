import { en } from '../i18n/en';
import type { ControlUpdateState } from '../lib/control';

interface UpdatePanelProps {
  variant: 'overview' | 'about';
  state: ControlUpdateState | null;
  actionError: string | null;
  onCheck(): void;
  onInstall(): void;
  onOpenRelease(): void;
}

export function UpdatePanel({
  variant,
  state,
  actionError,
  onCheck,
  onInstall,
  onOpenRelease,
}: UpdatePanelProps) {
  if (
    variant === 'overview' &&
    (!state ||
      state.phase === 'idle' ||
      state.phase === 'checking' ||
      state.phase === 'up_to_date')
  ) {
    return null;
  }

  const presentation = updatePresentation(state);
  const working =
    state?.phase === 'checking' ||
    state?.phase === 'downloading' ||
    state?.phase === 'installing';
  const canInstall =
    state?.phase === 'available' && state.install_kind !== 'manual';
  const canOpen =
    state?.phase === 'manual' ||
    state?.phase === 'available' ||
    state?.phase === 'failed';

  return (
    <section
      className={`update-panel update-panel--${variant} update-panel--${state?.phase ?? 'idle'}`}
      aria-live="polite"
    >
      <div className="update-panel__mark" aria-hidden="true">
        <svg viewBox="0 0 24 24">
          <path d="M12 3v12m0 0 4-4m-4 4-4-4" />
          <path d="M5 19h14" />
        </svg>
      </div>
      <div className="update-panel__copy">
        <div className="eyebrow">{en.updates.eyebrow}</div>
        <h2>{presentation.title}</h2>
        <p>{presentation.body}</p>
        {actionError && <p className="update-panel__error">{actionError}</p>}
        {variant === 'about' && state?.last_checked_at && (
          <small>
            {en.updates.lastChecked}{' '}
            {new Date(state.last_checked_at).toLocaleString()}
          </small>
        )}
      </div>
      <div className="update-panel__actions">
        {canInstall && (
          <button
            className="button button--primary"
            disabled={working}
            onClick={onInstall}
          >
            {en.updates.updateNow}
          </button>
        )}
        {canOpen && (
          <button
            className="button button--secondary"
            disabled={working}
            onClick={onOpenRelease}
          >
            {en.updates.openRelease}
          </button>
        )}
        {variant === 'about' &&
          state?.phase !== 'downloading' &&
          state?.phase !== 'installing' && (
            <button
              className="button button--secondary"
              disabled={working}
              onClick={onCheck}
            >
              {en.updates.check}
            </button>
          )}
      </div>
    </section>
  );
}

function updatePresentation(state: ControlUpdateState | null): {
  title: string;
  body: string;
} {
  if (!state || state.phase === 'idle') {
    return { title: en.updates.readyTitle, body: en.updates.readyBody };
  }
  if (state.phase === 'checking') {
    return { title: en.updates.checkingTitle, body: en.updates.checkingBody };
  }
  if (state.phase === 'up_to_date') {
    return {
      title: en.updates.currentTitle,
      body: en.updates.currentBody(state.current_version),
    };
  }
  if (state.phase === 'available' || state.phase === 'manual') {
    return {
      title: en.updates.availableTitle(
        state.latest_version ?? state.current_version,
      ),
      body:
        state.phase === 'manual'
          ? en.updates.manualBody
          : en.updates.availableBody,
    };
  }
  if (state.phase === 'downloading') {
    return {
      title: en.updates.downloadingTitle,
      body: en.updates.downloadingBody,
    };
  }
  if (state.phase === 'installing') {
    return {
      title: en.updates.installingTitle,
      body: en.updates.installingBody,
    };
  }
  if (state.phase === 'installed') {
    return {
      title: en.updates.installedTitle,
      body: en.updates.installedBody,
    };
  }
  if (state.phase === 'restart_required') {
    return {
      title: en.updates.restartTitle,
      body: en.updates.restartBody,
    };
  }
  return {
    title: en.updates.failedTitle,
    body:
      (state.error && en.updates.stateErrors[state.error]) ??
      en.updates.stateErrors.generic,
  };
}
