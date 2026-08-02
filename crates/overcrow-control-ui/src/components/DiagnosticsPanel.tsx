import { useEffect, useMemo, useRef, useState } from 'react';
import { X } from 'lucide-react';

import { en } from '../i18n/en';
import type {
  ControlLogSnapshot,
  ControlSnapshot,
  ControlSupportReceipt,
  ControlSupportReport,
} from '../lib/control';

type Diagnostics = ControlSnapshot['diagnostics'];
type LogLevel = 'INFO' | 'WARN' | 'ERROR' | 'UNKNOWN';
const MAX_DESCRIPTION_BYTES = 2_000;

interface LogEntry {
  raw: string;
  level: LogLevel;
  component: string;
}

interface DiagnosticsPanelProps {
  diagnostics: Diagnostics;
  loadLogs(): Promise<ControlLogSnapshot>;
  prepareSupportReport(
    description: string,
    includeLogs: boolean,
  ): Promise<ControlSupportReport>;
  submitSupportReport(reportId: string): Promise<ControlSupportReceipt>;
}

export function DiagnosticsPanel({
  diagnostics,
  loadLogs,
  prepareSupportReport,
  submitSupportReport,
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
  const [reportOpen, setReportOpen] = useState(false);
  const [description, setDescription] = useState('');
  const [includeLogs, setIncludeLogs] = useState(true);
  const [report, setReport] = useState<ControlSupportReport | null>(null);
  const [receipt, setReceipt] = useState<ControlSupportReceipt | null>(null);
  const [reportBusy, setReportBusy] = useState(false);
  const [reportError, setReportError] = useState<string | null>(null);
  const descriptionBytes = useMemo(
    () => new TextEncoder().encode(description).length,
    [description],
  );
  const descriptionValid =
    description.trim().length > 0 &&
    descriptionBytes <= MAX_DESCRIPTION_BYTES;

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

  const openReport = () => {
    setDescription('');
    setIncludeLogs(true);
    setReport(null);
    setReceipt(null);
    setReportError(null);
    setReportOpen(true);
  };

  const closeReport = () => {
    if (!reportBusy) setReportOpen(false);
  };

  const invalidateReport = () => {
    setReport(null);
    setReceipt(null);
    setReportError(null);
  };

  const createReport = async () => {
    if (!descriptionValid || reportBusy) return;
    setReportBusy(true);
    setReportError(null);
    try {
      setReport(await prepareSupportReport(description, includeLogs));
    } catch {
      setReportError(en.dashboard.reportPrepareFailed);
    } finally {
      setReportBusy(false);
    }
  };

  const copyReport = async () => {
    if (!report) return;
    try {
      if (!navigator.clipboard) throw new Error('clipboard unavailable');
      await navigator.clipboard.writeText(report.content);
    } catch {
      setReportError(en.dashboard.reportCopyFailed);
    }
  };

  const sendReport = async () => {
    if (!report || reportBusy) return;
    setReportBusy(true);
    setReportError(null);
    try {
      setReceipt(await submitSupportReport(report.report_id));
    } catch (reason) {
      setReportError(submissionErrorMessage(reason));
    } finally {
      setReportBusy(false);
    }
  };

  useEffect(() => {
    if (!reportOpen) return undefined;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !reportBusy) setReportOpen(false);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [reportBusy, reportOpen]);

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
          <article className="support-card">
            <div>
              <h3>{en.dashboard.reportProblem}</h3>
              <p>{en.dashboard.reportBody}</p>
            </div>
            <button
              className="button button--secondary"
              type="button"
              onClick={openReport}
            >
              {en.dashboard.reportProblem}
            </button>
          </article>
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

      {reportOpen && (
        <div className="report-dialog-backdrop">
          <section
            className="report-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="report-dialog-title"
          >
            <header className="report-dialog__header">
              <div>
                <p className="eyebrow">{en.dashboard.diagnosticsTitle}</p>
                <h2 id="report-dialog-title">{en.dashboard.reportProblem}</h2>
              </div>
              <button
                className="report-dialog__close"
                type="button"
                aria-label={en.common.close}
                disabled={reportBusy}
                onClick={closeReport}
              >
                <X aria-hidden="true" />
              </button>
            </header>

            <p className="report-dialog__intro">{en.dashboard.reportPrivacy}</p>

            <label className="report-field">
              <span>{en.dashboard.reportDescription}</span>
              <textarea
                value={description}
                rows={5}
                autoFocus
                disabled={reportBusy}
                onChange={(event) => {
                  setDescription(event.currentTarget.value);
                  invalidateReport();
                }}
              />
            </label>
            <p
              className={
                descriptionBytes > MAX_DESCRIPTION_BYTES
                  ? 'report-byte-count report-byte-count--invalid'
                  : 'report-byte-count'
              }
            >
              {descriptionBytes.toLocaleString('en-US')} /{' '}
              {MAX_DESCRIPTION_BYTES.toLocaleString('en-US')} bytes
            </p>

            <label className="report-checkbox">
              <input
                type="checkbox"
                aria-label={en.dashboard.reportIncludeLogs}
                checked={includeLogs}
                disabled={reportBusy}
                onChange={(event) => {
                  setIncludeLogs(event.currentTarget.checked);
                  invalidateReport();
                }}
              />
              <span>
                <strong>{en.dashboard.reportIncludeLogs}</strong>
                <small>{en.dashboard.reportIncludeLogsBody}</small>
              </span>
            </label>

            {reportError && (
              <p className="report-dialog__error" role="alert">
                {reportError}
              </p>
            )}

            {!report && (
              <div className="report-dialog__actions">
                <button
                  className="button button--ghost"
                  type="button"
                  disabled={reportBusy}
                  onClick={closeReport}
                >
                  {en.common.close}
                </button>
                <button
                  className="button button--primary"
                  type="button"
                  disabled={!descriptionValid || reportBusy}
                  onClick={() => void createReport()}
                >
                  {reportBusy
                    ? en.dashboard.reportPreparing
                    : en.dashboard.reportPrepare}
                </button>
              </div>
            )}

            {report && (
              <div className="report-preview">
                <div className="report-preview__heading">
                  <div>
                    <h3>{en.dashboard.reportPreview}</h3>
                    <p>{en.dashboard.reportReady}</p>
                  </div>
                  <button
                    className="button button--secondary"
                    type="button"
                    onClick={() => void copyReport()}
                  >
                    {en.dashboard.reportCopy}
                  </button>
                </div>
                <pre>{report.content}</pre>
                <div className="report-dialog__actions">
                  <button
                    className="button button--primary"
                    type="button"
                    disabled={reportBusy || receipt !== null}
                    onClick={() => void sendReport()}
                  >
                    {reportBusy
                      ? en.dashboard.reportSending
                      : en.dashboard.reportSend}
                  </button>
                </div>
                {receipt && (
                  <p className="report-preview__success" role="status">
                    {en.dashboard.reportSent}{' '}
                    <strong>{receipt.reference}</strong>
                  </p>
                )}
              </div>
            )}
          </section>
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

function submissionErrorMessage(reason: unknown): string {
  if (reason === 'support_rate_limited') {
    return en.dashboard.reportRateLimited;
  }
  if (reason === 'support_submission_timeout') {
    return en.dashboard.reportTimeout;
  }
  if (reason === 'support_report_conflict') {
    return en.dashboard.reportConflict;
  }
  if (
    reason === 'support_submission_rejected' ||
    reason === 'support_response_invalid'
  ) {
    return en.dashboard.reportServiceFailed;
  }
  return en.dashboard.reportNetworkFailed;
}
