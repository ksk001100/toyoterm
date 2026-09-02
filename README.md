# toyoterm

<p align="center">
  <img src="packaging/app-icon.png" alt="toyoterm icon" width="180">
</p>

[日本語](README.ja.md)

toyoterm is an experimental, programmable terminal emulator powered by Rust and embedded mruby. Its terminal hot path stays native, while Ruby is used for configuration, dynamic key bindings, runtime events, and commands.

This is a personal project built for my own use and an experimental toy.

> [!IMPORTANT]
> toyoterm is under active development. GUI workspaces, tabs, and split panes have independent PTY and terminal sessions. Multiple OS windows are intentionally deferred until after the initial release.

## Features

- Native PTY and platform-default shell
- GPU-rendered terminal window using `wgpu` and `glyphon`
- VT parsing backed by `alacritty_terminal`
- UTF-8 input, terminal resize, scrollback, and mouse-wheel support
- IME preedit rendering with commit and cancellation handling
- Text selection and clipboard copy/paste
- Embedded mruby 4.0 configuration runtime
- Dynamic Ruby key bindings that dispatch native commands
- Atomic configuration reload: invalid updates leave the previous config active
- Ruby events for startup/reload, windows, tabs, panes, title, cwd, bell, and workspace changes
- Native command and mux model for tabs, pane splits, and workspaces
- GUI tabs with one PTY and terminal backend per pane
- Rendered split panes with per-pane resize and focus
- A clickable tab bar with keyboard tab navigation
- A clickable workspace bar with per-workspace focus restoration
- User-defined Ruby commands and configurable native key bindings
- A live Ruby REPL connected to the running GUI's single mruby VM
- Local Ruby plugins with metadata, compatibility checks, and failure isolation
- Literal search across the viewport and scrollback
- OSC 8 hyperlinks and plain-URL detection with safe modifier-click opening
- Shell integration, a local IPC CLI, and a Ruby-driven status bar

## Current status

The primary development environment is Linux. The architecture and dependencies are cross-platform, but macOS and Windows support has not yet been fully validated.

Outside the initial release scope:

- Multiple OS windows
- Image protocols and session persistence

The main features are implemented, but the initial release still requires interactive validation on Linux Wayland/X11, macOS, and Windows, together with performance and image-regression coverage.

## Build and run

### Requirements

- A recent stable Rust toolchain
- A C compiler for the vendored mruby amalgamation
- Platform development libraries required by `winit`/`wgpu`

On Linux, a working Wayland or X11 desktop session is required. Install your distribution's C build tools, `pkg-config`, Wayland/X11, and xkbcommon development packages if they are not already available.

After cloning the repository, run:

```sh
cd toyoterm
cargo run
```

Build an optimized binary:

```sh
cargo build --release --locked
./target/release/toyoterm
```

### Install, upgrade, and uninstall

Download the artifact for your OS and CPU from a release. On Linux, extract the
archive and run `./install.sh`; this installs to `~/.local` and adds a desktop
menu entry. On macOS, open the DMG and drag `toyoterm.app` to Applications (the
`.tar.gz` is also available). On Windows, either run `Install-Toyoterm.ps1` from
the extracted portable zip or keep using it in place without installation.

Installing a newer artifact upgrades the existing installation. Linux installs
an uninstaller at `~/.local/lib/toyoterm/uninstall.sh`; Windows installs
`Uninstall-Toyoterm.ps1` next to the executable. User configuration under
`~/.config/toyoterm/` is deliberately retained. Every release includes SHA-256
checksums. See the [packaging and installation guide](docs/packaging.md) for
custom locations, portable use, verification, and uninstall details.

Use an explicit configuration file:

```sh
cargo run -- --config /path/to/config.rb
```

Connect a live Ruby REPL to the running GUI from another terminal. It supports multiline input, `:history`, and `exit`.

```sh
cargo run -- ruby console
```

## Configuration

toyoterm looks for configuration in this order:

1. The path passed with `--config`
2. `TOYOTERM_CONFIG_FILE`
3. `~/.config/toyoterm/config.rb`

The default path is optional. An explicitly selected file must exist and contain valid Ruby.

Example configuration:

