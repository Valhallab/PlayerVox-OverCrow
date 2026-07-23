# OverCrow MVP manual acceptance

This checklist validates the MVP in a real desktop session. It is intentionally
manual: the repository smoke tests do not start X11, Plasma, KWin, a game, or a
user D-Bus session.

No live compositor acceptance was performed as part of the Task 8 source
stabilization audit. Hyprland, Plasma Wayland, and X11 remain manual acceptance
work until dated evidence is recorded below this checklist.

## Prerequisites and evidence

- Run every command as the logged-in desktop user, without `sudo`.
- Install GNU `timeout` from coreutils; the diagnostic relies on its
  `--signal` and `--kill-after` options and does not probe `timeout --help` at
  runtime.
- Build the native package with `./scripts/build-arch-package.sh`, then install
  the resulting `dist/overcrow-bin-*.pkg.tar.zst` explicitly with Pacman. The
  build command itself does not install or start anything. Do not manually
  start renderer/bridge units for the opt-in lifecycle checks.
- Use a Steam/Proton game whose publisher permits an external overlay. Configure
  it as **windowed** or **borderless fullscreen**. Record the title and expected
  Steam App ID.
- For the stabilized Warframe flow, select Warframe in Control Center and
  verify that `overcrowctl status` reports Steam App ID `230410` before judging
  any Warframe widget or provider behavior.
- Prepare a safe, visible control in the game or another test window so a click
  received by the underlying window can be distinguished from a click consumed
  by the overlay.
- Use separate logins for the X11, Plasma Wayland, and Hyprland sections. Do
  not change `XDG_SESSION_TYPE` manually to simulate a compositor.
- Before changing service or KWin state, record the values needed for
  restoration. In Plasma, keep the following variables in the same shell until
  the KWin lease test is restored. The sentinel distinguishes a missing key
  from an existing empty/false value:

  ```sh
  systemctl --user is-active overcrow-core.service
  OVERCROW_KWIN_KEY_ABSENT=__OVERCROW_KWIN_KEY_ABSENT__
  overcrow_kwin_enabled_before=$(
      kreadconfig6 --file kwinrc --group Plugins \
          --key io.github.overcrow.kwinEnabled \
          --default "$OVERCROW_KWIN_KEY_ABSENT"
  )
  printf 'Initial KWin key: %s\n' "$overcrow_kwin_enabled_before"
  ```

At the beginning of each section, record the following read-only evidence. The
diagnostic prints only selected session fields and does not dump the environment.
It runs each external probe through GNU
`timeout --signal=TERM --kill-after=1s 3s`: TERM is sent after three seconds and
KILL after one additional second if required. A missing `timeout` skips all
external probes instead of retrying them directly. The process lines query the
current UID through the same bound, then restrict anchored argv0 matches to that
UID. Exit 124 and KILL-derived exit 137 both produce `timed out`; other failures
remain `unavailable`. Expect distinct `timed out`, `unavailable`, and `skipped:
timeout unavailable` states rather than treating them as negative results:

```sh
date -Is
./scripts/diagnose.sh
overcrowctl status
systemctl --user status overcrow-core.service --no-pager
journalctl --user -u overcrow-core.service --since "10 minutes ago" --no-pager
```

Keep the command outputs and screenshots requested below. A healthy idle
snapshot contains `"active_game":null` and
`"overlay_mode":"Passive"`. During a detected game, `active_game` is an
object with the expected PID/title/geometry and `backend`, and the overlay mode
must still begin as `Passive`.

## Opt-in lifecycle acceptance (run before each compositor section)

1. **Verify inert installation.** Immediately after installing the native
   package, and before opening the Control Center, run:

   ```sh
   pgrep -af '^(.*/)?overcrow-(core|overlay|hyprland)( |$)' || true
   systemctl --user is-enabled overcrow-core.service || true
   systemctl --user is-active overcrow-core.service || true
   ./scripts/diagnose.sh
   ```

   Expected evidence: no OverCrow process started, no unit was enabled, no
   permanent compositor shortcut was added, and lifecycle settings are missing
   or report the master state as disabled. Installation alone must not create a
   Hyprland managed block or install/enable a KWin package.

2. **Verify first Control Center launch.** Start `overcrow-control` from the
   `PlayerVox OverCrow` application launcher. Complete the English
   onboarding through the compatibility check and game-selection screen.
   Confirm that no game is preselected, unsupported environments cannot
   continue, and the master switch remains off unless activation is explicitly
   chosen on the final screen. Close the window and confirm that the tray icon
   remains. Use **Open Control Center**, then launch `overcrow-control` again;
   both actions must reveal the existing window without creating another tray
   icon or repeating onboarding.

