# Using toyoterm

See the [documentation index](README.md) for configuration, installation, and development guides.

## Controls

There are no built-in GUI key bindings. Copy the bindings from
[default configuration](../examples/default_config.rb) into your `config.rb` and change them as needed.

- Type normally to send input to the PTY
- Click a workspace or tab label to activate it
- Drag with the left mouse button: select text
- Mouse wheel: scroll through history, or report wheel input when the terminal application requests mouse reporting
- Control+click on Linux/Windows or Command+click on macOS: open an OSC 8 or detected web/mail link after scheme validation

See [URL opening security](url-security.md) for the allowed schemes and limits.

When a shell exits, toyoterm closes its pane automatically. Empty tabs and workspaces are collapsed, and exiting the final pane closes toyoterm. A pane is retained after a PTY read error so the failure remains visible for diagnosis.

### Clipboard security

OSC 52 clipboard access is disabled in v0.1. Terminal output may originate from an untrusted local process or remote host, so allowing OSC 52 would let it write the host clipboard without an explicit user gesture; clipboard query responses could also expose clipboard contents. Configured copy and paste shortcuts and the trusted-configuration Ruby API remain available. Future OSC 52 support must be opt-in, keep clipboard reads disabled by default, and provide an explicit permission or confirmation UI with a payload size limit.

## CLI

Start a new GUI process:

```text
toyoterm [--config PATH] [--title TITLE] [--app-id APP-ID]
         [--working-directory DIR] [-e COMMAND [ARG...]]
toyoterm gui [same GUI options]
```

With no command, toyoterm opens the GUI. `--app-id` sets the Wayland app ID or
X11 class on Linux. `--dir` aliases `--working-directory`; `--execute` and `--`
alias `-e`. Value options also accept `--option=value`. All arguments after
`-e`, `--execute`, or `--` belong to the child command, so put GUI options first.
The command is executed directly; use a shell explicitly for shell syntax.

```sh
toyoterm --working-directory /path/to/project -e bash -lc 'git status; exec bash'
```

The Linux desktop entry advertises launch options to `xdg-terminal-exec`,
allowing desktop integrations such as Omarchy to supply a title, application
ID, working directory, and command.

Only these commands connect to an existing GUI over local IPC:

| Command | Behavior |
| --- | --- |
| `toyoterm list` | Show the GUI's current mux state. |
| `toyoterm reload` | Reload its selected configuration file. |
| `toyoterm ruby console` | Open a multiline Ruby REPL in its persistent VM; supports `:history` and `exit`. `toyoterm ruby` is an alias. |
| `toyoterm cli list-panes` | List panes. |
| `toyoterm cli send-text --pane ID TEXT` | Send text to a pane; multiple text arguments are joined with spaces. |
| `toyoterm cli split [left\|right\|up\|down]` | Split the active pane; defaults to `right`. Also accepts `--direction DIRECTION`. |
| `toyoterm cli activate-workspace NAME` | Activate or create a workspace. |

If multiple GUIs are running, clients select the most recently started instance.
Set the same `TOYOTERM_INSTANCE` when starting a GUI and invoking its clients to
select a named instance. See [local IPC](ipc.md) for runtime paths,
authentication, and the same-user control boundary.

The following commands run locally without connecting to an existing GUI:

| Command | Behavior |
| --- | --- |
| `toyoterm shell-integration SHELL` | Print the bundled script for `bash`, `zsh`, `fish`, or `powershell`. |
| `toyoterm demo` | Exercise the mux's tabs and splits without a GUI. |
| `toyoterm pty-demo` | Spawn a process in a PTY. |
| `toyoterm screen-demo` | Parse PTY output into a terminal snapshot. |
| `toyoterm gui-smoke-test` | Open a temporary GUI and verify startup; requires a display and GPU support. |
| `toyoterm version` | Print the version; aliases: `--version`, `-V`. |
| `toyoterm help` | Print help; aliases: `--help`, `-h`. |

See [shell integration](shell-integration.md) for startup-file instructions.

## Logging

Diagnostics are written to stderr through `tracing`; the default level is `warn`. `TOYOTERM_LOG` sets the global level or comma-separated target filters. The available targets are `toyoterm::pty`, `toyoterm::render`, `toyoterm::mux`, `toyoterm::script`, `toyoterm::config`, `toyoterm::app`, and `toyoterm::ipc`. Short target names such as `pty` are accepted.

Dynamic key-binding and event callback durations are emitted at `debug` under `toyoterm::script`. Callbacks taking 100 ms or longer are logged at `warn` as slow callbacks, including their kind, name, duration, and success state.

```sh
TOYOTERM_LOG=debug toyoterm
TOYOTERM_LOG=warn,pty=trace,render=debug toyoterm
```

v0.1 writes logs only to stderr and does not create or rotate log files. Redirecting stderr is an explicit user choice, so retention and rotation then belong to the surrounding process manager. Logs never intentionally include PTY input/output, clipboard contents, or configuration source text. Diagnostics can include configuration paths, process and pane identifiers, callback names, dimensions, error messages, and Ruby backtraces; review them before sharing.

PowerShell uses environment assignment syntax such as:

```powershell
$env:TOYOTERM_LOG = 'warn,pty=trace,render=debug'
toyoterm
```

## Configuration errors and reload

GUI configuration failures show a non-fatal banner. `Open Log` expands the
diagnostic and `Dismiss` closes the banner. Invalid startup configuration falls
back to defaults while retaining the selected path for a later reload.
`toyoterm reload` retries that path after you fix the file. A failed reload
preserves the active configuration and running terminal sessions.

For configuration transactions and live updates, see the
[mruby API reference](mruby-api.md#loading-configuration).

## Rendering

A Nerd Font's exact installed family name can be set in `config.font.family`;
the Mono variant is useful for terminal prompts. Glyphs are positioned at
terminal cell coordinates. Fallback families, wallpaper, and opacity settings
are documented in the [configuration reference](mruby-api.md#configfont).

Windows requires a DirectX 12-capable graphics device and uses DirectComposition
with premultiplied alpha. Transparency remains enabled at opacity `1.0` so it
can be lowered again at runtime. Physical HDR/SDR and DPI checks are covered in
[platform validation](platform-validation.md#windows).