```ruby
Toyoterm.configure do |config|
  config.font do |font|
    font.family = "monospace"
    font.fallback = ["Noto Sans Mono CJK JP", "Noto Color Emoji"]
    font.size = 14
    font.weight = 400
  end

  config.colors do |colors|
    colors.background = "#090b0e"
    colors.foreground = "#dce1e8"
    colors.cursor = "#f5f7fa"
    colors.selection = "#375891"
    # ANSI indexes 0..15 can be themed individually.
    colors.ansi[1] = "#ff5f56"
  end

  config.window.opacity = 0.96
  config.scrollback_lines = 20_000
  config.leader key: "b", mods: "CTRL", timeout: 1000

  # Set an explicit shell when needed. Otherwise the platform default is used.
  # config.default_shell = "/bin/zsh"

  config.bind "CTRL+SHIFT+H" do |context|
    context.pane.send_text("echo hello from mruby\n")
  end

  # Common actions compile to native bindings and do not invoke mruby on key press.
  config.keys do
    leader("v").split(:right)
    ctrl_shift("e").split(:right)
    ctrl_shift("o").activate_pane(:right)
    ctrl_shift("t").new_tab
    ctrl_shift("r").reload_config
    alt("F10").toggle_maximize
    ctrl_shift("F11").toggle_fullscreen
  end
end

Toyoterm.on :app_started do |event|
  event.pane.send_text("echo toyoterm started\n")
end

Toyoterm.on :config_reloaded do |event|
  event.pane.send_text("echo config reloaded\n")
end
```

Toyoterm renders ANSI 256-color foregrounds and backgrounds. `colors.ansi`
contains the themeable base colors at indexes 0 through 15; indexes 16 through
231 use the standard 6×6×6 xterm color cube and indexes 232 through 255 use its
grayscale ramp. Assigning the entire `colors.ansi` array requires exactly 16
`#RRGGBB` strings.

`font.fallback` is optional. Installed families are tried in the listed order for missing CJK, emoji, symbol, and other glyphs, followed by the platform defaults. Unknown families are skipped by the font system.

### Key bindings

Key names are case-insensitive. Modifiers use names such as `CTRL`, `SHIFT`, `ALT`, and `SUPER`. Named keys include `ENTER`, `TAB`, `SPACE`, arrow keys, navigation keys, and `F1` through `F12`.

`config.keys` provides `key`, `ctrl`, `ctrl_shift`, `ctrl_alt`, `ctrl_super`, `primary`, `primary_shift`, `primary_alt`, `alt`, `super_key`, `leader`, and `physical` helpers. Static actions include pane and tab management, workspace and tab cycling, search, window state changes, reload, clipboard copy/paste, and visual selection (`start_visual_mode`, `toggle_visual_mode`, `start_visual_selection`, `select_visual_selection`, `end_visual_selection`, `move_visual_selection`, and `yank_selection`). `primary` resolves to `SUPER` on macOS and `CTRL` on Linux/Windows, so one configuration can follow each platform's conventions. Modifier names are portable: `ALT` is the Option key on macOS, while `SUPER` is Command on macOS and the Windows key on Windows. The `physical` helper distinguishes a hardware position from the logical character, for example `physical("KeyH", "CTRL")`. Physical bindings take priority over logical bindings. There are no built-in GUI key bindings; defining the same chord more than once is a configuration error.

`config.leader` defines a native leader prefix with a timeout in milliseconds. `leader("v")` bindings are resolved without invoking mruby. The leader prefix is discarded; an unmatched or expired suffix continues through normal key handling. Prefix repeat events are consumed without extending the original timeout, while IME activity, focus loss, and configuration reload clear leader state.

Unmatched keys bypass mruby and go directly through the native terminal key encoder. If a Ruby callback raises an exception, toyoterm logs the error and keeps the shell running.

For example, a visual selection can be configured with `leader("v").toggle_visual_mode`, `key("SPACE").select_visual_selection`, `move_visual_selection(:left)`, `:right`, `:up`, `:down`, `:line_start`, or `:line_end`, and `yank_selection`. Entering visual mode does not select anything; move to the desired log line first, then select and extend the range. Movement and selection actions are inactive in normal mode, so bindings for `h/j/k/l` do not intercept ordinary shell input; using the leader also keeps normal `v` available to the shell.

