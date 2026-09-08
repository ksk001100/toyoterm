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
- UTF-8 and IME input, scrollback, search, selection, and clipboard copy/paste
- OSC 8 links, plain-URL detection, and shell integration for cwd and command status
- Inline images using Sixel, Kitty graphics, and iTerm2 OSC 1337 ([supported subset and example](docs/image-protocols.md))
- Embedded mruby 4.0 configuration, native and Ruby key bindings, events, commands, plugins, and themes
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
Reload with `toyoterm reload`; use `toyoterm ruby console` for live Ruby updates.

Configuration and plugins are trusted code with filesystem, process,
environment, and clipboard access. They are not sandboxed. The embedded runtime
is mruby, so the complete CRuby standard library and gems are not available.

## Usage and documentation

- [Usage guide](docs/usage.md): mouse controls, CLI, logs, and troubleshooting
- [mruby API reference](docs/mruby-api.md): settings, bindings, callbacks, plugins, and themes
- [Shell integration](docs/shell-integration.md): cwd and command lifecycle reporting
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
