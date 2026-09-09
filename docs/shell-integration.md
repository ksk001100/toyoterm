# Shell integration

toyoterm uses established terminal escape sequences so shell state can be
observed without capturing command text or changing the PTY protocol.

## Protocol

Shells write these OSC sequences to the terminal. Both BEL (`0x07`) and ST
(`ESC \`, bytes `0x1b 0x5c`) terminators are accepted.

| State | Sequence | Pane metadata |
| --- | --- | --- |
| Working directory | `OSC 7;file://<host>/<percent-encoded-path> ST` | `cwd` |
| Remote host | `OSC 1337;RemoteHost=<user>@<host> ST` | `remote_host` |
| User variable | `OSC 1337;SetUserVar=<name>=<base64-value> ST` | `user_vars` |
| Prompt start | `OSC 133;A ST` | Ruby `prompt_started` event |
| Command-line start | `OSC 133;B ST` | Ruby `command_line_started` event |
| Command start | `OSC 133;C ST` | `command_running? = true`, Ruby `command_started` event |
| Command end | `OSC 133;D;<decimal-status> ST` | `command_running? = false`, `last_exit_status`, Ruby `command_finished` event |

OSC payloads are limited to 8 KiB. Invalid UTF-8 paths and malformed status
values are ignored; an OSC 133 command-end marker without a valid status still
ends the running state and clears `last_exit_status`. Command text is never
included in the protocol.

OSC 7 is also accepted independently of the bundled scripts, preserving cwd
updates from shells and remote tools that already emit it. The iTerm2-compatible
`OSC 1337;CurrentDir=<path>` form is accepted as an alternate cwd report.
`OSC 1337;RemoteHost=<user>@<host>` stores the bounded report on
`Pane#remote_host`; an empty user is accepted, while a missing host, invalid
UTF-8, control characters, and values over 1 KiB are ignored. Title changes remain
the standard OSC 0/2 terminal events. Title and cwd changes are delivered to
Ruby as `title_changed` and `cwd_changed` events. Prompt and command lifecycle
events expose the affected `pane`; `command_finished` also exposes
`exit_status`, or `nil` when no valid decimal status was reported. The bundled
Bash, Zsh, Fish, and PowerShell scripts emit all four markers, and compatible
external shell integrations are accepted as well.

toyoterm retains up to 4,096 `A`/`B`/`C`/`D` positions per terminal and moves
them with the scrollback grid. The `previous_prompt` and `next_prompt` actions
cycle through `A` positions in the active pane. `select_last_command_output`
selects the most recent complete `C`–`D` range, while
`select_previous_command_output` and `select_next_command_output` cycle through
all complete ranges, so any retained command output can be copied or yanked
through the normal selection actions. Each completed range also receives a
thin margin marker: the configured green ANSI color for status 0, red for a
nonzero status, and the foreground color when no valid status was supplied.
Markers are discarded on a
terminal resize because the backend does not expose a reliable mapping across
line reflow. The default example binds prompt navigation to leader+`[` /
leader+`]`, command-output cycling to leader+`p` / leader+`n`, and latest-output
selection to leader+`o`. Applications that emit only `C`/`D` still produce
lifecycle events but cannot be used for prompt navigation.

`SetUserVar` names must be non-empty UTF-8 without control characters and are
limited to 128 bytes. Values must decode to UTF-8 and are limited to 4 KiB;
each pane retains at most 64 distinct names. Updating an existing name remains
allowed after that limit is reached. The resulting read-only snapshot is
available as `Pane#user_vars`.

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