Ruby callbacks can access the host text clipboard through `Toyoterm.clipboard.read` and `Toyoterm.clipboard.write(text)`. The clipboard snapshot is refreshed immediately before each dynamic key-binding or event callback. `read` raises `RuntimeError` when the platform clipboard is unavailable. Writes are applied only after the callback completes successfully, so a callback exception rolls them back together with its other queued commands.

```ruby
config.bind "CTRL+SHIFT+Y" do
  Toyoterm.clipboard.write("pane #{Toyoterm.current_pane.id}")
end

config.bind "CTRL+SHIFT+P" do |context|
  context.pane.send_text(Toyoterm.clipboard.read)
end
```

Trusted configuration can also use the host environment, filesystem, and child processes:

```ruby
home = Toyoterm.env["HOME"]
contents = Toyoterm.read_file("/path/to/file")
result = Toyoterm.spawn("git", "status", "--short")
warn result.stderr unless result.success?
```

`Toyoterm.env` returns a copy of the environment snapshot taken when the Ruby VM is created; changing the Hash does not change the process environment. Entries that cannot be represented as UTF-8 are omitted. Paths, program names, and arguments must be UTF-8 and cannot contain NUL bytes. `read_file` returns a byte-preserving Ruby String. `spawn` runs synchronously on the script thread, captures byte-preserving `stdout` and `stderr`, and returns a `Toyoterm::ProcessResult` with `stdout`, `stderr`, `exit_status`, and `success?`; a process terminated without a portable exit code uses `-1`. Filesystem and process-launch failures raise `RuntimeError`, while a nonzero child exit is a normal result. These calls do not block PTY reading or rendering, but a long-running child delays other Ruby callbacks.

Configuration is trusted code and these APIs are intentionally unrestricted in the MVP. Local plugins currently run in the same mruby VM with the same filesystem, process, environment, and clipboard authority. Installing a plugin is therefore equivalent to allowing arbitrary code execution; only install plugin files whose source and updates you trust. A separate filesystem/process/network/clipboard capability model is deferred rather than claiming a sandbox that does not exist.

### Local plugins

At startup and on configuration reload, toyoterm loads `*.rb` files directly inside `~/.config/toyoterm/plugins/` in lexicographic filename order. The configuration can then append plugins in declaration order; relative paths are resolved from the declaring config or plugin file, and `~/` expands to the user's home directory:

```ruby
Toyoterm.plugin "plugins/project.rb"
Toyoterm.plugin "~/.config/toyoterm/extra/status.rb"
```

Every plugin file must define exactly one plugin with a unique name and a semantic version. `requires` is optional and constrains the toyoterm plugin API version (`0.1.0`) using comma-separated `=`, `<`, `<=`, `>`, or `>=` clauses.

```ruby
Toyoterm::Plugin.define "git-tools" do |plugin|
  plugin.version = "0.1.0"
  plugin.requires = ">= 0.1.0, < 0.2.0"

  plugin.command :git_root do |context|
    context.pane.send_text("git rev-parse --show-toplevel\n")
  end

  plugin.on :bell do |event|
    event.pane.badge = "bell"
  end

  plugin.bind "CTRL+G" do |context|
    context.pane.send_text("git status\n")
  end

  plugin.keys do
    ctrl_shift("G").command(:git_root)
  end
end
```

`plugin.command`, `plugin.on`, `plugin.bind`, and `plugin.keys` use the same command, event, dynamic-binding, and native-binding APIs as the main configuration. Loading the same canonical path twice is ignored. Duplicate plugin names or registrations, invalid metadata, incompatible API requirements, unreadable files, and Ruby exceptions disable only that plugin: all registrations made by the failed plugin are rolled back, remaining plugins continue loading, and a warning is written to `toyoterm::script`. A config error still rejects the complete candidate VM atomically.

### Ruby object model

Each callback receives a current snapshot through `Toyoterm.current_workspace`, `current_window`, `current_tab`, and `current_pane`. `Toyoterm.workspaces`, `windows`, and `workspace(name)` provide lookup; workspace, window, and tab objects expose their children. Pane metadata includes `title`, `cwd`, `pid`, `command_running?`, and `last_exit_status`. The command fields are populated when [shell integration](docs/shell-integration.md) is enabled. Mutating methods such as `split`, `close`, `focus`/`activate`, `new_tab`, and `create_window` enqueue native commands and take effect after the callback returns successfully. A saved object raises `Toyoterm::InvalidHandleError` after its native object is deleted.

