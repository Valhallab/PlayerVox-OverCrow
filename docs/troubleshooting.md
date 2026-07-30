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

### GNOME Wayland

GNOME 46–50 support requires the packaged OverCrow Shell extension. Check its
normalized state with `./scripts/diagnose.sh`. If the package is installed but
the extension is inactive, open the Control Center and run setup again, or use
this recovery command as the logged-in desktop user:

```sh
gnome-extensions enable overcrow@playervox.com
```

Then verify:

```sh
gnome-extensions info overcrow@playervox.com
```

The reported path must be
`/usr/share/gnome-shell/extensions/overcrow@playervox.com`. A user extension
with the same UUID is rejected because it would shadow the reviewed system
package. Log out and back in if GNOME has not discovered a newly installed
system extension in the current session.

## A Windows game added to Steam is missing

1. In Steam, add the Windows game as a non-Steam game.
2. Open its Steam properties and force a Proton compatibility tool.
3. Launch the game once through Steam so Steam assigns its runtime app ID.
4. Open **Games** in the Control Center and select **Refresh games**.
5. Select the row marked **Steam shortcut**.

Do not use **Add a native game** for a `.exe`; that picker deliberately accepts
native Linux executables only. **Type unverified** means Steam did not provide a
known application type. It is a warning, not a compatibility failure.

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

## Twitch chat does not connect

1. Confirm that the game is selected and active, and that the Twitch chat
   widget itself is enabled.
2. Open the widget options, choose a valid public channel, and select
   **Connect Twitch** (first time) or **Reconnect chat** (when already signed
   in). **Disconnect chat** closes chat and keeps your session;
   **Sign out of Twitch** revokes the account and requires a new device code.
   Upgrading from an IRC-based development build requires one new
   authorization for `user:read:chat` and `user:write:chat`.
3. If the widget says that Twitch is not configured, the installed build has
   an empty Client ID (a non-default or custom build). Official PlayerVox
   builds include the public Client ID. No account request was made.
4. If secure credential storage is unavailable, the current connection may
   still work but must be authorized again after restarting OverCrow.
5. Disable and re-enable the widget to clear transient chat state. Do not put
   access tokens, chat content, channel names, or provider responses in a bug
   report.

The chat may be joined while the channel is offline. OverCrow does not persist
chat messages and does not connect when no authorized game is active. It
renders usernames in semibold and messages in regular text. Native Twitch
emotes are fetched as bounded static PNGs from Twitch's CDN; a failed,
unsupported, or malformed emote remains visible as its text (for example,
`LUL`). Badge images and third-party emote providers are not supported.

## Report a problem

Open **Diagnostics → Report a problem** in the Control Center:

1. Describe the issue and choose whether to include sanitized logs.
2. Select **Prepare report**.
3. Review the exact in-memory preview.
4. Select **Send report** to submit it privately to PlayerVox support, or use
   **Copy report** to keep the text yourself.

The automatically collected fields contain only bounded installation state
and structured diagnostics. They exclude game names, manual executable paths,
notes, media metadata, provider payloads, and other stored user content. Your
own problem description is included exactly as shown in the preview. Nothing
is uploaded until you select **Send report**. The request is sent only to the
fixed PlayerVox support endpoint over HTTPS; OverCrow follows no redirects,
sends no account credentials, and stores no local report file. A successful
submission displays a reference you can retain for follow-up.

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
