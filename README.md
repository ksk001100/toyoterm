# toyoterm

<p align="center">
  <img src="packaging/app-icon.png" alt="toyoterm icon" width="180">
</p>

[日本語](README.ja.md)

toyoterm is an experimental, programmable terminal emulator powered by Rust and embedded Steel. Its terminal hot path stays native, while Scheme is used for configuration, dynamic key bindings, runtime events, and commands.

This is a personal project built for my own use and an experimental toy.

> [!IMPORTANT]
> toyoterm is under active development. GUI workspaces, tabs, and split panes have independent PTY and terminal sessions. Each GUI process has one OS window; running `toyoterm` again starts another process and window. Multiple OS windows within one process are intentionally deferred until after the initial release.

## Features

- Native PTY sessions, VT parsing with `alacritty_terminal`, and GPU rendering with `wgpu` and `glyphon`
- Workspaces, tabs, split panes, pane zoom, and independent shell sessions
- Block and underline cursors span both cells when positioned on a wide character; block cursors keep underlying text readable with an automatic contrasting color unless a cursor-text color is explicitly set
- UTF-8 and IME input, Kitty keyboard protocol progressive enhancements, mouse tracking and SGR mouse reporting, bounded OSC 50 runtime font-family switching, Kitty OSC 66 cell-spanning and fractionally scaled text, scrollback that returns to the bottom on input, search, selection, and clipboard copy/paste
- OSC titles (including separate icon-title metadata), palette/dynamic/special-attribute color controls and color stacks (including iTerm2 session, ANSI, link, selection, cursor-text, underline, visual-bell, transparent-background, Display P3, and registered Scheme theme presets), OSC 8 links, tab colors, bounded pane badges, full iTerm2 progress states and session-status indicators, opt-in bounded clipboard copies, URL launches, focus requests, policy-aware desktop notifications with standard sounds, named and cached transmitted icons, and response reports on every desktop platform, attention hints (including cursor fireworks), plain-URL detection, and shell integration for cwd, command-status markers and captured-output cleanup, remote-host/version context, prompt navigation, and iTerm2 mark navigation ([support matrix](docs/osc-support.md))
- Inline images on Linux, macOS, and Windows using 7/8-bit Sixel, Kitty graphics (including Unicode placeholders used by TUI frameworks), and iTerm2 OSC 1337, plus opt-in bounded iTerm2/Kitty file downloads and fixed-root Kitty uploads; child sessions discard stale outer-terminal capability hints so TUI image libraries can detect toyoterm correctly ([supported subset and example](docs/image-protocols.md))
- Embedded Steel 0.8.3 configuration, native and Scheme key bindings, events, commands, local Scheme libraries, themes, and searchable selection overlays
- Atomic configuration reload, a live Scheme console, local IPC, and configurable window bars and wallpaper, including opt-in fixed-root OSC wallpaper changes
- Grouped `osc-set!` permissions; assigning an absolute directory enables downloads, uploads, or OSC wallpaper changes without a separate permission flag ([configuration reference](docs/steel-api.md#settings))

## Current status

Linux is the primary development platform. CI runs builds, tests, packaging,
and GUI startup smoke tests on Linux, macOS, and Windows. Core behavior has also
been exercised on physical machines running each OS. The release candidate still
needs the documented checks; see [platform validation](docs/platform-validation.md).
CI and license compliance checks run on pull requests and pushes to `master`;
pushes to working branches do not trigger these workflows.
Multiple OS windows within one GUI process and session persistence remain outside
the initial release scope. Scheme window handles represent mux windows inside
that GUI process's single OS window.

## Build and run

Use a recent stable Rust toolchain, and the platform libraries required by `winit`/`wgpu`. Linux needs
a working Wayland or X11 session and the corresponding development libraries
(including xkbcommon and `pkg-config`). Windows rendering requires DirectX 12.

```sh
cargo run --locked
```

Build optimized binaries with `cargo build --release --locked`. Run
`target/release/toyoterm` on Linux/macOS. On Windows, use
`target/release/toyoterm-gui.exe` for a console-free GUI launch and
`target/release/toyoterm.exe` for CLI commands.

For release artifacts, Linux provides an archive with `install.sh`, macOS a DMG
or app-bundle archive, and Windows a per-user MSI installer plus a portable zip.
The Windows Start Menu shortcut opens toyoterm in the user's home directory.
Windows MSI metadata uses the numeric Cargo version (for example, `0.2.0-dev`
becomes `0.2.0`); executables and artifact filenames keep the full version.
See [installation, upgrade, uninstall, and checksums](docs/packaging.md).
Release packages include the linked guides and runnable examples.
The macOS app is unsigned; first launch may require the manual approval steps
in the [installation guide](docs/packaging.md#macos). If macOS reports that the
app is damaged after installing from the DMG, verify the download's checksum
and, if you trust it, run this in Terminal before launching it again:

```sh
xattr -dr com.apple.quarantine /Applications/toyoterm.app
```

## Configuration

Save a Scheme configuration and start it with `toyoterm --config /path/to/config.scm`.
There are no built-in GUI key bindings.

```scheme
(config-set! 'font.family "monospace")
(config-set! 'font.size 14)
(bind-action! "CTRL+SHIFT+T" 'new-tab)
(bind-action! "CTRL+SHIFT+E" 'split 'right)
(bind-action! "CTRL+SHIFT+R" 'reload-config)
```

Start with [minimal_config.scm](examples/minimal_config.scm) or
[default_config.scm](examples/default_config.scm). Configuration selection uses
`--config`, then `TOYOTERM_CONFIG_FILE`, then the platform's `config.scm` path.
See the [Steel API reference](docs/steel-api.md) for settings and validation.

Reload atomically with `toyoterm reload`. Use `toyoterm steel console` for
persistent interactive Scheme evaluation, including multiline definitions.
Windows console modes are restored on exit. Commands inside a pane connect to
that pane's GUI, including when several GUI instances are running.

Configuration and Scheme libraries are trusted local code with the process's
filesystem, process, environment and clipboard authority. They are not sandboxed.
Steel provides Scheme functions, lexical closures, macros, collections, modules,
and standard filesystem APIs. Load libraries explicitly with `(require "lib.scm")`;
relative imports resolve beside their declaring source. Ruby source is not supported.

`command!`, `on!` and `bind!` register removable callbacks. Static `bind-action!`
bindings run natively. Callbacks receive immutable object snapshots and queue typed
native commands. `context-action!` preserves the originating target. `spawn`
captures child output; `async!` executes outside both the script and GUI threads.
Task handles retain completed results without a global completed-task registry.
`bar!`, `widget` and `async-widget` provide independently refreshed status widgets.
Async widget working directories can follow the current pane through a context
procedure. Empty widget strings are omitted when joining section values.
Themes, selection overlays, OSC permissions, wallpaper and live configuration
remain available through the [Steel API](docs/steel-api.md).

## Usage and documentation

- [Usage guide](docs/usage.md): mouse controls, CLI, logs, and troubleshooting
- [Steel API reference](docs/steel-api.md): settings, bindings, callbacks, Scheme libraries, and themes
- [Shell integration](docs/shell-integration.md): cwd, remote-host context, command lifecycle reporting, and prompt navigation
- [Documentation index](docs/README.md): all user and developer guides

Type normally to send input to the shell, click tabs/workspaces to activate
them, and drag to select text. Control+click (Command+click on macOS) opens
allowed web/mail links. Exiting the final pane closes the application.

## Development

The [terminal fuzzing guide](docs/fuzzing.md) covers six cargo-fuzz targets,
seed replay in normal tests, CI smoke runs, hourly campaigns and crash regression.

The opt-in [end-to-end performance baseline](docs/performance-baseline.md) measures
the real PTY-to-frame path, pane scaling, images, search, Scheme overhead and config
reload. It saves JSON results for commit comparisons without CI timing thresholds.
For cross-terminal output throughput, run `scripts/terminal-throughput.py` inside
each terminal; the same guide explains its DSR acknowledgement metric and limits.
Ordinary isolated glyphs share up to 512 shaped buffers per pane, avoiding repeated
font fallback work for CJK and emoji while preserving grid positions and selection colors.

Run the opt-in short memory soak with `python3 scripts/soak-test.py headless --output .soak-results/short`.
The [soak validation guide](docs/soak-validation.md) also covers real GUI churn,
eight-hour idle runs, OS memory/GPU measurements and sleep/wake checks for RCs.
Normal tests do not run long soaks.

On Windows, the DX12 renderer limits its initial resource descriptor heap to
65,536 entries rather than wgpu's default million entries. This reduces startup
CPU/GPU allocations without changing fonts, wallpaper, transparency, or history.
Actual working set still depends on the graphics driver and loaded content; use
a release build and report CPU and GPU memory separately.

The generic `monospace` family resolves to an installed font before shaping,
avoiding an eager scan that retains unrelated monospaced fonts. Configured
fallbacks keep their priority, followed by the platform's script-specific fonts.
Native settings and registries use typed Steel calls without compiling per-field
source. Steel's allocation-driven GC stays on the script thread; requests and
reloads do not force full collection, which expands Steel 0.8.3's free list.
Unchanged settings, themes and registrations reuse the validated native snapshot
instead of being transferred again on each widget update.
To measure your own configuration, use the soak runner's `gui --scenario idle
--config /absolute/path/config.scm` option; see the
[memory investigation](docs/performance-baselines/2026-10-13-memory.md) for evidence
and the macOS comparison procedure.
The [Steel optimization measurements](docs/performance-baselines/2026-10-31-steel-optimization.md)
compare forced-GC removal and validated snapshot reuse with the personal config.

Unfocused windows fade slightly towards gray across terminal text, bars, and
images. Focusing the window restores the original colors; configured transparency
is preserved.

Large compressible wallpapers keep losslessly compressed pixels in CPU memory
and expand them in bounded row chunks directly into one GPU upload buffer,
including after device recovery, without a full decoded CPU image. Resolution,
colors, and PNG alpha are preserved. Small or poorly compressible images retain
raw pixels; normal redraws reuse the uploaded texture.

The [Unicode / IME / rendering regression guide](docs/unicode-rendering-validation.md)
lists automated cases and the developer fixture for manual font and IME checks.

See the [development guide](docs/development.md) for locked validation commands
and native smoke tests, including opt-in terminal performance measurements, and
the [release checklist](docs/releasing.md) for packaging.
The [crate architecture](docs/architecture.md) and [threading contract](docs/threading.md)
describe native ownership and the dedicated script thread. Static bindings
bypass Scheme; Scheme callbacks return commands for the main thread to apply.

## License

toyoterm is distributed under the [MIT License](LICENSE).

Steel is a Rust dependency pinned in Cargo.lock. See [Third-Party Notices](THIRD_PARTY_NOTICES.md). Rust dependency licenses are checked with `cargo-deny` in CI.