3. **Verify explicit authorization.** Select only the test game, then enable
   the master switch. Confirm that the private settings file is mode `0600`,
   Core becomes active as the selected-process watcher, and only the reviewed
   integration for the current compositor is installed. The renderer and
   Wayland bridge must still be absent while the selected game is not running.
   Repeat Start and Stop from the tray and confirm that its non-clickable status
   line and the Control Center stay synchronized.

4. **Verify selected-game autostart.** Launch the selected game normally from
   Steam. Confirm that Core starts the renderer and the compositor bridge (when
   applicable) without another Control Center action. Launch an unselected
   game separately and confirm it never authorizes an overlay.

5. **Verify dynamic shortcut ownership.** Focus the selected game and confirm
   `Meta+Alt+O` is available. Return to Passive, focus VS Code or another normal
   application, and confirm that application receives the same accelerator and
   OverCrow remains Passive. Return to the selected game and confirm the
   shortcut becomes available again. If the desktop portal refuses the
   shortcut, record `Shortcut availability` from `./scripts/diagnose.sh`; no
   permanent fallback binding may appear.

6. **Verify game-exit cleanup.** Exit the selected game while Passive, then
   repeat while Interactive. In both cases the overlay becomes Passive, the
   shortcut is released, and renderer/bridge units stop. Core may remain as the
   enabled lifecycle watcher.

7. **Verify global disable and persistence.** Turn the master switch off.
   Confirm settings persist `enabled: false` before Core stops, every runtime
   process exits, and the shortcut is absent. Log out and back in, launch the
   selected game, and confirm OverCrow remains completely inert until the user
   explicitly enables it again.

8. **Verify explicit application exit.** Start OverCrow, then choose **Quit**
   from the tray. Confirm that the tray icon, Core, renderer, compositor bridge,
   and shortcut all disappear. Closing the Control Center window alone must
   never produce this full-exit behavior.

## X11 acceptance

1. **Confirm the X11 baseline.** Run `./scripts/diagnose.sh` before starting the
   game. Expected evidence: `XDG_SESSION_TYPE: x11`, the core service is active,
   the D-Bus name has an owner, the diagnostic classifies both
   `_NET_SUPPORTING_WM_CHECK` and `_NET_ACTIVE_WINDOW` through bounded `xprop`
   probes, and the renderer process is reported after launching OverCrow.

2. **Verify process and window classification.** Start the Steam/Proton game in
   windowed or borderless mode, launch OverCrow, focus the game, wait at least
   two seconds, then run:

   ```sh
   xprop -root _NET_ACTIVE_WINDOW
   # Substitute the reported window ID below.
   xprop -id 0xWINDOW_ID _NET_WM_PID WM_CLASS _NET_WM_NAME
   overcrowctl status
   ```

   Expected evidence: `active_game` is non-null; its PID agrees with
   `_NET_WM_PID`; its title/class identifies the intended game; `backend` is
   `"x11"`; and `steam_app_id` agrees with the known App ID when Steam metadata
   is present. Focusing an unrelated non-game window must return
   `active_game` to `null` and force `Passive`; refocusing the game must detect
   it again.

3. **Verify transparency in both supported presentation modes.** Check once in
   ordinary windowed mode and once in borderless fullscreen. Expected visual
   evidence: the stopwatch panel is visible near the game's top-left corner,
   the rest of the overlay is transparent, the game is not replaced by a black
   window, and the overlay follows the reported game position and size. Capture
   one screenshot per mode. The X11 source currently reports a fixed scale of
   `1.0`; this checklist has not validated live X11 HiDPI or mixed-DPI
   placement, so a non-HiDPI result must not be treated as that validation.

4. **Verify passive click-through.** Run:

   ```sh
   overcrowctl passive
   overcrowctl status
   ```

   Expected evidence: the JSON reports `"overlay_mode":"Passive"`. Clicking a
   known underlying target, including beneath the visible stopwatch panel,
   reaches the game/test window rather than the overlay.

5. **Verify the CLI toggle.** With the game focused, run:

   ```sh
   overcrowctl toggle
   overcrowctl status
   ```

   Expected evidence: the mode becomes `Interactive`, and the overlay consumes
   input instead of passing it to the underlying target. Run `overcrowctl
   toggle` again and confirm the mode and click-through both return to
   `Passive`.

6. **Verify the Escape fail-safe.** Enter interactive mode with `overcrowctl
   interactive`, click the overlay so it has keyboard focus, and press Escape.
   Expected evidence from `overcrowctl status`: the mode promptly becomes
   `Passive`, and clicks reach the underlying target again. If Escape cannot be
   delivered because another window owns keyboard focus, first focus the
   interactive overlay; that is a test setup issue, not a passing result.

