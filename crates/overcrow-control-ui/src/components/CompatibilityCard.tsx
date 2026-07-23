import { en } from '../i18n/en';
import type { ControlSnapshot } from '../lib/control';

export function CompatibilityCard({
  compatibility,
  compact = false,
}: {
  compatibility: ControlSnapshot['compatibility'];
  compact?: boolean;
}) {
  const tone = compatibility.activation_allowed ? 'available' : 'blocked';
  return (
    <section className={`compatibility-card compatibility-card--${tone} ${compact ? 'compatibility-card--compact' : ''}`}>
      <div className="compatibility-card__status">
        <span className="status-dot" aria-hidden="true" />
        {en.compatibility[compatibility.status]}
      </div>
      <h3>{en.compatibility.detected}</h3>
      <p className="compatibility-card__environment">
        {compatibility.operating_system} · {en.compatibility.desktops[compatibility.desktop]} ·{' '}
        {en.compatibility.sessions[compatibility.session]}
      </p>
      <p>{en.compatibility[compatibility.reason]}</p>
    </section>
  );
}
