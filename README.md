# toyoterm

[日本語](README.ja.md)

toyoterm is an experimental, programmable terminal emulator powered by Rust and embedded mruby. Its terminal hot path stays native, while Ruby is used for configuration, dynamic key bindings, runtime events, and commands.

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
- `app_started` and `config_reloaded` Ruby events
- Native command and mux model for tabs, pane splits, and workspaces
- GUI tabs with one PTY and terminal backend per pane
- Rendered split panes with per-pane resize and focus
- A clickable tab bar with keyboard tab navigation
- A clickable workspace bar with per-workspace focus restoration
- A fuzzy-search command palette and user-defined Ruby commands
- A live Ruby REPL connected to the running GUI's single mruby VM

## Current status

The primary development environment is Linux. The architecture and dependencies are cross-platform, but macOS and Windows support has not yet been fully validated.

Not yet exposed in the GUI:

- Multiple OS windows
- Search, links, image protocols, and session persistence

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

Use an explicit configuration file:

```sh
cargo run -- --config /path/to/config.rb
```

Connect a live Ruby REPL to the running GUI from another terminal. It supports multiline input, `:history`, and `exit`.

```sh
cargo run -- ruby console
```

Open the Command Palette with `Ctrl+Shift+P` (`Cmd+Shift+P` on macOS), or the Commands button in the top-right corner.

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
  end
end

Toyoterm.on :app_started do |event|
  event.pane.send_text("echo toyoterm started\n")
end

Toyoterm.on :config_reloaded do |event|
  event.pane.send_text("echo config reloaded\n")
end
```

`font.fallback` is optional. Installed families are tried in the listed order for missing CJK, emoji, symbol, and other glyphs, followed by the platform defaults. Unknown families are skipped by the font system.

### Key bindings

Key names are case-insensitive. Modifiers use names such as `CTRL`, `SHIFT`, `ALT`, and `SUPER`. Named keys include `ENTER`, `TAB`, `SPACE`, arrow keys, navigation keys, and `F1` through `F12`.

`config.keys` provides `ctrl`, `ctrl_shift`, `primary`, `primary_shift`, `alt`, `super_key`, `leader`, and `physical` helpers. `primary` resolves to `SUPER` on macOS and `CTRL` on Linux/Windows, so one configuration can follow each platform's conventions. Modifier names are portable: `ALT` is the Option key on macOS, while `SUPER` is Command on macOS and the Windows key on Windows. The `physical` helper distinguishes a hardware position from the logical character, for example `physical("KeyH", "CTRL")`. When both match, physical bindings take priority over logical bindings. User-configured bindings take priority over built-in GUI shortcuts. Defining the same chord more than once is a configuration error.

`config.leader` defines a native leader prefix with a timeout in milliseconds. `leader("v")` bindings are resolved without invoking mruby. The leader prefix is discarded; an unmatched or expired suffix continues through normal key handling. Leader state is cleared by repeat events, IME activity, focus loss, and configuration reload.

Unmatched keys bypass mruby and go directly through the native terminal key encoder. If a Ruby callback raises an exception, toyoterm logs the error and keeps the shell running.

Ruby callbacks can access the host text clipboard through `Toyoterm.clipboard.read` and `Toyoterm.clipboard.write(text)`. The clipboard snapshot is refreshed immediately before each dynamic key-binding or event callback. `read` raises `RuntimeError` when the platform clipboard is unavailable. Writes are applied only after the callback completes successfully, so a callback exception rolls them back together with its other queued commands.

```ruby
config.bind "CTRL+SHIFT+Y" do
  Toyoterm.clipboard.write("pane #{Toyoterm.current_pane.id}")
end

config.bind "CTRL+SHIFT+P" do |context|
  context.pane.send_text(Toyoterm.clipboard.read)
end
```

### Ruby object model

Each callback receives a current snapshot through `Toyoterm.current_workspace`, `current_window`, `current_tab`, and `current_pane`. `Toyoterm.workspaces`, `windows`, and `workspace(name)` provide lookup; workspace, window, and tab objects expose their children. Pane metadata includes `title`, `cwd`, and `pid`. Mutating methods such as `split`, `close`, `focus`/`activate`, `new_tab`, and `create_window` enqueue native commands and take effect after the callback returns successfully. A saved object raises `Toyoterm::InvalidHandleError` after its native object is deleted.

`pane.badge` is callback-owned display metadata stored by pane ID for the lifetime of the current Ruby VM. Rendering badges is intentionally separate from this API contract. `pane.chdir` is not provided: the shell owns its working directory, so configurations that want shell-specific directory changes should use `pane.send_text("cd ...\n")` with appropriate shell escaping.

### Runtime events

`Toyoterm.on` supports `window_created`, `window_closed`, `tab_created`, `tab_closed`, `pane_created`, `pane_closed`, `pane_focused`, `title_changed`, `cwd_changed`, `bell`, and `workspace_changed`, in addition to the startup and reload events. `Toyoterm::Event` exposes `name`, `workspace`, `window`, `tab`, `pane`, `title`, and `cwd`; fields unrelated to an event are `nil`. Closed-object events retain the deleted object's typed ID, but dereferencing its state raises `Toyoterm::InvalidHandleError`. `cwd_changed` is generated from OSC 7 `file://` notifications emitted by the shell.