`pane.badge` is callback-owned display metadata stored by pane ID for the lifetime of the current Ruby VM. Rendering badges is intentionally separate from this API contract. `pane.chdir` is not provided: the shell owns its working directory, so configurations that want shell-specific directory changes should use `pane.send_text("cd ...\n")` with appropriate shell escaping.

### Runtime events

`Toyoterm.on` supports `window_created`, `window_closed`, `tab_created`, `tab_closed`, `pane_created`, `pane_closed`, `pane_focused`, `title_changed`, `cwd_changed`, `bell`, and `workspace_changed`, in addition to the startup and reload events. `Toyoterm::Event` exposes `name`, `workspace`, `window`, `tab`, `pane`, `title`, and `cwd`; fields unrelated to an event are `nil`. Closed-object events retain the deleted object's typed ID, but dereferencing its state raises `Toyoterm::InvalidHandleError`. `cwd_changed` is generated from OSC 7 `file://` notifications emitted by the shell.

Native producers append events to one FIFO queue and never call mruby directly. Each callback runs to completion, then its queued commands are applied before the next event. Events caused by those commands are appended to the queue, preventing recursive callback entry. Delivery is limited to 1,024 events per application turn to stop self-generating event loops. Events without registered handlers are discarded before any Ruby VM call.

An optional status bar can be configured with `Toyoterm.status(interval: 1.0)`. The callback receives a context exposing the current `workspace`, `window`, `tab`, and `pane`, and its return value is converted to text. The bar is hidden when no callback is configured. Intervals shorter than 100 ms are rejected, and callbacks run on the script worker so slow status generation does not block terminal rendering. Commands queued by a status callback are discarded.

```ruby
Toyoterm.status(interval: 1.0) do |context|
  [context.workspace.name, context.pane.cwd].compact.join(" | ")
end
```

### Hot reload

`Toyoterm.reload_config` reloads the same file selected at startup. The new source is evaluated and validated in a fresh mruby VM before it replaces the active configuration. A successful reload updates colors, font metrics, opacity, scrollback, key bindings, and event handlers without replacing the running terminal session.

Configuration errors include the source filename, line number, and Ruby backtrace. The previous configuration remains active when a reload fails.

GUI configuration failures open a non-fatal error banner. `Open Log` expands the complete diagnostic and `Dismiss` closes the banner. A broken startup configuration falls back to defaults while retaining its path for a later reload.

Changing `default_shell` does not replace the shell that is already running; it applies when a new terminal session is created.

From the Ruby Console or `toyoterm ruby console`, `Toyoterm.configure` can change settings without reloading the config file. Settings such as `font.family`, `font.fallback`, `font.size`, `font.weight`, `colors`, `window.opacity`, `scrollback_lines`, and `leader` are validated after evaluation and applied immediately to the current window, renderer, and terminals. Invalid values roll back as one transaction.

```ruby
Toyoterm.configure do |config|
  config.font.size = 16
  config.font.family = "JetBrains Mono"
  config.window.opacity = 0.9
end
```

Executable configurations are available at `examples/minimal_config.rb`; `examples/default_config.rb` contains the former standard GUI bindings as ordinary Ruby configuration.

The embedded runtime is mruby, not CRuby. CRuby gems, native extensions, and the complete CRuby standard library are not available unless toyoterm explicitly bundles them. `mruby-time` is not bundled in v0.1 because the current configuration and event APIs do not require it.

### Logging

Diagnostics are written to stderr through `tracing`; the default level is `warn`. `TOYOTERM_LOG` sets the global level or comma-separated target filters. The available targets are `toyoterm::pty`, `toyoterm::render`, `toyoterm::mux`, `toyoterm::script`, `toyoterm::config`, and `toyoterm::app`. Short target names such as `pty` are accepted.

Dynamic key-binding and event callback durations are emitted at `debug` under `toyoterm::script`. Callbacks taking 100 ms or longer are logged at `warn` as slow callbacks, including their kind, name, duration, and success state.

