# Troubleshooting

Start with the read-only status commands:

```sh
overcrowctl status
overcrowctl logs
./scripts/diagnose.sh
```

The diagnostic reports session type, supported compositor integration, D-Bus
ownership, active processes, and detected overlay windows. It bounds external
commands and does not modify the desktop.

## The overlay does not appear

1. Open `overcrow-control` and confirm that the master switch is enabled.
2. Confirm that the exact game is selected.
3. Use windowed or borderless fullscreen, not exclusive fullscreen.
4. Focus the game window, then press `Meta+Alt+O`.
5. Check `overcrowctl status` for an `active_game`.
6. Run `./scripts/diagnose.sh` and compare the detected compositor with the
   [compatibility table](../README.md#compatibility).

An unfocused, unselected, unsupported, or ambiguously identified game fails
closed and cannot authorize an overlay.

## Input or overlay mode is stuck

First press `Esc`, then `Meta+Alt+O`. If the mode still does not return to
Passive, use:

```sh
overcrowctl passive
```

If compositor state remains inconsistent, disable and re-enable OverCrow from
the Control Center. This runs the managed cleanup path and preserves selected
games and widget settings. Avoid killing individual processes unless logs are
being collected for a bug report.

## Inspecting services and logs

```sh
systemctl --user status overcrow-core.service
systemctl --user status overcrow-overlay.service overcrow-hyprland.service
journalctl --user -u overcrow-core.service -u overcrow-overlay.service
tail -F "${XDG_STATE_HOME:-$HOME/.local/state}/overcrow/logs/"*.log
```

The local files are private, rotate at 2 MiB, retain three archives per
component, and are never uploaded automatically. `overcrowctl logs` returns at
most the newest 2,000 merged lines.

When reporting a bug, include the display environment and versions, the exact
game presentation mode, `overcrowctl status`, the relevant bounded log window,
and minimal reproduction steps. Review output before sharing it even though
OverCrow intentionally excludes user content and raw provider data.

Report unresolved vulnerabilities privately through [SECURITY.md](../SECURITY.md).

## Clean uninstall

Disable OverCrow in the Control Center, then remove the package:

```sh
sudo pacman -R overcrow-bin
```

Pacman removes managed program and integration files. User preferences remain
in `${XDG_CONFIG_HOME:-$HOME/.config}/overcrow/`; remove that directory only if
you intentionally want to discard the configuration.

## Live acceptance

Automated tests cannot reproduce a real compositor, Proton game, pointer lock,
or workspace transition. Maintainers validating a release should follow the
[manual MVP checklist](testing/manual-mvp.md) on each claimed display backend.