7. **Verify game exit fails closed.** Enter interactive mode, then quit the game
   normally. Within the next process/window polling cycle, run `overcrowctl
   status`. Expected evidence: `active_game` is `null`, the mode is `Passive`,
   the stopwatch panel disappears, and no invisible window traps mouse input.

   Repeat while the OverCrow window owns focus. Before closing the game, use
   window-manager tooling that does not refocus it to move or resize the game
   and confirm that `overcrowctl status` reports the fresh geometry. Then
   minimize or close the underlying game without first focusing it. Expected
   evidence: the daemon clears `active_game` and forces `Passive` instead of
   retaining the last X11 window. This focused-overlay sequence requires a real
   X server; the fake-backend regression tests do not validate EWMH delivery,
   real `BadWindow` replies, or compositor map-state behavior.

8. **Verify daemon restart and renderer reconnection.** Start and focus the game
   again, ensure the overlay is passive, then run:

   ```sh
   systemctl --user restart overcrow-core.service
   systemctl --user is-active overcrow-core.service
   ./scripts/diagnose.sh
   overcrowctl status
   ```

   Expected evidence: the unit returns `active`, the D-Bus name is owned again,
   the existing renderer reconnects without being relaunched, the initial mode
   after reconnection is `Passive`, and the focused game is detected again.
   Record the service journal if reconnection takes longer than a few polling
   cycles.

## Plasma Wayland acceptance

1. **Confirm the Plasma Wayland baseline.** In a real Plasma Wayland login, run
   `./scripts/diagnose.sh`. Expected evidence: `XDG_SESSION_TYPE: wayland`, the
   desktop identifies Plasma/KDE, the D-Bus name has an owner, the KWin package
   is installed and enabled, and `wayland-info` reports at least
   `wl_compositor`, `xdg_wm_base`, and the available Plasma/KWin globals. An
   XWayland `DISPLAY` may also make the X11 probe succeed; that does not replace
   the Wayland checks.

2. **Verify bridge reporting.** Start the Steam/Proton game in windowed mode,
   launch OverCrow, focus the game, wait at least two seconds, and run
   `overcrowctl status`. Repeat once with OverCrow launched before the game and
   once after it, then repeat in borderless fullscreen. Expected evidence:
   `active_game` identifies the intended game and has `backend: "wayland"`, a
   positive scale, and geometry matching the game on its current output. In
   every launch order, the OverCrow window must acquire the same KWin frame
   position and size as soon as both windows exist.

3. **Verify KWin overlay policy.** In both supported modes, switch between the
   game, another window, and the overlay. Expected visual evidence: the overlay
   remains above the game, has no window border, and is absent from the taskbar,
   pager, and task switcher. Move and resize the game, then move it to another
   output when two outputs are available. The overlay must follow the game's
   full `x`/`y`/`width`/`height` frame after each geometry/output change. Give
   the overlay focus for at least six seconds (three keepalives) and confirm it
   remains aligned without the overlay itself replacing `active_game`. If a
   test build can open multiple OverCrow top-level windows, repeat these checks
   with all of them alive and confirm every one follows the game. Capture a
   screenshot showing the stopwatch above the borderless game and record
   `overcrowctl status` at the same time.

4. **Verify the registered shortcut.** With a detected game, record
   `overcrowctl status`, press `Meta+Alt+O`, and record status again. Expected
   evidence: one key press changes `Passive` to `Interactive`; a second changes
   it back to `Passive`. Confirm click-through tracks those two states.

5. **Verify the Escape fail-safe on Wayland.** With a detected game, run
   `overcrowctl interactive`, record `overcrowctl status`, click the overlay so
   it owns keyboard focus, then press Escape. Record status again. Expected
   evidence: the mode changes from `Interactive` to `Passive` and a click on the
   prepared underlying target passes through to the game/test window. Merely
   seeing `Passive` without verifying restored mouse passthrough is not a
   passing result.

