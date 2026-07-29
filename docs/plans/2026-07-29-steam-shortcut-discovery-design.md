# Steam shortcut discovery design

## Goal

Allow users to select Windows games added to Steam and launched through Proton
without selecting a `.exe` manually. Keep Steam tools and runtimes out of the
game list, preserve legitimate games when Steam metadata is incomplete, and
sort the resulting list alphabetically.

## Discovery model

The existing manifest discovery remains authoritative for installed official
Steam applications. Discovery additionally reads:

- `appcache/appinfo.vdf` to classify official applications;
- each bounded `userdata/<account>/config/shortcuts.vdf` to discover non-Steam
  shortcuts.

Binary VDF parsing uses a small iterative reader local to the Control Center.
It supports only the primitive types needed to skip unrelated values and
extract `appid`, `appname`, and `common/type`; it does not construct the full
Steam cache tree. Reads, profiles, entries, nesting, names, and retained
warnings are bounded. Files must be regular canonical files contained beneath
a canonical Steam root. Shortcut executable paths, launch options, icons,
tags, and user identifiers are neither retained nor exposed.

Each discovered entry has one presentation kind:

- `steam_game`: an official application positively classified as a game or
  demo;
- `steam_shortcut`: a non-Steam shortcut with Steam's generated 32-bit app ID;
- `unverified`: an installed manifest whose application type is unavailable or
  unknown.

Types positively identified as tools, runtimes, configuration packages, DLC,
media, or other non-game content are excluded. Missing or unreadable app-info
metadata does not hide installed manifests: they remain visible as
`unverified`, with a bounded diagnostic warning.

Conflicting records for the same app ID fail closed and are omitted with one
bounded warning. Duplicate identical records are coalesced. The displayed
entries are sorted by Unicode-case-folded name and then app ID for stable
ordering.

## Identity and persistence

The existing `selected_steam_app_ids: BTreeSet<u32>` remains unchanged. Steam
already exports the generated shortcut ID as `SteamAppId` to the Proton process
tree, so Core can match shortcuts through its existing process-environment
identity path. No executable-path matching or Wine process heuristics are
introduced.

The Control Center only permits selecting IDs present in the filtered discovery
result. Positively classified non-game IDs cannot be newly selected. Existing
settings and stable IDs require no migration.

## Control Center

The game row metadata distinguishes:

- ordinary official Steam games;
- `Steam shortcut`;
- `Type unverified`.

The last label is a warning, not an error. The direct executable picker remains
native-Linux-only; selecting a `.exe` returns an actionable message explaining
that the game must be added to Steam, configured to use Proton, launched once,
and rediscovered.

Adding the presentation kind changes the serialized Control Center snapshot, so
its schema version is incremented atomically with backend, frontend, tray, and
fixture updates.

## Failure behavior

One malformed cache or shortcut file must not discard valid results from other
Steam roots or accounts. The affected source is skipped and a bounded warning
is exposed through existing diagnostics. Exhausted limits stop only the
affected scan. Parsing never runs on the UI thread.

When type metadata is missing, OverCrow prefers an explicit warning over a name
heuristic. It does not maintain a brittle static blacklist of App IDs or infer
application type from names.

## Verification

Focused tests cover:

- official game/demo inclusion and known non-game exclusion;
- unknown or missing type fallback;
- non-Steam shortcut parsing, including a high-bit app ID;
- malformed, oversized, deeply nested, conflicting, and escaping inputs;
- bounded warnings and partial-source recovery;
- deterministic case-insensitive ordering;
- snapshot schema and UI labels;
- the actionable `.exe` picker error.

Final validation uses formatting, Clippy with warnings denied, the locked
workspace test suite, and the standard diff/status checks. Live acceptance is a
short Proton shortcut test on the user's real Steam session.
