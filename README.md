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

- Native PTY sessions, VT parsing with `alacritty_terminal`, and GPU rendering with `wgpu` and `glyphon`
- Workspaces, tabs, split panes, pane zoom, and independent shell sessions
- UTF-8 and IME input, scrollback that returns to the bottom on input, search, selection, and clipboard copy/paste
- OSC titles (including separate icon-title metadata), palette/dynamic color controls and color stacks (including iTerm2 session, ANSI, link, selection, cursor-text, underline, and Display P3 colors), OSC 8 links, tab colors, bounded pane badges, progress and session-status indicators, opt-in bounded clipboard copies, URL launches, policy-aware desktop notifications and attention hints, plain-URL detection, and shell integration for cwd, command-status markers, remote-host/version context, prompt navigation, and iTerm2 mark navigation ([support matrix](docs/osc-support.md))
- Inline images on Linux, macOS, and Windows using 7/8-bit Sixel, Kitty graphics (including Unicode placeholders used by TUI frameworks), and iTerm2 OSC 1337; child sessions discard stale outer-terminal capability hints so TUI image libraries can detect toyoterm correctly ([supported subset and example](docs/image-protocols.md))
- Embedded mruby 4.0 configuration, native and Ruby key bindings, events, commands, plugins, themes, and searchable selection overlays
- Atomic configuration reload, a live Ruby console, local IPC, and configurable window bars and wallpaper

## Current status

Linux is the primary development platform. CI runs builds, tests, packaging,
and GUI startup smoke tests on Linux, macOS, and Windows. Interactive validation
on physical machines is still required; see [platform validation](docs/platform-validation.md).
Multiple OS windows and session persistence remain outside
the initial release scope. Ruby `Window` handles represent mux windows inside
the application's single OS window.

## Build and run

Use a recent stable Rust toolchain, a C compiler for the vendored mruby
amalgamation, and the platform libraries required by `winit`/`wgpu`. Linux needs
a working Wayland or X11 session and the corresponding development libraries
(including xkbcommon and `pkg-config`). Windows rendering requires DirectX 12.

```sh
cargo run --locked
```

Build an optimized binary with `cargo build --release --locked`; run
`target/release/toyoterm` (`target/release/toyoterm.exe` on Windows).

For release artifacts, Linux provides an archive with `install.sh`, macOS a DMG
or app-bundle archive, and Windows a portable zip with an optional per-user
installer. See [installation, upgrade, uninstall, and checksums](docs/packaging.md).

## Configuration

There are no built-in GUI key bindings. Save a configuration and start it with:

```sh
toyoterm --config /path/to/config.rb
```

```ruby
Toyoterm.configure do |config|
  config.font.family = "monospace"
  config.font.size = 14

  config.keys do
    ctrl_shift("t").new_tab
    ctrl_shift("e").split(:right)
    ctrl_shift("r").reload_config
    primary_shift("c").copy_selection
    primary_shift("v").paste_clipboard
  end
end
```

Use [minimal_config.rb](examples/minimal_config.rb) or
[default_config.rb](examples/default_config.rb) as a starting point. A Nerd Font
is useful for prompt icons; specify its exact installed family name.

Configuration is selected by `--config`, then `TOYOTERM_CONFIG_FILE`, then the
platform default. See [configuration loading](docs/mruby-api.md#loading-configuration)
for Linux/macOS and Windows paths and error recovery.
Reload with `toyoterm reload`; use `toyoterm ruby console` for live Ruby updates,
including multiline definitions and variables retained between entries.

Configuration and plugins are trusted code with filesystem, process,
environment, and clipboard access. They are not sandboxed. The embedded runtime
is mruby, so the complete CRuby standard library and gems are not available.
The bundled runtime includes mruby's portable standard-library, math, and
metaprogramming APIs; see the API reference for details. Platform-dependent
I/O and socket gems remain excluded in favor of toyoterm's host APIs.
External commands can be captured synchronously with `Toyoterm.spawn`, or run in
the background without blocking the script thread via `Toyoterm.async`. It
returns an `AsyncTask` that can be retained in a widget closure without global
state; pass `cwd:` to run a command in a specific working directory.
For status bars, `bar.section(...)` joins synchronous values and independent
`add_async(...)` tasks with a separator.
The scripting API exposes `Toyoterm.version`, `Toyoterm.api_version`, feature
detection, native logging, removable registrations, context-bound actions, and
a read-only configuration snapshot. Async tasks release completed results from
the VM registry and support callback/result cancellation. See the API reference
for the pre-release `find_workspace` / `open_workspace` and `MuxWindow` names.

## Usage and documentation

- [Usage guide](docs/usage.md): mouse controls, CLI, logs, and troubleshooting
- [mruby API reference](docs/mruby-api.md): settings, bindings, callbacks, plugins, and themes
- [Shell integration](docs/shell-integration.md): cwd, remote-host context, command lifecycle reporting, and prompt navigation
- [Documentation index](docs/README.md): all user and developer guides

Type normally to send input to the shell, click tabs/workspaces to activate
them, and drag to select text. Control+click (Command+click on macOS) opens
allowed web/mail links. Exiting the final pane closes the application.

## Development

See the [development guide](docs/development.md) for locked validation commands
and native smoke tests, and the [release checklist](docs/releasing.md) for packaging.
The [crate architecture](docs/architecture.md) and [threading contract](docs/threading.md)
describe native ownership and the dedicated script thread. Static bindings
bypass Ruby; Ruby callbacks return commands for the main thread to apply.

## License

toyoterm is distributed under the [MIT License](LICENSE).

The repository vendors the official mruby 4.0.0 amalgamation under its MIT license. See [Third-Party Notices](THIRD_PARTY_NOTICES.md) and [the preserved mruby license](vendor/mruby/LICENSE) for details. Rust dependency licenses are checked with `cargo-deny` in CI.