6. **Verify the five-second bridge lease.** Begin with a detected game, then
   temporarily disable the KWin package and ask KWin to reconfigure:

   ```sh
   kwriteconfig6 --file kwinrc --group Plugins --key io.github.overcrow.kwinEnabled false
   qdbus6 org.kde.KWin /KWin reconfigure
   # If the distribution provides qdbus instead, use it with the same arguments.
   sleep 6
   overcrowctl status
   ```

   Expected evidence: after the report keepalive has been absent for more than
   five seconds, `active_game` is `null` and the mode is `Passive`; mouse input
   is not trapped. Re-enable the package immediately, reconfigure KWin, refocus
   the game, and verify reporting resumes:

   ```sh
   kwriteconfig6 --file kwinrc --group Plugins --key io.github.overcrow.kwinEnabled true
   qdbus6 org.kde.KWin /KWin reconfigure
   overcrowctl status
   ```

   After confirming reporting resumed, restore both the original presence and
   value of the key. If it was absent, delete it rather than writing `false` or
   an empty substitute:

   ```sh
   if [ "$overcrow_kwin_enabled_before" = "$OVERCROW_KWIN_KEY_ABSENT" ]; then
       kwriteconfig6 --file kwinrc --group Plugins \
           --key io.github.overcrow.kwinEnabled --delete
   else
       kwriteconfig6 --file kwinrc --group Plugins \
           --key io.github.overcrow.kwinEnabled \
           "$overcrow_kwin_enabled_before"
   fi
   qdbus6 org.kde.KWin /KWin reconfigure
   # Use qdbus with the same arguments when qdbus6 is unavailable.
   ```

7. **Verify non-reportable game states, exit, and daemon restart.** With the
   game detected and the overlay interactive, minimize the game. `ClearWindow`
   must take effect immediately: `active_game` becomes `null`, mode becomes
   `Passive`, and the minimized game is no longer used to move/resize the
   overlay. Restore and refocus the game; a fresh `backend: "wayland"` report
   and exact placement must return. Then close the game and repeat X11 step 7.
   KWin may emit `windowHidden` before removal on Wayland; throughout that
   hidden/closed transition the old game must remain cleared and must not drive
   placement. Finally repeat X11 step 8 for daemon restart and repeat the
   `Meta+Alt+O` shortcut after restart.

The repository tests simulate KWin signals and geometry writes, but only this
live Plasma Wayland section can validate compositor-level placement, stacking,
focus, and D-Bus dispatch. Keep the requested evidence; automated smoke results
alone are not acceptance of the final Wayland placement fix.

## Stabilized Warframe widget acceptance on Hyprland

Run this sequence in order in one real Hyprland 0.55 session. Do not count the
source tests or simulated bridge fixtures as evidence for these compositor and
provider checks.

1. **Establish the selected game and widget catalog.** In Control Center,
   select Warframe (Steam App ID `230410`), enable OverCrow, and leave the game
   closed. Open the overlay settings and confirm that the catalog contains
   exactly these eleven entries, once each: Session, Clock, Performance, Manual
   stopwatch, Media, Notes, Warframe status, Fissures, Market, Sortie & Archon,
   and Invasions. Enable at least Session, Manual stopwatch, Warframe status,
   Fissures, Market, Sortie & Archon, and Invasions. Leave Market's Passive
   visibility disabled for the first run.

   Record the pre-launch state:

   ```sh
   overcrowctl status
   hyprctl -j globalshortcuts
   hyprctl -j binds | jq '[.[] | select(.dispatcher == "global" and
       (.arg == "com.playervox.OverCrow:toggle-overlay" or
        .arg == "com.playervox.OverCrow:toggle-manual-stopwatch" or
        .arg == "com.playervox.OverCrow:reset-manual-stopwatch"))]'
   ```

   Expected evidence: `active_game` is null, mode is `Passive`, and no physical
   OverCrow shortcut binding is owned before a selected game session.

2. **Launch Warframe and verify Passive read-only behavior.** Run Warframe in
   windowed or borderless-fullscreen mode, focus it, and wait at most two bridge
   keepalive periods. Cross-check `overcrowctl status` against
   `hyprctl -j clients`: the active selected game must be Warframe with App ID
   `230410`, and mode must remain `Passive`. The enabled Passive widgets are
   click-through and show no buttons, text fields, drag or resize handles,
   playback controls, copy actions, or completion toggles. The Market widget is
   absent by default. Prepared clicks must reach Warframe.

   World-state widgets may read only
   `https://api.warframe.com/cdn/worldState.php`. They require no login or API
   key. A displayed provider error may coexist with the last-good timestamp and
   payload for five minutes; after that payload ages past five minutes it must
   be cleared and shown unavailable, not redated as fresh.

3. **Enter Interactive and exercise Market.** Press `Meta+Alt+O`. Confirm mode
   changes to `Interactive`, the scrim appears, the overlay takes focus, and
   widget controls become available. In Market, enter a query, choose an item,
   wait for orders, and copy a generated trade message. Only this step may
   request `https://api.warframe.market/v2/items` and
   `https://api.warframe.market/v2/orders/item/{slug}`. The client must neither
   prompt for credentials nor follow a redirect to another host.

   Return to Passive, explicitly enable Market's Passive visibility, and
   inspect it again. It may show the last selected information, but must expose
   no search field, item selection, copy control, or new Market network request.
   Re-enter Interactive before continuing.