Native producers append events to one FIFO queue and never call mruby directly. Each callback runs to completion, then its queued commands are applied before the next event. Events caused by those commands are appended to the queue, preventing recursive callback entry. Delivery is limited to 1,024 events per application turn to stop self-generating event loops. Events without registered handlers are discarded before any Ruby VM call.

### Hot reload

`Toyoterm.reload_config` reloads the same file selected at startup. The new source is evaluated and validated in a fresh mruby VM before it replaces the active configuration. A successful reload updates colors, font metrics, opacity, scrollback, key bindings, and event handlers without replacing the running terminal session.

Configuration errors include the source filename, line number, and Ruby backtrace. The previous configuration remains active when a reload fails.

GUI configuration failures open a non-fatal error banner. `Open Log` expands the complete diagnostic, `Open Ruby Console` explains the current console availability, and `Dismiss` closes the banner. A broken startup configuration falls back to defaults while retaining its path for a later reload.

Changing `default_shell` does not replace the shell that is already running; it applies when a new terminal session is created.

An executable starter configuration is available at `examples/minimal_config.rb` and can be tested with `toyoterm --config examples/minimal_config.rb`.

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

- Type normally to send input to the PTY
- `Ctrl+Shift+C` on Linux/Windows or `Cmd+C` on macOS: copy the selection
- `Ctrl+Shift+V` on Linux/Windows or `Cmd+V` on macOS: paste
- `Ctrl+Shift+T` on Linux/Windows or `Cmd+T` on macOS: open a new tab
- `Ctrl+Shift+R` on Linux/Windows or `Cmd+Shift+R` on macOS: reload the active configuration file
- Click `Commands` → `Reload Config`: reload the active configuration file from the GUI
- `Ctrl+Shift+W` on Linux/Windows or `Cmd+W` on macOS: close the active tab (the final tab is kept open)
- `Ctrl+Tab` / `Ctrl+Shift+Tab`: activate the next / previous tab
- `Ctrl+Shift+\` / `Ctrl+Shift+-` on Linux/Windows or `Cmd+D` / `Cmd+Shift+D` on macOS: split the active pane right / down
- `Ctrl+Shift+Arrow` on Linux/Windows or `Cmd+Option+Arrow` on macOS: focus the nearest pane in that direction
- `Ctrl+Shift+Q` on Linux/Windows or `Cmd+Shift+W` on macOS: close the active pane (the final pane is kept open)
- `Ctrl+Shift+N` on Linux/Windows or `Cmd+N` on macOS: create and activate a workspace
- `Ctrl+Alt+Left` / `Ctrl+Alt+Right`: activate the previous / next workspace
- Click a workspace or tab label to activate it
- Drag with the left mouse button: select text
- Mouse wheel: scroll through history, or report wheel input when the terminal application requests mouse reporting

When a shell exits, toyoterm closes its pane automatically. Empty tabs and workspaces are collapsed, and exiting the final pane closes toyoterm. A pane is retained after a PTY read error so the failure remains visible for diagnosis.

### Clipboard security

OSC 52 clipboard access is disabled in v0.1. Terminal output may originate from an untrusted local process or remote host, so allowing OSC 52 would let it write the host clipboard without an explicit user gesture; clipboard query responses could also expose clipboard contents. The built-in copy and paste shortcuts and the trusted-configuration Ruby API remain available. Future OSC 52 support must be opt-in, keep clipboard reads disabled by default, and provide an explicit permission or confirmation UI with a payload size limit.

## CLI

```text
toyoterm [--config PATH]
toyoterm gui [--config PATH]
toyoterm list
toyoterm demo
toyoterm pty-demo
toyoterm screen-demo
toyoterm version
toyoterm help
```

The `list` and `demo` commands currently exercise the in-memory mux model; they do not inspect or control an already-running GUI instance.

## Security

Configuration is trusted Ruby code evaluated inside the embedded mruby runtime. toyoterm does not currently provide a sandbox or capability restrictions for configuration and future plugins. Only load configuration obtained from sources you trust.

## Development

Run the test suite and static checks:

```sh
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo fmt --check
sh scripts/check-licenses.sh
```

Create a release archive under `dist/`:

```sh
sh scripts/package.sh
```

The archive contains the toyoterm binary, the project license, third-party notices, and the mruby license.

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
