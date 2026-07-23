import { useMemo, useState } from 'react';

import { en } from '../i18n/en';
import type { ControlLogSnapshot, ControlSnapshot } from '../lib/control';
import { Brand } from './Brand';
import { CompatibilityCard } from './CompatibilityCard';
import { DiagnosticsPanel } from './DiagnosticsPanel';

type Page = 'overview' | 'games' | 'diagnostics' | 'about';

interface DashboardProps {
  snapshot: ControlSnapshot;
  busy: boolean;
  onEnable(enabled: boolean): void;
  onRefresh(): void;
  onSelectGame(appId: number, selected: boolean): void;
  onPickManualGame(): void;
  onRemoveManualGame(id: string): void;
  onLoadLogs(): Promise<ControlLogSnapshot>;
}

export function Dashboard(props: DashboardProps) {
  const { snapshot, busy } = props;
  const [page, setPage] = useState<Page>('overview');
  const [query, setQuery] = useState('');
  const selectedCount = snapshot.games.filter((game) => game.selected).length + snapshot.manual_games.length;
  const filteredGames = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    return normalized
      ? snapshot.games.filter((game) => game.name.toLocaleLowerCase().includes(normalized))
      : snapshot.games;
  }, [query, snapshot.games]);

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

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <Brand compact />
        <p className="sidebar__label">{en.dashboard.controlCenter}</p>
        <nav aria-label={en.dashboard.controlCenter}>
          <NavButton active={page === 'overview'} onClick={() => setPage('overview')} icon="overview">
            {en.dashboard.navOverview}
          </NavButton>
          <NavButton active={page === 'games'} onClick={() => setPage('games')} icon="games">
            {en.dashboard.navGames}
          </NavButton>
          <NavButton active={page === 'diagnostics'} onClick={() => setPage('diagnostics')} icon="diagnostics">
            {en.dashboard.navDiagnostics}
          </NavButton>
          <NavButton active={page === 'about'} onClick={() => setPage('about')} icon="about">
            {en.dashboard.navAbout}
          </NavButton>
        </nav>
        <div className="sidebar__compatibility">
          <span className={`status-dot status-dot--${snapshot.compatibility.activation_allowed ? 'ok' : 'blocked'}`} />
          <div>
            <strong>{en.compatibility[snapshot.compatibility.status]}</strong>
            <small>{snapshot.compatibility.desktop} · {snapshot.compatibility.session}</small>
          </div>
        </div>
      </aside>

      <main className="dashboard">
        <header className="dashboard__header">
          <div>
            <div className="eyebrow">{en.dashboard.controlCenter}</div>
            <h1>{page === 'overview' ? en.dashboard.navOverview : page === 'games' ? en.dashboard.gamesTitle : page === 'diagnostics' ? en.dashboard.diagnosticsTitle : en.dashboard.aboutTitle}</h1>
          </div>
        </header>

        {snapshot.notices.map((notice) => (
          <div className={`notice notice--${notice.level}`} key={notice.operation}>
            {notice.message}
          </div>
        ))}

        {page === 'overview' && (
          <div className="dashboard__content">
            <section className={`status-hero ${snapshot.master_switch_checked ? 'status-hero--on' : ''}`}>
              <div className="status-hero__orb"><span /></div>
              <div className="status-hero__copy">
                <div className="eyebrow">{en.dashboard.systemStatus}</div>
                <h2>{lifecycleTitle}</h2>
                <p>{snapshot.master_switch_checked ? en.dashboard.enabledBody : en.dashboard.disabledBody}</p>
              </div>
              <button
                className={`button ${snapshot.master_switch_checked ? 'button--secondary' : 'button--primary'} status-hero__action`}
                disabled={busy || !snapshot.master_switch_enabled}
                onClick={() => props.onEnable(!snapshot.master_switch_checked)}
              >
                {lifecycleAction}
              </button>
            </section>
            <div className="metrics">
              <Metric label={en.dashboard.selectedGames} value={String(selectedCount)} detail={en.dashboard.explicitlyAuthorized} />
              <Metric label={en.dashboard.shortcut} value={snapshot.shortcut} detail={en.dashboard.shortcutScope} />
              <Metric label={en.dashboard.compatibility} value={en.compatibility[snapshot.compatibility.status]} detail={`${snapshot.compatibility.desktop} · ${snapshot.compatibility.session}`} />
            </div>
            <CompatibilityCard compatibility={snapshot.compatibility} compact />
          </div>
        )}

        {page === 'games' && (
          <div className="dashboard__content">
            <div className="section-intro">
              <p>{en.dashboard.gamesBody}</p>
              <div className="toolbar">
                <input
                  type="search"
                  value={query}
                  onChange={(event) => setQuery(event.currentTarget.value)}
                  placeholder={en.dashboard.search}
                  aria-label={en.dashboard.search}
                />
                <button className="button button--secondary" onClick={props.onRefresh} disabled={busy}>{en.common.refresh}</button>
                <button className="button button--primary" onClick={props.onPickManualGame} disabled={busy || !snapshot.selection_editing_enabled}>+ {en.common.addNative}</button>
              </div>
            </div>
            <div className="game-selector game-selector--dashboard">
              {filteredGames.map((game) => (
                <label className="game-row" key={game.app_id}>
                  <span className="game-row__icon" aria-hidden="true">{game.name.slice(0, 1).toUpperCase()}</span>
                  <span className="game-row__name">{game.name}<small>{en.dashboard.steam} · {en.dashboard.app} {game.app_id}</small></span>
                  <input type="checkbox" checked={game.selected} disabled={busy || !snapshot.selection_editing_enabled} onChange={(event) => props.onSelectGame(game.app_id, event.currentTarget.checked)} />
                  <span className="switch" aria-hidden="true" />
                </label>
              ))}
              {snapshot.manual_games.map((game) => (
                <div className="game-row" key={game.id}>
                  <span className="game-row__icon" aria-hidden="true">{game.name.slice(0, 1).toUpperCase()}</span>
                  <span className="game-row__name">{game.name}<small>{game.executable}</small></span>
                  <button className="text-button" onClick={() => props.onRemoveManualGame(game.id)} disabled={busy || !snapshot.selection_editing_enabled}>{en.common.remove}</button>
                </div>
              ))}
              {filteredGames.length === 0 && snapshot.manual_games.length === 0 && <p className="empty-state">{en.onboarding.noGames}</p>}
            </div>
          </div>
        )}

        {page === 'diagnostics' && (
          <DiagnosticsPanel
            diagnostics={snapshot.diagnostics}
            loadLogs={props.onLoadLogs}
          />
        )}

        {page === 'about' && (
          <div className="dashboard__content about-card">
            <Brand />
            <p>{en.dashboard.aboutBody}</p>
            <p>{en.dashboard.license}</p>
            <code>{en.dashboard.source}</code>
          </div>
        )}
      </main>
    </div>
  );
}

