import { en } from '../i18n/en';

export function Brand({ compact = false }: { compact?: boolean }) {
  return (
    <div className={compact ? 'brand brand--compact' : 'brand'}>
      <span className="brand__mark" aria-hidden="true">
        <img src="/playervox-mark-dark.svg" alt="" />
      </span>
      <span className="brand__wordmark" aria-label={en.brand.fullName}>
        <strong>
          <span>{en.brand.player}</span>
          <span className="brand__accent">{en.brand.vox}</span>
        </strong>
        <small>{en.brand.product}</small>
      </span>
    </div>
  );
}
