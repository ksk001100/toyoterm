# toyoterm

<p align="center">
  <img src="packaging/app-icon.png" alt="toyoterm icon" width="180">
</p>

[日本語](README.ja.md)

toyoterm is an experimental, programmable terminal emulator powered by Rust and embedded mruby. Its terminal hot path stays native, while Ruby is used for configuration, dynamic key bindings, runtime events, and commands.

This is a personal project built for my own use and an experimental toy.

> [!IMPORTANT]
> toyoterm is under active development. GUI workspaces, tabs, and split panes have independent PTY and terminal sessions. Each GUI process has one OS window; running `toyoterm` again starts another process and window. Multiple OS windows within one process are intentionally deferred until after the initial release.

## Features

- Native PTY sessions, VT parsing with `alacritty_terminal`, and GPU rendering with `wgpu` and `glyphon`
- Workspaces, tabs, split panes, pane zoom, and independent shell sessions
- Block and underline cursors span both cells when positioned on a wide character; block cursors keep underlying text readable with an automatic contrasting color unless a cursor-text color is explicitly set
- UTF-8 and IME input, Kitty keyboard protocol progressive enhancements, mouse tracking and SGR mouse reporting, bounded OSC 50 runtime font-family switching, Kitty OSC 66 cell-spanning and fractionally scaled text, scrollback that returns to the bottom on input, search, selection, and clipboard copy/paste
- OSC titles (including separate icon-title metadata), palette/dynamic/special-attribute color controls and color stacks (including iTerm2 session, ANSI, link, selection, cursor-text, underline, visual-bell, transparent-background, Display P3, and registered Ruby theme presets), OSC 8 links, tab colors, bounded pane badges, full iTerm2 progress states and session-status indicators, opt-in bounded clipboard copies, URL launches, focus requests, policy-aware desktop notifications with standard sounds, named and cached transmitted icons, and response reports on every desktop platform, attention hints (including cursor fireworks), plain-URL detection, and shell integration for cwd, command-status markers and captured-output cleanup, remote-host/version context, prompt navigation, and iTerm2 mark navigation ([support matrix](docs/osc-support.md))
- Inline images on Linux, macOS, and Windows using 7/8-bit Sixel, Kitty graphics (including Unicode placeholders used by TUI frameworks), and iTerm2 OSC 1337, plus opt-in bounded iTerm2/Kitty file downloads and fixed-root Kitty uploads; child sessions discard stale outer-terminal capability hints so TUI image libraries can detect toyoterm correctly ([supported subset and example](docs/image-protocols.md))
- Embedded mruby 4.0 configuration, native and Ruby key bindings, events, commands, local Ruby libraries, themes, and searchable selection overlays
- Atomic configuration reload, a live Ruby console, local IPC, and configurable window bars and wallpaper, including opt-in fixed-root OSC wallpaper changes
- Grouped `config.osc` permissions; assigning an absolute directory enables downloads, uploads, or OSC wallpaper changes without a separate permission flag ([configuration reference](docs/mruby-api.md#configosc))

## Current status

Linux is the primary development platform. CI runs builds, tests, packaging,
and GUI startup smoke tests on Linux, macOS, and Windows. Core behavior has also
been exercised on physical machines running each OS. The release candidate still
needs the documented checks; see [platform validation](docs/platform-validation.md).
CI and license compliance checks run on pull requests and pushes to `master`;
pushes to working branches do not trigger these workflows.
Multiple OS windows within one GUI process and session persistence remain outside
the initial release scope. Ruby `MuxWindow` handles represent mux windows inside
that GUI process's single OS window.

## Build and run

Use a recent stable Rust toolchain, a C compiler for the vendored mruby
amalgamation, and the platform libraries required by `winit`/`wgpu`. Linux needs
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

Ruby commands support descriptions, discovery with `Toyoterm.commands` and
`Toyoterm.command_info`, and composition with `Toyoterm.call_command`.
Bind `Toyoterm.command_palette` to open a searchable command selector; see
[the runnable example](examples/command_palette.rb) and
[API semantics](docs/mruby-api.md#command-discovery-composition-and-palette).
`Toyoterm.command_help` browses descriptions, arguments, effective keys, advice,
and definition locations without invoking commands. A definition-selection
callback connects help to a library's navigation workflow. Try
[the help example](examples/command_help.rb) with Alt+H; see
[command help](docs/mruby-api.md#command-help-and-definitions).
Commands can declare typed `arguments:` with defaults, choices, ranges, history,
and completion. Key bindings and palettes collect them before execution;
`call_command(arguments: ...)` validates programmatic calls. Try the
[Pane-label example](examples/command_arguments.rb); see
[command arguments](docs/mruby-api.md#typed-command-arguments).
Libraries can publish named hooks with `Toyoterm.add_hook` / `Toyoterm.run_hook`
and adapt commands with removable before, after, and around advice through
`Toyoterm.advise_command`; see [extension hooks](docs/mruby-api.md#extension-hooks-and-command-advice).
Hook handlers expose labels, priorities and source locations through
`Toyoterm.hook_handlers`. Provider/policy pipelines can stop at the first
success or failure. Try [the independent-library example](examples/hook_composition.rb)
with Alt+P; see [loading and composing libraries](docs/extension-libraries.md).
Multi-step bindings use `config.keys.sequence("CTRL+X", "CTRL+T").new_tab`.
Inspect bindings with `Toyoterm.describe_key` / `Toyoterm.command_keys`, or
open `Toyoterm.key_help`; see [key sequences](docs/mruby-api.md#multi-step-key-sequences-and-inspection).
Named keymaps support inheritance and removable global or Pane-local activation.
Major/minor modes add scoped lifecycle callbacks and cleanup of registrations
created during those callbacks. Try [the navigation-mode example](examples/modes.rb)
with Alt+N, then h/j/k/l and Escape; see [keymaps and modes](docs/mruby-api.md#keymaps-and-modes).
`Keymap#activate_transient` temporarily captures keys for one operation or
repeated terminal commands, with scoped stacking and automatic cancellation.
Try [the split/Pane-tools example](examples/transient_keymaps.rb) with Alt+S
or Alt+R; see [transient keymaps](docs/mruby-api.md#transient-keymaps).
Extensions can collect arguments with `Toyoterm.read_string`, including dynamic
candidate providers and named history. The [completion example](examples/completion.rb)
chooses a directory and opens a terminal Pane there; see
[text input and completion](docs/mruby-api.md#text-input-and-completion).
Long prompt input scrolls around the cursor by display columns, preserving
Japanese text and emoji graphemes at narrow widths.
While an extension overlay is open, the IME candidate anchor follows its visible
input cursor, including horizontal scrolling and preedit.
`Toyoterm.show_panel` displays read-only extension output with native filtering
and paging; its handle can update or close it. Try the live terminal monitor
in [the panel example](examples/panels.rb) with Alt+M; see
[read-only panels](docs/mruby-api.md#read-only-panels).
`Toyoterm.after` / `Toyoterm.every` add cancellable delayed and repeating work.
Modes own timers and async tasks created during their lifecycle or owned task
callbacks, and release them on disable/close/reload. Try the
[clock and reminder example](examples/timers.rb); see
[timers and task ownership](docs/mruby-api.md#timers-and-task-ownership).
`pane_ready` observes successful PTY preparation and `pane_resized` reports
actual per-Pane row/column changes, including split/layout changes. Try
[the dimensions-badge example](examples/pane_events.rb); see
[runtime events](docs/mruby-api.md#runtime-events).
`Toyoterm.resource_scope` groups an extension's registrations, mode activations,
tasks and overlays for transactional cleanup without defining a mode.
`scope.wrap` gives later callbacks the same ownership. Try
[the scoped-library example](examples/resource_scopes.rb) with Alt+S/Alt+M;
see [resource scopes](docs/mruby-api.md#resource-scopes).
`Toyoterm.work_each` processes enumerable items in cooperative batches with
item/time budgets, cancellation and progress/overrun statistics. It yields only
between items. Try [the terminal word-count example](examples/cooperative_work.rb)
with Alt+W; see [cooperative work](docs/mruby-api.md#cooperative-work).
`Toyoterm.with_instruction_budget(instructions: 100_000) { ... }` limits Ruby
opcode execution in an explicit block and rolls back its host effects on error.
It does not interrupt C functions or blocking I/O. Try
[the bounded terminal-summary example](examples/instruction_budgets.rb) with Alt+I;
see [instruction budgets](docs/mruby-api.md#instruction-budgets).
Native actions expose names, argument choices, and keys through `Toyoterm.actions`,
`Toyoterm.action_info`, and `Toyoterm.action_keys`. `Toyoterm.advise_action` can wrap, change, or suppress
named actions from native bindings and Ruby. `Toyoterm.operation_palette` lists
native actions and Ruby commands together; see the [action-policy example](examples/actions.rb)
and [action API](docs/mruby-api.md#native-action-discovery-and-advice).

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
is useful for prompt icons; specify its exact installed family name. Logical
symbol bindings use the produced character, so `key("$")` matches `$` even
though typing it requires Shift on many keyboard layouts.

Configuration is selected by `--config`, then `TOYOTERM_CONFIG_FILE`, then the
platform default. See [configuration loading](docs/mruby-api.md#loading-configuration)
for Linux/macOS and Windows paths and error recovery.
Reload with `toyoterm reload`; use `toyoterm ruby console` for live Ruby updates,
including multiline definitions and variables retained between entries.
On Windows, the console enables line input while active and restores the
parent shell's input mode on exit. Empty lines keep the console open.
Commands run inside a toyoterm pane connect to that pane's GUI, even when a
shell changes its temporary-directory environment or several GUIs are open.

Configuration and required Ruby libraries are trusted code with filesystem, process,
environment, and clipboard access. They are not sandboxed. The embedded runtime
is mruby, so the complete CRuby standard library and gems are not available.
The bundled runtime includes mruby's standard-library, math, metaprogramming,
and filesystem APIs. Use ordinary Ruby `File` and `Dir` methods for filesystem
work; socket APIs remain excluded. See the API reference for details.
External commands can be captured synchronously with `Toyoterm.spawn`, or run in
the background without blocking the script thread via `Toyoterm.async`. It
returns an `AsyncTask` that can be retained in a widget closure without global
state; pass `cwd:` to run a command in a specific working directory.
For status bars, `bar.section(...)` joins values whose `add(...)` and
`add_async(...)` refresh intervals are managed independently.
The scripting API exposes `Toyoterm.version`, `Toyoterm.api_version`, feature
detection, native logging, removable registrations, context-bound actions, and
a read-only configuration snapshot. Async tasks release completed results from
the VM registry and support callback/result cancellation. See the API reference
for the pre-release `find_workspace` / `open_workspace` and `MuxWindow` names.

Load local Ruby libraries explicitly from `config.rb` with
`require "name"` or `require_relative "path"`. toyoterm searches beside the
config and in its `lib/` directory. These locations are exposed through
`$LOAD_PATH`, and required sources are evaluated immediately. Directories are
not scanned automatically. Required files use the ordinary `Toyoterm.command`,
`Toyoterm.on`, `Toyoterm.theme`, and `Toyoterm.configure` APIs.

## Usage and documentation

- [Usage guide](docs/usage.md): mouse controls, CLI, logs, and troubleshooting
- [mruby API reference](docs/mruby-api.md): settings, bindings, callbacks, Ruby libraries, and themes
- [Shell integration](docs/shell-integration.md): cwd, remote-host context, command lifecycle reporting, and prompt navigation
- [Documentation index](docs/README.md): all user and developer guides

Type normally to send input to the shell, click tabs/workspaces to activate
them, and drag to select text. Control+click (Command+click on macOS) opens
allowed web/mail links. Exiting the final pane closes the application.

## Development

`cargo run --locked -- gui-smoke-test --extension-ui` presents a Ruby-created
Japanese/emoji input prompt and read-only panel before exiting. It requires a
display/GPU stack and checks frame presentation, not OS keyboard/IME interaction.
See [development checks](docs/development.md).
Windows-hosted cross builds now emit PE icon resources only for Windows
targets. Existing Zig/Rust tools can also verify Linux linking without running
the binaries; see [cross-target validation](docs/platform-validation.md#cross-target-checks-from-windows).

The [terminal fuzzing guide](docs/fuzzing.md) covers six cargo-fuzz targets,
seed replay in normal tests, CI smoke runs, hourly campaigns and crash regression.

The opt-in [end-to-end performance baseline](docs/performance-baseline.md) measures
the real PTY-to-frame path, pane scaling, images, search, Ruby overhead and config
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
Configuration validation also releases temporary Ruby compilation objects on
the script thread, including with large theme registries.
To measure your own configuration, use the soak runner's `gui --scenario idle
--config /absolute/path/config.rb` option; see the
[memory investigation](docs/performance-baselines/2026-10-13-memory.md) for evidence
and the macOS comparison procedure.

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
bypass Ruby; Ruby callbacks return commands for the main thread to apply.

## License

toyoterm is distributed under the [MIT License](LICENSE).

The repository vendors the official mruby 4.0.0 amalgamation under its MIT license. See [Third-Party Notices](THIRD_PARTY_NOTICES.md) and [the preserved mruby license](vendor/mruby/LICENSE) for details. Rust dependency licenses are checked with `cargo-deny` in CI.