4. **Verify activity identity and rotation.** Toggle completion for one current
   Sortie/Archon item and one invasion, close and reopen the renderer, and
   confirm the same provider activity retains completion. If two simultaneous
   invasions share a node, their controls must still act independently. After a
   provider rotation or a fixture-equivalent naturally appears, reordered
   current activities retain their own state while departed activity keys are
   pruned; new activities must not inherit an older completion mark.

5. **Move focus to an editor and return to the game.** With the overlay still
   Interactive, focus VS Code or another editor beside Warframe. The editor
   must remain usable without repeated focus theft; Core must still identify
   the selected Warframe window as the geometry authority, and the overlay must
   not jump to the editor. Move the pointer back over Warframe and confirm the
   overlay regains interaction priority without a click. Press Escape or
   `Meta+Alt+O` to return to Passive, focus the editor once more, then focus
   Warframe again. The overlay shortcut must be released outside the game while
   Passive and reacquired when Warframe is focused. No foreign binding may be
   replaced or removed.

6. **Interrupt a resize without committing it.** Re-enter Interactive and
   complete one deliberate resize of a widget; close and reopen the renderer
   and confirm that size persists. Begin a second resize, then interrupt it by
   leaving Interactive or otherwise ending the gesture without a completed
   release. Reopen the overlay. The transient preview must be gone and the last
   completed width and height must remain. Repeat with a drag interruption and
   confirm only a completed drop changes stored position.

   The committed widget data belongs in
   `$XDG_CONFIG_HOME/overcrow/widgets.json` (or
   `~/.config/overcrow/widgets.json`). `overlay.json` is only a legacy migration
   source when `widgets.json` does not exist. Warframe preferences belong in
   `warframe.json`, and lifecycle selection/master state in `settings.json`.

7. **Verify the Core-owned Manual stopwatch.** While Warframe is active, press
   `Meta+Alt+P` and confirm the Manual stopwatch starts, then press it again and
   confirm it pauses. Press `Meta+Alt+R` and require zero. Immediately press
   `Meta+Alt+P`: the timer may advance optimistically from zero, but it must
   never revive the pre-reset elapsed time. Hide the control after sending an
   action and confirm a later Core sample remains authoritative. In a disposable
   fault-injection session where an action is deliberately not acknowledged,
   the first Core sample received at or after the three-second deadline must
   replace the optimistic state. In Passive, stopwatch buttons are absent even
   though an owned global shortcut may update the Core-authoritative value.

8. **Exit Warframe and verify fail-closed cleanup.** Enter Interactive, then
   close Warframe. Within five seconds require `active_game:null`, `Passive`, no
   rendered game widgets, no trapped input, and no OverCrow physical shortcut
   bindings. The bridge must remove only its exact in-memory bindings; no
   Hyprland configuration file may have been edited. Reopen the editor and
   confirm its shortcuts work normally.

Keep dated status output, the three shortcut views, relevant user-journal
extracts, and screenshots for every transition. A checked-in test result is not
a substitute for this real Warframe/Hyprland sequence.

## Extended Hyprland integration acceptance

1. **Confirm installation and session health.** In a real Hyprland 0.55 login,
   run:

   ```sh
   ./scripts/diagnose.sh
   systemctl --user status overcrow-core.service \
       overcrow-hyprland.service overcrow-overlay.service --no-pager
   hyprctl configerrors
   ```

   Expected evidence: both Hyprland sockets are present, the bridge is active,
   the managed config is linked, no new config error is reported, and exactly
   one overlay class is normally present. The diagnostic withholds client
   titles and raw JSON.

2. **Verify focus semantics with Palworld.** Keep Palworld running on one
   workspace and focus a browser on another. `overcrowctl status` must contain
   `active_game:null` and `Passive`. Focus Palworld, wait no more than two
   keepalive periods, and repeat the command. Expected evidence: the game PID,
   class `steam_app_1623730`, logical position/size, positive output scale, and
   `backend:"wayland"` agree with `hyprctl -j clients`. A running but unfocused
   game is intentionally inactive.

3. **Verify window policy and placement.** Test ordinary windowed mode and
   borderless fullscreen. With Passive visibility disabled for every widget, no
   panel is visible and the overlay is fully transparent in Passive mode. No
   Hyprland border, rounding shadow, blur, or animation is applied to the overlay.
   Interactive mode instead draws a uniform black scrim at about 70% opacity
   inside the game rectangle, with enabled widgets at their stored positions and
   a compact control bar at the bottom center. Move/resize the game, change
   workspace, toggle fullscreen, and move it to another monitor when available.
   The overlay must reconcile to the game's exact logical rectangle. Geometry
   is sampled every 33 ms, so an external Wayland overlay may show a one-to-two
   frame adjustment; it must not remain independently offset or sized. Give the
   overlay focus for at least six seconds and confirm the underlying game
   remains the reported source. Put VS Code beside the game on the same
   workspace while Interactive is enabled, focus VS Code, and confirm
   `active_game` still identifies Palworld and the overlay stays at the last
   game rectangle instead of moving or resizing over VS Code. Switch to another
   workspace and confirm the unpinned overlay is not visible there.

