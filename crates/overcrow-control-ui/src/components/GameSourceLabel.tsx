import { en } from '../i18n/en';
import type { ControlSnapshot } from '../lib/control';

type ControlGame = ControlSnapshot['games'][number];

export function GameSourceLabel({ game }: { game: ControlGame }) {
  const source =
    game.kind === 'steam_shortcut'
      ? en.dashboard.steamShortcut
      : game.kind === 'unverified'
        ? en.dashboard.typeUnverified
        : en.dashboard.steam;

  return <small>{source} · {en.dashboard.app} {game.app_id}</small>;
}
