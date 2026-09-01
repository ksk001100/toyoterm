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

## Current status

The primary development environment is Linux. The architecture and dependencies are cross-platform, but macOS and Windows support has not yet been fully validated.

Not yet exposed in the GUI:

- Multiple OS windows
- Live Ruby REPL and remote-control CLI
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

  # Set an explicit shell when needed. Otherwise the platform default is used.
  # config.default_shell = "/bin/zsh"

  config.bind "CTRL+SHIFT+H" do |context|
    context.pane.send_text("echo hello from mruby\n")
  end

  config.bind "CTRL+SHIFT+R" do
    Toyoterm.reload_config
  end

  # Common actions compile to native bindings and do not invoke mruby on key press.
  config.keys do
    ctrl_shift("e").split(:right)
    ctrl_shift("o").activate_pane(:right)
    ctrl_shift("t").new_tab
  end
end

Toyoterm.on :app_started do |event|
  event.pane.send_text("echo toyoterm started\n")
end

Toyoterm.on :config_reloaded do |event|
  event.pane.send_text("echo config reloaded\n")
end
```

### Key bindings

Key names are case-insensitive. Modifiers use names such as `CTRL`, `SHIFT`, `ALT`, and `SUPER`. Named keys include `ENTER`, `TAB`, `SPACE`, arrow keys, navigation keys, and `F1` through `F12`.

`config.keys` provides `ctrl`, `ctrl_shift`, `alt`, `super_key`, and `physical` helpers. The `physical` helper distinguishes a hardware position from the logical character, for example `physical("KeyH", "CTRL")`. When both match, physical bindings take priority over logical bindings. User-configured bindings take priority over built-in GUI shortcuts. Defining the same chord more than once is a configuration error.

Unmatched keys bypass mruby and go directly through the native terminal key encoder. If a Ruby callback raises an exception, toyoterm logs the error and keeps the shell running.

### Hot reload

`Toyoterm.reload_config` reloads the same file selected at startup. The new source is evaluated and validated in a fresh mruby VM before it replaces the active configuration. A successful reload updates colors, font metrics, opacity, scrollback, key bindings, and event handlers without replacing the running terminal session.

Changing `default_shell` does not replace the shell that is already running; it applies when a new terminal session is created.

An executable starter configuration is available at `examples/minimal_config.rb` and can be tested with `toyoterm --config examples/minimal_config.rb`.

The embedded runtime is mruby, not CRuby. CRuby gems, native extensions, and the complete CRuby standard library are not available unless toyoterm explicitly bundles them. `mruby-time` is not bundled in v0.1 because the current configuration and event APIs do not require it.

## Controls

- Type normally to send input to the PTY
- `Ctrl+Shift+C` on Linux/Windows or `Cmd+C` on macOS: copy the selection
- `Ctrl+Shift+V` on Linux/Windows or `Cmd+V` on macOS: paste
- `Ctrl+Shift+T`: open a new tab
- `Ctrl+Shift+W`: close the active tab (the final tab is kept open)
- `Ctrl+Tab` / `Ctrl+Shift+Tab`: activate the next / previous tab
- `Ctrl+Shift+\` / `Ctrl+Shift+-`: split the active pane right / down
- `Ctrl+Shift+Arrow`: focus the nearest pane in that direction
- `Ctrl+Shift+Q`: close the active pane (the final pane is kept open)
- `Ctrl+Shift+N`: create and activate a workspace
- `Ctrl+Alt+Left` / `Ctrl+Alt+Right`: activate the previous / next workspace
- Click a workspace or tab label to activate it
- Drag with the left mouse button: select text
- Mouse wheel: scroll through history, or report wheel input when the terminal application requests mouse reporting

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
