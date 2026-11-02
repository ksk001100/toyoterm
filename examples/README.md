# Extension recipes

[日本語](README.ja.md)

These standalone configurations turn the Ruby API into everyday terminal tools.
Run from the repository root (or replace the path with an absolute path):

```sh
cargo run --locked -- --config examples/status_bar/config.rb
```

With an installed binary, use `toyoterm --config examples/status_bar/config.rb`.
Replace `status_bar` with any recipe below. Each directory has its own guide.

| Recipe | Experience | Try it |
| --- | --- | --- |
| [Status bar](status_bar/README.md) | Workspace, cwd, live command status, async Git branch, clock, failure badges | Run a successful and a failing command, then change directory |
| [Dynamic theme](dynamic_theme/README.md) | Day/night colors following local time, with a manual override | Alt+T opens the theme policy selector |
| [Pane workbench](pane_workbench/README.md) | Two same-directory shells, project tabs, cross-workspace Pane finder | Alt+W, Alt+N, Alt+P |
| [Command journal](command_journal/README.md) | Recent exit statuses and elapsed times in a searchable panel | Alt+J reviews results; Alt+K clears them |

All four use [shared navigation](shared/keys.rb): Ctrl+Shift+T creates a tab,
Ctrl+Tab / Ctrl+Shift+Tab changes tabs, Ctrl+Shift+E / D splits right/down,
Ctrl+Shift+arrows moves between Panes, Ctrl+Shift+Z toggles zoom, and
Ctrl+Shift+R reloads. Copy/paste uses Ctrl+Shift+C/V (Command+Shift+C/V on macOS).
Selectors and panels close with Escape. No recipe sends commands to your shell
or closes existing sessions. Alt+W adds two shells each time you press it.

Enable [shell integration](../docs/shell-integration.md) in the shell you use
for cwd, command-running state, exit status, and the journal. Without it, the
clock, theme selection and Pane navigation still work. Directory-based launches
use the normal inherited directory when no cwd report is available. Only the
status bar needs an optional external program, `git`; no network is used.
No recipe requires a gem, icon font, Unix shell utility, or a specific shell.

Each recipe is a complete config, with a bottom bar of its own. To adapt one to
your config, load its code explicitly with `require_relative`, reconcile key
bindings, and combine widgets into a single `window.bar :bottom` block (duplicate
bars are rejected). Copy `shared/` and any required sibling files too.
Successful reload resets in-memory recipe state and replaces registrations;
failed reload keeps the previous config. See the [Ruby API](../docs/mruby-api.md)
for snapshot, callback and rollback rules.

The existing top-level `.rb` examples remain focused demonstrations of individual
APIs, including [modes](modes.rb), [resource scopes](resource_scopes.rb),
[completion](completion.rb), and [cooperative work](cooperative_work.rb).