4. **Verify passthrough and shortcuts.** With Palworld focused, press
   `Meta+Alt+O`; status must change from `Passive` to `Interactive`, the scrim
   must appear, and `hyprctl activewindow` must identify
   `io.github.overcrow.Overlay` without requiring a click. Verify that keyboard
   and pointer actions inside the game rectangle no longer reach Palworld. The
   bottom bar must show `Super + Alt + O · open/close`, `Esc · close`, and the
   `Widgets` button. The Manual stopwatch panel shows its own
   `Super+Alt+P` / `Super+Alt+R` footer only while Interactive.

   Before and after each shortcut transition below, record these read-only
   views:

   ```sh
   hyprctl -j globalshortcuts
   hyprctl -j binds | jq '[.[] | select(.dispatcher == "global" and .arg == "com.playervox.OverCrow:toggle-overlay")]'
   overcrowctl status
   ```

   The portal view must expose `com.playervox.OverCrow:toggle-overlay`. Validate all five runtime
   ownership transitions:

   - Focus Palworld while Passive: exactly one `OverCrow overlay` binding for
     `SUPER + ALT + O` appears, and pressing it opens the overlay.
   - Close the overlay, then focus VS Code: the binding disappears and VS Code
     remains free to receive the same accelerator.
   - Refocus Palworld: the exact binding is reacquired and opens the overlay
     again.
   - Leave Palworld while the overlay is Interactive: the binding remains so
     the overlay can still be closed. After closing it outside the game, the
     binding disappears.
   - Exit Palworld or disable OverCrow: the binding disappears. Stopping the
     bridge must also remove one exact owned binding through its cleanup path.

   At no point may a foreign binding on the same accelerator be overwritten or
   removed. The diagnostic must report that case as
   `Hyprland OverCrow runtime shortcut: conflict on SUPER + ALT + O`.

   Open `Widgets`, enable Session and its `Passive mode` option. Close the
   overlay with Escape and confirm the Session panel remains visible while all
   clicks pass through to Palworld. Restart `overcrow-overlay.service`, refocus
   Palworld, and verify the panel is still visible in Passive mode. Re-enter
   Interactive, disable its Passive option, return to Passive, and verify the
   panel disappears.

   Start this timing check with a game that has already been running for at
   least two minutes. On the first visible frame, the session card must report
   approximately that process age rather than restarting at zero. Its value
   must advance once per second without freezing. Restart only
   `overcrow-overlay.service`: after reconnection, the displayed age must still
   follow the same game process and must not reset. If Core cannot obtain
   process timing, the safe fallback is `--:--:--`, never a plausible but false
   zero-based timer.

   While Interactive, drag the whole session card near the lower-right corner
   and release it. Escape to Passive and reopen the overlay: the card must keep
   its position and must not capture input while Passive. Restart the renderer
   and confirm the position remains. Resize the game smaller and larger, then
   move it between outputs when available: the card must preserve its relative
   normalized placement, remain fully inside the game rectangle with a margin,
   and settle with the overlay's normal one-to-two-frame geometry adjustment.
   Open `Widgets` and click Session's `Reset`; the card must return to the
   upper-left margin and remain there after another renderer restart.

   The persisted JSON must be under
   `$XDG_CONFIG_HOME/overcrow/widgets.json`, falling back to
   `~/.config/overcrow/widgets.json`. A legacy `overlay.json` is only read for
   migration when `widgets.json` is absent. The current file may contain
   Passive visibility, normalized position ratios between `0.0` and `1.0`,
   scale, dimensions, and background choice; elapsed time itself must not be
   persisted.

   Re-enter Interactive and inspect the live clients:

   ```sh
   hyprctl clients -j | jq '[.[] | {
       class,
       tags: [.tags[]? | select(. == "overcrow-interactive" or
           . == "overcrow-game-input-blocked")]
   } | select(.tags | length > 0)]'
   ```

   Exactly one Palworld client must carry `overcrow-game-input-blocked`, and
   the exact overlay must carry `overcrow-interactive`.

   First test with the in-game menu closed and no visible game cursor. Keep
   Interactive active, move from the overlay across its border onto VS Code,
   then return without clicking. Palworld must never recover movement, camera,
   keyboard, or Escape handling; VS Code may gain focus without OverCrow
   repeatedly stealing it, Core must remain Interactive, and re-entry must make
   `hyprctl activewindow` identify `io.github.overcrow.Overlay` again.

   Repeat with the in-game menu open and its cursor visible. Moving toward VS
   Code must not warp or magnetize the pointer repeatedly back into Palworld.
   VS Code must remain usable, and returning over the game rectangle must give
   the overlay priority without a click.

   Press the shortcut again and verify the scrim disappears, both reserved tags
   disappear, the game regains focus, and a prepared click reaches it. Enter
   Interactive once more and press Escape; verify the same `Passive` state,
   tag removal, game focus restoration, and click-through. Global compositor
   shortcuts may still fire, and controller input is not captured. The managed
   exact-class rule must contain `no_focus on` and `no_follow_mouse on`; the
   `overcrow-interactive` tag rule must override them with `no_focus off` and
   `no_follow_mouse off`; and the `overcrow-game-input-blocked` tag rule must
   apply `no_focus on`. No rule may pin the overlay. `Ctrl+Tab` must remain
   available to applications and must not appear in the managed OverCrow
   bindings.

   Re-enter Interactive after the overlay and game rectangles have converged.
   Use Omarchy's configured grow/shrink-active-window shortcuts. Although
   Hyprland initially targets the focused overlay, within one or two frames the
   exact delta must be applied to the uniquely validated game and the overlay
   must converge to it. Repeat once while the game itself is focused: the game
   remains authoritative. A simultaneously changed game and overlay must never
   cause a third-party or stale rectangle to be dispatched to the game.