function NavButton({ active, icon, children, onClick }: { active: boolean; icon: Page; children: string; onClick(): void }) {
  return <button className={active ? 'nav-button nav-button--active' : 'nav-button'} onClick={onClick}><span className="nav-button__icon" aria-hidden="true"><NavIcon name={icon} /></span>{children}</button>;
}

function NavIcon({ name }: { name: Page }) {
  if (name === 'overview') {
    return <svg viewBox="0 0 24 24"><rect x="3" y="3" width="7" height="7" rx="1.5" /><rect x="14" y="3" width="7" height="7" rx="1.5" /><rect x="3" y="14" width="7" height="7" rx="1.5" /><rect x="14" y="14" width="7" height="7" rx="1.5" /></svg>;
  }
  if (name === 'games') {
    return <svg viewBox="0 0 24 24"><path d="M8 8h8a5 5 0 0 1 4.7 3.3l.8 2.4a3 3 0 0 1-4.8 3.2L15 15.5H9l-1.7 1.4a3 3 0 0 1-4.8-3.2l.8-2.4A5 5 0 0 1 8 8Z" /><path d="M7 12h4M9 10v4" /><circle cx="16.5" cy="11.5" r=".8" /><circle cx="18.5" cy="13.5" r=".8" /></svg>;
  }
  if (name === 'diagnostics') {
    return <svg viewBox="0 0 24 24"><path d="M3 12h4l2.3-6 4.2 12 2.3-6H21" /></svg>;
  }
  return <svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="9" /><path d="M12 11v6M12 7.5v.5" /></svg>;
}

function Metric({ label, value, detail }: { label: string; value: string; detail: string }) {
  return <article className="metric"><span>{label}</span><strong>{value}</strong><small>{detail}</small></article>;
}
