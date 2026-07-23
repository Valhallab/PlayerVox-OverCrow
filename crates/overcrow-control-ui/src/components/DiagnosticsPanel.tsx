import { useMemo, useRef, useState } from 'react';

import { en } from '../i18n/en';
import type { ControlLogSnapshot, ControlSnapshot } from '../lib/control';

type Diagnostics = ControlSnapshot['diagnostics'];
type LogLevel = 'INFO' | 'WARN' | 'ERROR' | 'UNKNOWN';

interface LogEntry {
  raw: string;
  level: LogLevel;
  component: string;
}

interface DiagnosticsPanelProps {
  diagnostics: Diagnostics;
  loadLogs(): Promise<ControlLogSnapshot>;
}

export function DiagnosticsPanel({
  diagnostics,
  loadLogs,
}: DiagnosticsPanelProps) {
  const [tab, setTab] = useState<'overview' | 'logs'>('overview');
  const [logs, setLogs] = useState<ControlLogSnapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [component, setComponent] = useState('all');
  const [level, setLevel] = useState<LogLevel | 'all'>('all');
  const [query, setQuery] = useState('');
  const requested = useRef(false);
  const latestLogs = useRef<ControlLogSnapshot | null>(null);

  const entries = useMemo(
    () => logs?.lines.map(parseLogLine) ?? [],
    [logs],
  );
  const components = useMemo(
    () =>
      [...new Set(entries.map((entry) => entry.component))].sort((a, b) =>
        a.localeCompare(b),
      ),
    [entries],
  );
  const visibleEntries = useMemo(() => {
    const normalizedQuery = query.trim().toLocaleLowerCase();
    return entries.filter(
      (entry) =>
        (component === 'all' || entry.component === component) &&
        (level === 'all' || entry.level === level) &&
        (!normalizedQuery ||
          entry.raw.toLocaleLowerCase().includes(normalizedQuery)),
    );
  }, [component, entries, level, query]);

  const refreshLogs = async () => {
    setLoading(true);
    setError(null);
    try {
      const snapshot = await loadLogs();
      latestLogs.current = snapshot;
      setLogs(snapshot);
    } catch {
      setError(
        latestLogs.current
          ? en.dashboard.logsRefreshFailed
          : en.dashboard.logsLoadFailed,
      );
    } finally {
      setLoading(false);
    }
  };

  const showLogs = () => {
    setTab('logs');
    if (!requested.current) {
      requested.current = true;
      void refreshLogs();
    }
  };

  const copyVisibleLogs = async () => {
    try {
      if (!navigator.clipboard) throw new Error('clipboard unavailable');
      await navigator.clipboard.writeText(
        visibleEntries.map((entry) => entry.raw).join('\n'),
      );
    } catch {
      setError(en.dashboard.logsCopyFailed);
    }
  };

  return (
    <div className="dashboard__content">
      <div className="diagnostics-tabs" role="tablist" aria-label={en.dashboard.diagnosticsTitle}>
        <button
          className={tab === 'overview' ? 'diagnostics-tab diagnostics-tab--active' : 'diagnostics-tab'}
          type="button"
          role="tab"
          aria-selected={tab === 'overview'}
          onClick={() => setTab('overview')}
        >
          {en.dashboard.diagnosticsOverview}
        </button>
        <button
          className={tab === 'logs' ? 'diagnostics-tab diagnostics-tab--active' : 'diagnostics-tab'}
          type="button"
          role="tab"
          aria-selected={tab === 'logs'}
          onClick={showLogs}
        >
          {en.dashboard.diagnosticsLogs}
        </button>
      </div>

      {tab === 'overview' ? (
        <div role="tabpanel">
          <p className="section-copy">{en.dashboard.diagnosticsBody}</p>
          <div className="diagnostics-list">
            {diagnostics.map((item) => (
              <article key={item.label}>
                <span className={`diagnostic-dot diagnostic-dot--${item.level}`} />
                <div><h3>{item.label}</h3><p>{item.detail}</p></div>
              </article>
            ))}
          </div>
        </div>
      ) : (
        <div className="log-viewer" role="tabpanel">
          <div className="log-toolbar">
            <label>
              <span>{en.dashboard.logsComponent}</span>
              <select
                value={component}
                onChange={(event) => setComponent(event.currentTarget.value)}
              >
                <option value="all">{en.dashboard.logsAll}</option>
                {components.map((name) => (
                  <option key={name} value={name}>{name}</option>
                ))}
              </select>
            </label>
            <label>
              <span>{en.dashboard.logsLevel}</span>
              <select
                value={level}
                onChange={(event) =>
                  setLevel(event.currentTarget.value as LogLevel | 'all')
                }
              >
                <option value="all">{en.dashboard.logsAll}</option>
                <option value="INFO">{en.dashboard.logsInfo}</option>
                <option value="WARN">{en.dashboard.logsWarning}</option>
                <option value="ERROR">{en.dashboard.logsError}</option>
              </select>
            </label>
            <label className="log-toolbar__search">
              <span>{en.dashboard.logsSearch}</span>
              <input
                type="search"
                value={query}
                onChange={(event) => setQuery(event.currentTarget.value)}
                placeholder={en.dashboard.logsSearch}
              />
            </label>
            <div className="log-toolbar__actions">
              <button
                className="button button--secondary"
                type="button"
                disabled={loading || visibleEntries.length === 0}
                onClick={() => void copyVisibleLogs()}
              >
                {en.dashboard.logsCopy}
              </button>
              <button
                className="button button--primary"
                type="button"
                disabled={loading}
                onClick={() => void refreshLogs()}
              >
                {en.dashboard.logsRefresh}
              </button>
            </div>
          </div>

          {error && <p className="log-viewer__error" role="alert">{error}</p>}
          {logs?.truncated && <p className="log-viewer__notice">{en.dashboard.logsTruncated}</p>}

          <div className="log-lines" aria-live="polite">
            {loading && !logs ? (
              <p className="log-viewer__empty">{en.dashboard.logsLoading}</p>
            ) : entries.length === 0 ? (
              <p className="log-viewer__empty">{en.dashboard.logsEmpty}</p>
            ) : visibleEntries.length === 0 ? (
              <p className="log-viewer__empty">{en.dashboard.logsNoMatches}</p>
            ) : (
              visibleEntries.map((entry, index) => (
                <code
                  className={`log-line log-line--${entry.level.toLocaleLowerCase()}`}
                  key={`${index}:${entry.raw}`}
                >
                  {entry.raw}
                </code>
              ))
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function parseLogLine(raw: string): LogEntry {
  const fields = raw.split(' ', 4);
  const parsedLevel = fields[1];
  const level: LogLevel =
    parsedLevel === 'INFO' || parsedLevel === 'WARN' || parsedLevel === 'ERROR'
      ? parsedLevel
      : 'UNKNOWN';
  const component = fields[2]?.match(/^[a-z0-9_-]+$/)
    ? fields[2]
    : 'other';
  return { raw, level, component };
}