5. **Verify fail-closed lifecycle.** Enter Interactive, then minimize or close
   Palworld. Within five seconds require `active_game:null`, `Passive`, no
   rendered game widgets, and no trapped mouse input. Restart/refocus the game,
   then run:

   ```sh
   systemctl --user stop overcrow-hyprland.service
   hyprctl clients -j | jq '[.[] | select(any(.tags[]?;
       . == "overcrow-game-input-blocked" or . == "overcrow-interactive"))]'
   sleep 6
   overcrowctl status
   systemctl --user start overcrow-hyprland.service
   ```

   Expected evidence immediately after the stop: the tag query returns `[]`
   and Palworld is focusable, proving `ExecStopPost` cleanup did not wait for
   Core. After six seconds the bridge lease expires to null/Passive. After
   restart and game focus, reporting and exact placement resume.

6. **Verify integration idempotence.** Disable and re-enable OverCrow from the
   Control Center and confirm there is one managed marker block, not a
   duplicate. Do not inject a deliberate syntax error into the live user
   config or Omarchy defaults.

Only these live checks validate Hyprland stacking, focus, raw socket dispatch,
and compositor rendering. Passing the Rust and shell fixtures alone is not a
live Hyprland acceptance result.

## Event-driven performance acceptance

Run this measurement on the real desktop session after the functional
Hyprland checks, not in a container or nested compositor. Use the same game,
presentation mode, widget profile, machine power mode, and **60-second** sample
duration for every applicable state. Close unrelated heavy workloads and allow
each state to settle for 15 seconds before sampling. Results are comparative;
there is no universal acceptable CPU percentage.

Create an evidence directory and record the installed revision and service
PIDs. A zero `MainPID` is valid when lifecycle policy has stopped that unit:

```sh
evidence="overcrow-performance-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$evidence"
git rev-parse HEAD >"$evidence/revision.txt" 2>/dev/null || true

record_processes() {
  label=$1
  core_pid=$(systemctl --user show overcrow-core.service -p MainPID --value)
  overlay_pid=$(systemctl --user show overcrow-overlay.service -p MainPID --value)
  bridge_pid=$(systemctl --user show overcrow-hyprland.service -p MainPID --value)
  pids=$(printf '%s\n' "$core_pid" "$overlay_pid" "$bridge_pid" |
    awk '$1 ~ /^[1-9][0-9]*$/ { if (seen[$1]++ == 0) printf "%s%s", sep, $1; sep="," }')

  systemctl --user show overcrow-core.service overcrow-overlay.service \
    overcrow-hyprland.service -p Id -p ActiveState -p SubState -p MainPID \
    >"$evidence/$label-services.txt"

  if [ -n "$pids" ]; then
    ps -p "$pids" -o pid,comm,%cpu,rss,nlwp,etime \
      >"$evidence/$label-processes-before.txt"
    for pid in $(printf '%s' "$pids" | tr ',' ' '); do
      sed -n '/^voluntary_ctxt_switches:/p;/^nonvoluntary_ctxt_switches:/p' \
        "/proc/$pid/status" >"$evidence/$label-$pid-context-before.txt"
    done

    if command -v pidstat >/dev/null 2>&1; then
      pidstat -u -r -w -p "$pids" 1 60 >"$evidence/$label-pidstat.txt"
    else
      sleep 60
    fi

    ps -p "$pids" -o pid,comm,%cpu,rss,nlwp,etime \
      >"$evidence/$label-processes-after.txt"
    for pid in $(printf '%s' "$pids" | tr ',' ' '); do
      [ -r "/proc/$pid/status" ] || continue
      sed -n '/^voluntary_ctxt_switches:/p;/^nonvoluntary_ctxt_switches:/p' \
        "/proc/$pid/status" >"$evidence/$label-$pid-context-after.txt"
    done
  else
    sleep 60
  fi
}
```

