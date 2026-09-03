# Shell integration

toyoterm uses established terminal escape sequences so shell state can be
observed without capturing command text or changing the PTY protocol.

## Protocol

Shells write these OSC sequences to the terminal. Both BEL (`0x07`) and ST
(`ESC \\`) terminators are accepted.

| State | Sequence | Pane metadata |
| --- | --- | --- |
| Working directory | `OSC 7;file://<host>/<percent-encoded-path> ST` | `cwd` |
| Command start | `OSC 133;C ST` | `command_running? = true`, Ruby `command_started` event |
| Command end | `OSC 133;D;<decimal-status> ST` | `command_running? = false`, `last_exit_status`, Ruby `command_finished` event |

OSC payloads are limited to 8 KiB. Invalid UTF-8 paths and malformed status
values are ignored; an OSC 133 command-end marker without a valid status still
ends the running state and clears `last_exit_status`. Command text is never
included in the protocol.

OSC 7 is also accepted independently of the bundled scripts, preserving cwd
updates from shells and remote tools that already emit it. Title changes remain
the standard OSC 0/2 terminal events. Title and cwd changes are delivered to
Ruby as `title_changed` and `cwd_changed` events. Command lifecycle events expose
the affected `pane`; `command_finished` also exposes `exit_status`, or `nil`
when no valid decimal status was reported.

## Enabling a shell

toyoterm sets `TERM_PROGRAM=toyoterm` and embeds each script in the executable.
Add the matching line to the shell's interactive startup file:

```bash
# ~/.bashrc
source <(toyoterm shell-integration bash)

# ~/.zshrc
source <(toyoterm shell-integration zsh)
```

```fish
# ~/.config/fish/config.fish
toyoterm shell-integration fish | source
```

```powershell
# $PROFILE
toyoterm shell-integration powershell | Out-String | Invoke-Expression
```

Every script is guarded by `TERM_PROGRAM`, interactive-shell detection, and an
idempotence variable, so the same startup file remains usable in other terminal
emulators and nested initialization does not install duplicate hooks.
