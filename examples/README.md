# Extension recipes

[日本語](README.ja.md)

Each scenario has a directory, a runnable `config.rb`, and a guide. From the
repository root (or use an absolute config path):

```sh
cargo run --locked -- --config examples/command_forge/config.rb
```

With an installed binary, use `toyoterm --config examples/command_forge/config.rb`.

For Neovim configuration rather than Ruby recipes, see the
[LazyVim / Snacks image integration](neovim/README.md).

## Build your own terminal behaviour with Ruby

| Recipe | Experience | Try it |
| --- | --- | --- |
| [Command Forge](command_forge/README.md) | Data -> generated Ruby methods -> commands, keys and help; shared hooks decorate every generated command | Alt+1/2/3, Alt+I to inspect |
| [Project modes](project_modes/README.md) | Per-Pane major modes turn the same key into Rust or Node tasks according to cwd | Alt+Enter, Alt+H to explain |
| [Output lenses](output_lenses/README.md) | Generated Method objects and independent providers turn visible terminal output into searchable/copyable results | Alt+L |
| [Status bar](status_bar/README.md) | Workspace, cwd, command status, async Git branch, clock and failure badges | Run commands and change directory |
| [Dynamic theme](dynamic_theme/README.md) | Day/night colors following local time with a manual override | Alt+T |
| [Pane workbench](pane_workbench/README.md) | Same-directory shells and project tabs, cross-workspace Pane finder | Alt+W, Alt+N, Alt+P |
| [Command journal](command_journal/README.md) | Exit statuses and elapsed times in a searchable panel | Alt+J, Alt+K |

## Combined API workbenches

Related former single-file examples now share entry points. Components remain
separately loadable when you want to study one API.

| Directory | Combined examples | Entry point / controls |
| --- | --- | --- |
| [Starter](starter/README.md) | Full default and minimal settings (alternatives) | `starter/config.rb` or `starter/minimal.rb` |
| [Commands](commands/README.md) | Palette, help, typed arguments, directory completion | `commands/config.rb`; Alt+X/H/L/P |
| [Pane tools](pane_tools/README.md) | Navigation mode, transient maps, action advice, grid events | `pane_tools/config.rb`; Alt+N/S/R/M |
| [Monitoring](monitoring/README.md) | Live panel, Pane-local clock, reminder | `monitoring/config.rb`; Alt+M/T/R |
| [Screen analysis](screen_analysis/README.md) | Cooperative counting and instruction-bounded summary | `screen_analysis/config.rb`; Alt+W/I |
| [Extension libraries](extension_libraries/README.md) | Independent hook providers/policies/observers and scope-owned cleanup | `extension_libraries/config.rb`; Alt+P/S/M |
| [Performance](performance/README.md) | Native vs Ruby key-to-frame fixture | `performance/config.rb`; Ctrl+Shift+F11/F12 |

The old `theme_selector.rb` is integrated into `dynamic_theme/config.rb`, which
also registers usable themes. The former `async_bar.rb` is the standalone
[network bar variant](status_bar/network.rb), launched with
`--config examples/status_bar/network.rb`; it uses `ping` and `curl`, contacts
1.1.1.1 and wttr.in, and is an alternative to the offline/default status bar.
The Python rendering/image probes remain in this directory as validation tools.

## Controls and adaptation

All `config.rb` workbenches except starter/performance use
[shared navigation](shared/keys.rb): Ctrl+Shift+T creates a tab,
Ctrl+Tab / Ctrl+Shift+Tab changes tabs, Ctrl+Shift+E / D splits right/down,
Ctrl+Shift+arrows moves between Panes, Ctrl+Shift+Z toggles zoom, and
Ctrl+Shift+R reloads. Copy/paste uses Ctrl+Shift+C/V (Command+Shift+C/V on macOS).
Panels/selectors capture input; Escape closes them. Modes can temporarily
replace bindings. Alt+W in the Pane workbench adds two shells on every press.

Enable [shell integration](../docs/shell-integration.md) for cwd, command status,
the journal and automatic project modes. No gem, icon font or specific shell is
required. Project-mode tasks require Cargo/npm and execute only on request.
Git is optional for the default status bar. The network variant uses external
services; the other recipes make no network requests themselves. Some starter
and command-palette actions deliberately send shell text: inspect their code
before using them in a busy shell.

To adapt a recipe, explicitly `require_relative` its components, reconcile keys,
and combine widgets into one `window.bar :bottom` block. Duplicate bars are
rejected. Copy required sibling files and `shared/` too. Successful reload
replaces registrations and resets in-memory state; failed reload preserves the
previous config. Ruby objects and external effects are not rolled back.
See the [Ruby API](../docs/mruby-api.md) for callback and snapshot rules.