For optional D-Bus evidence, run the following in a second terminal during
each 60-second sample, replacing `STATE` with the same label. It observes the
user bus without invoking an OverCrow method:

```sh
timeout 60s busctl --user monitor io.github.overcrow.Core1 \
  >"$evidence/STATE-dbus.txt" 2>&1 || [ "$?" -eq 124 ]
```

Count incoming `SnapshotVersioned` method calls and `SnapshotChanged`
notifications separately. Notifications may be coalesced and revision gaps are
valid; they are wake hints for convergence to the newest snapshot, not a
lossless event stream. In a current-version steady state, snapshot reads should
be limited to connection/baseline and 30-second reconciliation needs. A regular
250 ms series is acceptable only while deliberately testing the documented
mixed-version fallback against a Core that returns `UnknownMethod` for
`SnapshotVersioned`.

Measure these states in order, using a distinct short label:

1. `no-game`: the selected game is stopped. Confirm the overlay and compositor
   bridge are stopped by lifecycle policy, then run `record_processes no-game`.
2. `passive`: start and focus the selected game, leave OverCrow Passive, keep
   the pointer and widgets idle, then run `record_processes passive`.
3. `interactive`: open the overlay with `Meta+Alt+O`, do not interact with its
   controls during the sample, then run `record_processes interactive`.
4. `stopwatch`: start the Manual stopwatch with `Meta+Alt+P`, return the overlay
   to Passive, confirm its display continues advancing, then run
   `record_processes stopwatch`.
5. `warframe-unchanged`: pause the Manual stopwatch with `Meta+Alt+P`, reset it
   with `Meta+Alt+R`, and allow the overlay to settle for 15 seconds. With
   Warframe selected and focused, enable the five Warframe widgets, wait for
   any initial requests to complete, make no filter, query, or catalog change,
   and run `record_processes warframe-unchanged`.

Record presentation mode and any provider refresh that occurred during a
sample. Compare before/after RSS and thread counts, averaged CPU when `pidstat`
is available, context-switch deltas as a scheduler-wakeup proxy, and D-Bus
counts. If a pre-pass build is available, repeat the exact sequence with that
binary; otherwise retain this run as the baseline for the next release. Pass
requires all existing focus, input, geometry, clock, stopwatch, and widget
behavior to remain correct, with no unexplained growth or periodic high-rate
snapshot calls. When comparing builds, the event-driven build must reduce Core
snapshot calls and steady worker wakeups in the unchanged states. Do not claim
live performance acceptance without attaching these files and the functional
Hyprland result.

## Supported presentation boundary

Acceptance covers **windowed and borderless-fullscreen windows only**. Exclusive
fullscreen is explicitly unsupported in the MVP because it may bypass normal
compositor windows. Do not count an absent or covered overlay in exclusive mode
as an MVP regression, and do not mark acceptance as failed solely because that
mode cannot be overlaid. Record the game's presentation mode with every visual
result so borderless is not mistaken for exclusive fullscreen.

## Restore and stop

1. Return the overlay to its safe state with `overcrowctl passive`.
2. If the KWin lease-test restoration was not already completed, run its exact
   sentinel-based restoration block now: use `kwriteconfig6 --delete` when the
   key was initially absent, otherwise write its recorded original value. Then
   run `qdbus6 org.kde.KWin /KWin reconfigure` (or the equivalent `qdbus`
   command).
3. Quit the test game normally.
4. Close the OverCrow window from the desktop. If it cannot be closed normally,
   terminate only the renderer process shown by `./scripts/diagnose.sh`.
5. If the user service was inactive before testing, stop it with:

   ```sh
   systemctl --user stop overcrow-core.service
   ```

6. Run `./scripts/diagnose.sh` one final time and retain it with the acceptance
   evidence. The expected stopped state is no renderer process and, when the
   service was stopped, no D-Bus owner.