```sh
TOYOTERM_LOG=debug toyoterm
TOYOTERM_LOG=warn,pty=trace,render=debug toyoterm
```

v0.1 writes logs only to stderr and does not create or rotate log files. Redirecting stderr is an explicit user choice, so retention and rotation then belong to the surrounding process manager. Logs never intentionally include PTY input/output, clipboard contents, or configuration source text. Diagnostics can include configuration paths, process and pane identifiers, callback names, dimensions, error messages, and Ruby backtraces; review them before sharing.

## Controls

There are no built-in GUI key bindings. Copy the bindings from
`examples/default_config.rb` into your `config.rb` and change them as needed.

- Type normally to send input to the PTY
- Click a workspace or tab label to activate it
- Drag with the left mouse button: select text
- Mouse wheel: scroll through history, or report wheel input when the terminal application requests mouse reporting
- Control+click on Linux/Windows or Command+click on macOS: open an OSC 8 or detected web/mail link after scheme validation

When a shell exits, toyoterm closes its pane automatically. Empty tabs and workspaces are collapsed, and exiting the final pane closes toyoterm. A pane is retained after a PTY read error so the failure remains visible for diagnosis.

### Clipboard security

OSC 52 clipboard access is disabled in v0.1. Terminal output may originate from an untrusted local process or remote host, so allowing OSC 52 would let it write the host clipboard without an explicit user gesture; clipboard query responses could also expose clipboard contents. Configured copy and paste shortcuts and the trusted-configuration Ruby API remain available. Future OSC 52 support must be opt-in, keep clipboard reads disabled by default, and provide an explicit permission or confirmation UI with a payload size limit.

## CLI

```text
toyoterm [--config PATH]
toyoterm gui [--config PATH]
toyoterm list
toyoterm reload
toyoterm ruby console
toyoterm cli list-panes
toyoterm cli send-text --pane ID TEXT
toyoterm cli split [left|right|up|down]
toyoterm cli activate-workspace NAME
toyoterm demo
toyoterm pty-demo
toyoterm screen-demo
toyoterm version
toyoterm help
```

Except for the local `demo` commands, these commands connect to a running GUI over a Unix domain socket or Windows named pipe. `list` reports its live mux state; the `cli` mutations use the same native command model as Ruby. If multiple GUIs are running, the most recently started one is selected. Set the same `TOYOTERM_INSTANCE=name` when starting the GUI and invoking a client to address a stable named instance.

The IPC state directory and Unix socket are owner-only. Each request also carries a random per-instance token and a protocol version. See [Local IPC design](docs/ipc.md) for the protocol and security boundaries.

## Security

Configuration is trusted Ruby code evaluated inside the embedded mruby runtime. Plugins are third-party arbitrary code and receive the same authority as configuration. toyoterm does not currently provide a sandbox or capability restrictions for either. Review plugin source and its update channel before installing it, and only load configuration and plugins obtained from sources you trust.

## Development

Run the test suite and static checks:

```sh
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
sh scripts/check-licenses.sh
```

Create a release archive under `dist/`:

```sh
sh scripts/package.sh
```

Linux produces a `.tar.gz`, macOS an unsigned `.app` in both `.tar.gz` and DMG
formats, and Windows a portable `.zip` with an optional per-user installer.
Archives are verified by installing and running their packaged binary, and
SHA-256 sidecars are written next to them. See the
[packaging guide](docs/packaging.md), [release checklist](docs/releasing.md), and
[platform validation guide](docs/platform-validation.md).

## Architecture

```text
winit events
    ├─ native key binding resolver ─> mruby callback ─> native Command
    └─ terminal key encoder
                                      ↓
                                  native Mux
                                      ↓
                                     PTY
                                      ↓
                              alacritty_terminal
                                      ↓
                                wgpu + glyphon
```

The embedded mruby VM is single-threaded. Ruby callbacks enqueue native commands instead of mutating terminal or mux internals directly.

## License

toyoterm is distributed under the [MIT License](LICENSE).

The repository vendors the official mruby 4.0.0 amalgamation under its MIT license. See [Third-Party Notices](THIRD_PARTY_NOTICES.md) and [the preserved mruby license](vendor/mruby/LICENSE) for details. Rust dependency licenses are checked with `cargo-deny` in CI.
