import { useState } from 'react';

import { en } from '../i18n/en';
import type { ControlSnapshot } from '../lib/control';
import { Brand } from './Brand';
import { CompatibilityCard } from './CompatibilityCard';

type Step = 'welcome' | 'compatibility' | 'games' | 'ready';

interface OnboardingProps {
  snapshot: ControlSnapshot;
  busy: boolean;
  onRefresh(): void;
  onSelectGame(appId: number, selected: boolean): void;
  onPickManualGame(): void;
  onFinish(enable: boolean): void;
}

export function Onboarding({
  snapshot,
  busy,
  onRefresh,
  onSelectGame,
  onPickManualGame,
  onFinish,
}: OnboardingProps) {
  const [step, setStep] = useState<Step>('welcome');
  const selectedCount =
    snapshot.games.filter((game) => game.selected).length + snapshot.manual_games.length;

  const begin = () => {
    onRefresh();
    setStep('compatibility');
  };

  return (
    <main className="onboarding">
      <div className="onboarding__ambient" aria-hidden="true" />
      <header className="onboarding__header">
        <Brand />
        <span className="step-indicator">
          {step === 'welcome' ? '01' : step === 'compatibility' ? '02' : step === 'games' ? '03' : '04'} / 04
        </span>
      </header>

      {step === 'welcome' && (
        <section className="onboarding__hero">
          <div className="eyebrow">{en.onboarding.eyebrow}</div>
          <h1>{en.onboarding.title}</h1>
          <p className="lead">{en.onboarding.intro}</p>
          <div className="principles">
            <article>
              <span className="principle-icon">01</span>
              <div>
                <h2>{en.onboarding.safetyTitle}</h2>
                <p>{en.onboarding.safetyBody}</p>
              </div>
            </article>
            <article>
              <span className="principle-icon">02</span>
              <div>
                <h2>{en.onboarding.privacyTitle}</h2>
                <p>{en.onboarding.privacyBody}</p>
              </div>
            </article>
          </div>
          <button className="button button--primary button--large" onClick={begin}>
            {en.onboarding.start}<span aria-hidden="true">→</span>
          </button>
        </section>
      )}

      {step === 'compatibility' && (
        <section className="onboarding__panel">
          <div className="eyebrow">{en.onboarding.compatibilityEyebrow}</div>
          <h1>{en.onboarding.compatibilityTitle}</h1>
          <p className="lead">{en.onboarding.compatibilityIntro}</p>
          <CompatibilityCard compatibility={snapshot.compatibility} />
          {!snapshot.compatibility.activation_allowed && (
            <p className="blocked-message">{en.onboarding.blocked}</p>
          )}
          <div className="button-row">
            <button className="button button--ghost" onClick={() => setStep('welcome')}>
              {en.common.back}
            </button>
            <button className="button button--secondary" onClick={onRefresh} disabled={busy}>
              {en.common.retry}
            </button>
            <button
              className="button button--primary"
              onClick={() => setStep('games')}
              disabled={!snapshot.compatibility.activation_allowed || busy}
            >
              {en.common.continue}<span aria-hidden="true">→</span>
            </button>
          </div>
        </section>
      )}

      {step === 'games' && (
        <section className="onboarding__panel">
          <div className="eyebrow">{en.onboarding.gamesEyebrow}</div>
          <h1>{en.onboarding.gamesTitle}</h1>
          <p className="lead">{en.onboarding.gamesIntro}</p>
          <GameSelector
            snapshot={snapshot}
            busy={busy}
            onSelectGame={onSelectGame}
            onPickManualGame={onPickManualGame}
          />
          <div className="selection-summary">
            <strong>{selectedCount}</strong> {en.onboarding.selected}
          </div>
          <div className="button-row">
            <button className="button button--ghost" onClick={() => setStep('compatibility')}>
              {en.common.back}
            </button>
            <button className="button button--secondary" onClick={onRefresh} disabled={busy}>
              {en.common.refresh}
            </button>
            <button
              className="button button--primary"
              onClick={() => setStep('ready')}
              disabled={selectedCount === 0 || busy}
            >
              {en.common.continue}<span aria-hidden="true">→</span>
            </button>
          </div>
        </section>
      )}

      {step === 'ready' && (
        <section className="onboarding__panel onboarding__panel--ready">
          <div className="ready-mark" aria-hidden="true">✓</div>
          <div className="eyebrow">{en.onboarding.readyEyebrow}</div>
          <h1>{en.onboarding.readyTitle}</h1>
          <p className="lead">{en.onboarding.readyBody}</p>
          <div className="ready-summary">
            <span><strong>{selectedCount}</strong> {en.dashboard.selectedGames.toLowerCase()}</span>
            <span><strong>{snapshot.shortcut}</strong> {en.dashboard.shortcut.toLowerCase()}</span>
          </div>
          <div className="button-row button-row--center">
            <button className="button button--ghost" onClick={() => setStep('games')}>
              {en.common.back}
            </button>
            <button className="button button--secondary" onClick={() => onFinish(false)} disabled={busy}>
              {en.onboarding.finishOff}
            </button>
            <button
              className="button button--primary"
              onClick={() => onFinish(true)}
              disabled={!snapshot.master_switch_enabled || busy}
            >
              {en.onboarding.enableFinish}
            </button>
          </div>
        </section>
      )}
    </main>
  );
}

export function GameSelector({
  snapshot,
  busy,
  onSelectGame,
  onPickManualGame,
}: {
  snapshot: ControlSnapshot;
  busy: boolean;
  onSelectGame(appId: number, selected: boolean): void;
  onPickManualGame(): void;
}) {
  return (
    <div className="game-selector">
      {snapshot.games.length === 0 && snapshot.manual_games.length === 0 && (
        <p className="empty-state">{en.onboarding.noGames}</p>
      )}
      {snapshot.games.map((game) => (
        <label className="game-row" key={game.app_id}>
          <span className="game-row__icon" aria-hidden="true">{game.name.slice(0, 1).toUpperCase()}</span>
          <span className="game-row__name">{game.name}</span>
          <input
            type="checkbox"
            checked={game.selected}
            disabled={busy || !snapshot.selection_editing_enabled}
            onChange={(event) => onSelectGame(game.app_id, event.currentTarget.checked)}
          />
          <span className="switch" aria-hidden="true" />
        </label>
      ))}
      {snapshot.manual_games.map((game) => (
        <div className="game-row" key={game.id}>
          <span className="game-row__icon" aria-hidden="true">{game.name.slice(0, 1).toUpperCase()}</span>
          <span className="game-row__name">
            {game.name}<small>{en.common.nativeLinuxGame}</small>
          </span>
          <span className="tag">{en.common.allowed}</span>
        </div>
      ))}
      <button className="add-game" onClick={onPickManualGame} disabled={busy || !snapshot.selection_editing_enabled}>
        <span aria-hidden="true">+</span>{en.common.addNative}
      </button>
    </div>
  );
}
