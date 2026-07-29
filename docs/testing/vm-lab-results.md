# Virtual machine acceptance results

Record the exact OverCrow commit and test each row independently. Valid result
values are `PASS`, `FAIL`, and `BLOCKED`.

| Environment | Session | Commit | Native game | Proton game | Overlay | Input | Scaling | Recovery | Result |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Bazzite KDE | Plasma Wayland | | | | | | | | |
| Xubuntu 24.04 | XFCE X11 | | | | | | | | |
| Debian 13 KDE | Plasma Wayland | | | | | | | | |
| CachyOS KDE | Plasma Wayland | | | | | | | | |
| CachyOS KDE | Plasma X11 | | | | | | | | |
| CachyOS XFCE | XFCE X11 | | | | | | | | |

## Native package acceptance

| Date | Environment | Package | Build | Reboot persistence | Integrity | Inert services | Result |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 2026-07-29 | Bazzite KDE, Fedora 42 base | `overcrow-0.1.0~pre_alpha.3-1.fc42.x86_64` | PASS | PASS | PASS | PASS | PASS |

`rpm -V --nomtime overcrow` returned zero after reboot. OSTree changed only
payload timestamps. `overcrow-core` was disabled and all three user services
were inactive. Graphical onboarding and in-game acceptance remain separate
manual checks and are not implied by this package result.

## Failure record

Copy this section for each `FAIL` or `BLOCKED` result.

```text
Environment:
Session:
OverCrow commit:
Result:
First failing checklist item:
Minimal reproduction:
Sanitized diagnostic report:
Relevant bounded log events:
Physical-hardware retest required:
```

Do not include Steam or Twitch credentials, chat messages, usernames, notes,
game paths, host paths, or other private content.
