import { useCallback, useEffect, useState } from 'react';

import { Dashboard } from './components/Dashboard';
import { Onboarding } from './components/Onboarding';
import { en } from './i18n/en';
import {
  controlClient,
  hasOperationInFlight,
  type ControlClient,
  type ControlSnapshot,
} from './lib/control';

const ONBOARDING_KEY = 'overcrow.onboardingVersion';
const ONBOARDING_VERSION = '1';
const POLL_DELAY_MS = 200;

interface AppError {
  code: string;
  message: string;
}

export function App({
  client = controlClient,
  storage = window.localStorage,
}: {
  client?: ControlClient;
  storage?: Storage;
}) {
  const [snapshot, setSnapshot] = useState<ControlSnapshot | null>(null);
  const [onboardingComplete, setOnboardingComplete] = useState(
    () => storage.getItem(ONBOARDING_KEY) === ONBOARDING_VERSION,
  );
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<AppError | null>(null);

  const run = useCallback(async (operation: () => Promise<ControlSnapshot>) => {
    setBusy(true);
    setError(null);
    try {
      setSnapshot(await operation());
      return true;
    } catch (reason) {
      const code = typeof reason === 'string' ? reason : 'generic';
      setError({ code, message: messageForError(reason) });
      return false;
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    let active = true;
    let unsubscribe: (() => void) | undefined;
    void (async () => {
      let subscriptionUnavailable = false;
      try {
        const registered = await client.subscribe((next) => {
          if (active) setSnapshot(next);
        });
        if (!active) {
          registered();
          return;
        }
        unsubscribe = registered;
      } catch {
        subscriptionUnavailable = true;
      }

      if (!active) return;
      const loaded = await run(() => client.getState());
      if (
        active &&
        loaded &&
        storage.getItem(ONBOARDING_KEY) === ONBOARDING_VERSION
      ) {
        await run(() => client.refreshGames());
      }
      if (active && subscriptionUnavailable) {
        setError({ code: 'state_unavailable', message: en.errors.state_unavailable });
      }
    })();

    return () => {
      active = false;
      unsubscribe?.();
    };
  }, [client, run, storage]);

  useEffect(() => {
    if (!snapshot || !hasOperationInFlight(snapshot)) return;
    const timer = window.setTimeout(() => {
      void run(() => client.getState());
    }, POLL_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [client, run, snapshot]);

  const finishOnboarding = async (enable: boolean) => {
    if (enable && !(await run(() => client.setEnabled(true)))) return;
    if (!enable && snapshot?.master_switch_checked) {
      if (!(await run(() => client.setEnabled(false)))) return;
    }
    storage.setItem(ONBOARDING_KEY, ONBOARDING_VERSION);
    setOnboardingComplete(true);
  };

  if (!snapshot) {
    return (
      <main className="loading-screen">
        <img src="/playervox-mark-dark.svg" alt="" />
        <div className="loading-line" />
        {error && <ErrorBanner message={error.message} onRetry={() => void run(() => client.getState())} />}
      </main>
    );
  }

  const actions = {
    onRefresh: () => void run(() => client.refreshGames()),
    onSelectGame: (appId: number, selected: boolean) =>
      void run(() => client.setGameSelected(appId, selected)),
    onPickManualGame: () => void run(() => client.pickManualGame()),
  };

  return (
    <>
      {!onboardingComplete ? (
        <Onboarding
          snapshot={snapshot}
          busy={busy || hasOperationInFlight(snapshot)}
          {...actions}
          onFinish={(enable) => void finishOnboarding(enable)}
        />
      ) : (
        <Dashboard
          snapshot={snapshot}
          busy={busy || hasOperationInFlight(snapshot)}
          {...actions}
          onEnable={(enabled) => void run(() => client.setEnabled(enabled))}
          onLoadLogs={() => client.getRecentLogs()}
          onRemoveManualGame={(id) => void run(() => client.removeManualGame(id))}
        />
      )}
      {error && <ErrorBanner message={error.message} onRetry={() => void run(() => client.getState())} />}
    </>
  );
}

function messageForError(reason: unknown): string {
  const code = typeof reason === 'string' ? reason : '';
  return en.errors[code as keyof typeof en.errors] ?? en.errors.generic;
}

function ErrorBanner({ message, onRetry }: { message: string; onRetry(): void }) {
  return (
    <div className="error-banner" role="alert">
      <div><strong>{en.errors.title}</strong><span>{message}</span></div>
      <button className="button button--secondary" onClick={onRetry}>{en.common.retry}</button>
    </div>
  );
}
