# mruby configuration DSL and API

toyoterm embeds mruby for configuration, key bindings, runtime events, status
text, commands, and local plugins. This document is the reference for the Ruby
surface intended for configuration and plugin authors.

The runtime is mruby, not CRuby. CRuby gems, native extensions, and the complete
CRuby standard library are unavailable unless toyoterm explicitly bundles them.
Methods beginning with `__` are host integration details and are not public API.

## Loading configuration

toyoterm selects one configuration source in this order:

1. The path passed with `--config`
2. `TOYOTERM_CONFIG_FILE`
3. Platform default config path:
   - Linux / Unix: `$XDG_CONFIG_HOME/toyoterm/config.rb` (falls back to `~/.config/toyoterm/config.rb` if unset)
   - Windows: `%APPDATA%\toyoterm\config.rb` (falls back to `%USERPROFILE%\.config\toyoterm\config.rb` if it does not exist)

The default path is optional. A path selected explicitly must exist and contain
valid Ruby. Start toyoterm with a particular file using:

```sh
toyoterm --config /path/to/config.rb
```

The smallest useful configuration is:

```ruby
Toyoterm.configure do |config|
  config.font.family = "monospace"
  config.font.size = 14

  config.keys do
    ctrl_shift("t").new_tab
    ctrl_shift("\\").split(:right)
    ctrl_shift("r").reload_config
  end
end
```

`examples/default_config.rb` is a complete starting point. toyoterm has no
built-in GUI key bindings, so copy the bindings you want into your config.

Configuration reload is atomic. A candidate file is evaluated and validated in
a fresh VM; if it fails, the previous configuration remains active. Run
`Toyoterm.reload_config`, use a binding whose action is `reload_config`, or run
`toyoterm reload` to reload the selected file.

## Configuration DSL

`Toyoterm.configure { |config| ... }` yields a `Toyoterm::Config`. Nested
sections work either with a block or as an object:

```ruby
Toyoterm.configure do |config|
  config.window do |window|
    window.opacity = 0.95
  end
  config.ui.padding_x = 10
end
```

### Top-level settings

| Setting | Default | Meaning and validation |
| --- | --- | --- |
| `default_shell` | `nil` | Program used for new terminal sessions. `nil` or an empty value uses the platform default. Reloading does not replace a running shell. |
| `scrollback_lines` | `10_000` | Non-negative integer number of retained scrollback lines. |
| `theme` / `theme=` | `nil` | Name of a plugin theme. An unknown or empty name rejects the configuration. |

### `config.font`

| Setting | Default | Validation |
| --- | --- | --- |
| `family` | `"monospace"` | Non-empty installed font family name. |
| `fallback` | `[]` | Array of at most 32 non-empty, unique family names; it must not repeat `family`. |
| `size` | `14.0` | Positive finite number. |
| `weight` | `400` | Integer from 1 through 1000. |

Unknown font families are skipped by the font system. Use exact installed family
names, particularly for Nerd Fonts.

### `config.colors`

All colors use `#RRGGBB` strings.

| Setting | Default |
| --- | --- |
| `background` | `#090b0e` |
| `foreground` | `#dce1e8` |
| `cursor` | `#f5f7fa` |
| `selection` | `#375891` |
| `tab_bar` | `#11151b` |
| `tab_active` | `#18243a` |
| `tab_inactive` | `#15191f` |
| `workspace_bar` | `#0d1014` |
| `status_bar` | `#101419` |
| `pane_border` | `#375891` |
| `zoomed_pane_border` | `#ffbe3a` |
| `search_match` | `#c4972f` |
| `search_match_active` | `#ffbe3a` |

`colors.zoomed_pane_border` returns the `#RRGGBB` string used for the active
pane's border on all four sides while zoomed. Set it with
`config.colors.zoomed_pane_border = "#ffbe3a"`; assignment returns the assigned
value. Ordinary active panes use `pane_border`, including a single unzoomed
pane. Both use `ui.active_pane_border_width` (zero hides the indicator).
The setting supports themes, reload, and runtime configuration. Invalid colors
reject the configuration transaction and preserve the previous settings.

`colors.ansi` is an array of exactly 16 `#RRGGBB` strings for ANSI indexes
0 through 15. Indexes 16 through 231 use the standard xterm 6x6x6 cube, and
indexes 232 through 255 use the xterm grayscale ramp. Individual base colors can
be replaced with assignments such as `config.colors.ansi[1] = "#ff5f56"`.

### `config.window`

| Setting | Default | Validation or behavior |
| --- | --- | --- |
| `opacity` | `1.0` | Finite number from 0 through 1. |
| `width` | `960` | Positive finite initial logical width. |
| `height` | `600` | Positive finite initial logical height. |
| `min_width` | `320` | Positive finite minimum logical width. |
| `min_height` | `180` | Positive finite minimum logical height. |
| `decorations` | `true` | Boolean. |
| `resizable` | `true` | Boolean. |
| `always_on_top` | `false` | Boolean. |
| `title` | `"toyoterm"` | Non-empty string. |

Initial dimensions apply when the window is created. Mutable window properties
are also applied after a successful reload.

### `config.ui`

| Setting | Default | Validation |
| --- | --- | --- |
| `padding_x` | `8` | Non-negative finite number. |
| `padding_y` | `8` | Non-negative finite number. |
| `line_height` | `1.2857143` | Positive finite number. |
| `tab_bar` | `true` | Boolean controlling tab-bar visibility. |
| `tab_bar_height` | `30` | Positive finite number. |
| `tab_width` | `160` | Positive finite number. |
| `workspace_bar` | `true` | Boolean controlling workspace-bar visibility. |
| `workspace_bar_height` | `24` | Positive finite number. |
| `workspace_width` | `160` | Positive finite number. |
| `status_bar_height` | `24` | Positive finite number. |
| `pane_divider_width` | `2` | Non-negative finite number. |
| `active_pane_border_width` | `2` | Non-negative finite number. |

### `config.behavior`

| Setting | Default | Validation |
| --- | --- | --- |
| `scroll_lines` | `3` | Positive finite number of lines per mouse-wheel step. |
| `copy_on_select` | `false` | Boolean. |

## Key bindings

Static bindings resolve directly to a native action and do not invoke mruby when
pressed. Dynamic bindings invoke a Ruby block on the script thread. Defining the
same normalized chord more than once is an error, including a collision between
the two forms.

### Static bindings

`config.keys` accepts either an instance-evaluated block or an explicit block
parameter:

```ruby
config.keys do
  ctrl_shift("t").new_tab
  ctrl_shift("d").split(:down)
end

config.keys do |keys|
  keys.primary("c").copy_selection
end
```

| Helper | Modifier |
| --- | --- |
| `key(key)` | None |
| `ctrl(key)` | Control |
| `ctrl_shift(key)` | Control+Shift |
| `ctrl_alt(key)` | Control+Alt |
| `ctrl_super(key)` | Control+Super |
| `alt(key)` | Alt/Option |
| `super_key(key)` | Super/Command |
| `primary(key)` | Command on macOS, Control elsewhere |
| `primary_shift(key)` | Primary+Shift |
| `primary_alt(key)` | Primary+Alt |
| `leader(key)` | The configured leader prefix |
| `physical(key, mods = "")` | Physical position, such as `physical("KeyH", "CTRL")` |

Key and modifier names are case-insensitive. Named keys include `ENTER`, `TAB`,
`SPACE`, `ESCAPE`, arrow and navigation keys, and `F1` through `F12`. Physical
bindings take priority over logical bindings.

Each helper returns a binding with one of these actions:

| Action | Argument |
| --- | --- |
| `activate_pane(direction)` | `:left`, `:right`, `:up`, or `:down` |
| `split(direction)` | `:left`, `:right`, `:up`, or `:down` |
| `new_tab`, `close_pane`, `close_tab`, `new_workspace` | None |
| `reload_config`, `search` | None |
| `maximize_window`, `toggle_maximize`, `minimize_window`, `toggle_fullscreen`, `toggle_zoom` | None |
| `next_tab`, `previous_tab`, `next_workspace`, `previous_workspace` | None |
| `copy_selection`, `paste_clipboard` | None |
| `start_visual_mode`, `toggle_visual_mode`, `start_visual_selection`, `select_visual_selection`, `end_visual_selection` | None |
| `move_visual_selection(direction)` | `:left`, `:right`, `:up`, `:down`, `:line_start`, or `:line_end` |
| `yank_selection` | None |
| `command(name)` | Name registered with `Toyoterm.command` |

Aliases are available for readability: `enter_visual_mode`,
`toggle_visual_selection`, `select`, `exit_visual_mode`, `visual_move`,
`copy_visual_selection`, and `toggle_pane_zoom`.

Configure a leader with a positive timeout in milliseconds:

```ruby
config.leader key: "b", mods: "CTRL", timeout: 1000
config.keys { leader("v").toggle_visual_mode }
```

The leader prefix is consumed. An unmatched or expired suffix continues through
normal input handling. IME activity, focus loss, and configuration reload clear
leader state.

### Dynamic bindings

`config.bind(chord) { |context| ... }` invokes Ruby with a
`Toyoterm::KeyBindingContext`. `Toyoterm::CommandContext` is an alias of the same
class. Both expose `context.pane`.

```ruby
config.bind "CTRL+SHIFT+H" do |context|
  context.pane.send_text("echo hello from mruby\n")
end
```

Unmatched keys bypass mruby and go directly to the terminal input encoder. If a
callback raises, all commands and clipboard writes queued by that callback are
discarded and the terminal keeps running.

## Commands and object model

Every callback sees a snapshot of the native object model:

| Module method | Return value |
| --- | --- |
| `Toyoterm.current_workspace` | Current `Toyoterm::Workspace` |
| `Toyoterm.current_window` | Current `Toyoterm::Window` |
| `Toyoterm.current_tab` | Current `Toyoterm::Tab` |
| `Toyoterm.current_pane` | Current `Toyoterm::Pane` |
| `Toyoterm.workspaces` | All workspaces, ordered by ID |
| `Toyoterm.windows` | All windows, ordered by ID |
| `Toyoterm.workspace(name)` | Matching workspace, or `nil` |
| `Toyoterm.switch_workspace(name)` | Queues activation or creation of a named workspace and returns `nil` |
| `Toyoterm.action(name, argument = nil)` | Queues a built-in action and returns `nil` |

All native objects inherit from `Toyoterm::NativeHandle`. They expose a
non-negative integer `id`, equality and hashing by class and ID, `valid?`, and
`validate!`. A handle saved across callbacks can become stale. Accessing state
or enqueuing a mutation through a stale handle raises
`Toyoterm::InvalidHandleError`, whose `kind` and `id` identify the object.

Mutating methods enqueue native work. Commands are applied only after the Ruby
callback returns successfully, so the callback continues to see its input
snapshot.

`Toyoterm.switch_workspace(name)` converts `name` with `to_s` and rejects an
empty name or NUL byte. If the name already exists it is activated; otherwise a
complete workspace, window, tab, and pane hierarchy is created and activated.
The new objects are not visible in the current callback snapshot. The command
and its resulting `workspace_changed` and focus events are discarded if the
callback raises before returning.

```ruby
Toyoterm.command :backend do
  Toyoterm.switch_workspace(:backend)
end
```

`Toyoterm.action(name, argument = nil)` makes the native actions from the
static-binding table available inside dynamic bindings, commands, and event
handlers. Action names can be Strings or Symbols and are normalized to
lowercase. `split` and `activate_pane` require `:left`, `:right`, `:up`, or
`:down`; `move_visual_selection` requires one of its six documented motions.
All other actions take no argument. An empty or unknown action, a missing or
invalid required argument, or an argument supplied to a no-argument action
raises `ArgumentError` before anything is queued.

The `command(name)` static-binding action is intentionally excluded; call
shared Ruby code directly instead. Actions run in queue order only after the
callback succeeds, and are discarded if it raises. They operate on the active
pane, tab, workspace, or native window at application time; use a handle method
when an operation must target a specific snapshot object. Actions that depend
on UI state retain their ordinary behavior: for example `search` opens the
interactive search bar and `yank_selection` does nothing unless a visual
selection is active.

```ruby
Toyoterm.command :presentation_mode do
  Toyoterm.action(:toggle_fullscreen)
end

Toyoterm.on :bell do
  Toyoterm.action(:search)
end
```

### `Toyoterm::Workspace`

| Member | Result |
| --- | --- |
| `name` | Workspace name. |
| `windows` | Child `Window` handles. |
| `activate` | Queues activation and returns `self`. |
| `create_window(command: nil, cwd: nil, env: nil)` | Queues a new window in this workspace and returns `self`. |

### `Toyoterm::Window`

| Member | Result |
| --- | --- |
| `tabs` | Child `Tab` handles. |
| `new_tab(command: nil, cwd: nil, env: nil)` | Queues a new tab in this window and returns `self`. |
| `close` | Queues closing this window and returns `self`. |
| `focus` | Queues activation and returns `self`. |

### `Toyoterm::Tab`

| Member | Result |
| --- | --- |
| `title` | Current title. |
| `panes` | Child `Pane` handles. |
| `close` | Queues closing this tab and returns `self`. |
| `focus` / `activate` | Queues activation and returns `self`. |

### `Toyoterm::Pane`

| Member | Result |
| --- | --- |
| `title` | Current terminal title. |
| `cwd` | Working directory or `nil`; requires OSC 7 reporting. |
| `pid` | Child process ID or `nil`. |
| `command_running?` | Whether shell integration reports an active command. |
| `last_exit_status` | Last reported exit status or `nil`. |
| `screen_text` | Visible terminal rows joined with newlines. |
| `split(direction, command: nil, cwd: nil, env: nil)` | Queues `:left`, `:right`, `:up`, or `:down`; returns `self`. |
| `close` | Queues closing the pane and returns `self`. |
| `focus` | Queues activation and returns `self`. |
| `send_text(text)` | Queues text for the PTY and returns `self`; rejects NUL bytes. |
| `search(query, direction: :next)` | Queues a literal scrollback search and returns `self`. |
| `badge` / `badge=` | Reads or queues pane-corner display text. Assign `nil` to clear it. |

`Workspace#create_window`, `Window#new_tab`, and `Pane#split` accept an optional
launch specification:

```ruby
Toyoterm.command :dev_layout do |context|
  Toyoterm.current_workspace.create_window(command: "btop")
  context.pane.split(
    :right,
    command: ["cargo", "watch", "-x", "test"],
    cwd: context.pane.cwd,
    env: { "RUST_BACKTRACE" => "1", "OLD_TOKEN" => nil }
  )
end
```

`command` is either a non-empty program String or a non-empty argv Array of
Strings whose first entry is the program. It is executed directly without shell
parsing. When `command` is `nil`, the configured or platform default shell is
used. `cwd` is an optional non-empty UTF-8 path. `env` is an optional Hash with
non-empty String keys and String or `nil` values; names cannot contain `=`, and
`nil` removes an inherited variable. Launch strings cannot contain NUL bytes.
Invalid types and values raise `TypeError` or `ArgumentError` before anything is
queued. As with other mutations, the new handle is not visible inside the
callback that creates it, and the entire launch is discarded if the callback
raises.

`Pane#search` accepts a non-empty query converted with `to_s` and a direction of
`:next` or `:previous`. It activates the target pane, opens the existing search
bar, highlights literal matches in its visible screen and scrollback, and moves
to the requested match. The query cannot contain NUL. Repeated calls continue
from the terminal's current match; a query with no matches is not an error and
displays zero matches. The search is applied only after a successful callback,
like other queued mutations.

```ruby
Toyoterm.command :previous_error do |context|
  context.pane.search("error", direction: :previous)
end
```

`Pane#screen_text` returns a new String containing the viewport captured before
the callback began. Rows are joined with `\n`, trailing whitespace on each row
is removed, blank rows are retained, and no extra final newline is added. Text
outside the current viewport is not included, even when it remains in
scrollback. Changing the returned String does not affect the terminal or later
reads. As with other handle reads, a stale Pane raises
`Toyoterm::InvalidHandleError`. Capturing is proportional to the visible grids
across all panes, so avoid high-frequency polling when many panes are open.

```ruby
Toyoterm.command :copy_screen do |context|
  Toyoterm.clipboard.write(context.pane.screen_text)
end
```

`pane.chdir` is intentionally absent because the shell owns its working
directory. If needed, send a correctly escaped shell command with `send_text`.
See `docs/shell-integration.md` for cwd and command-status reporting.

Register a named command and bind it to a static key:

```ruby
Toyoterm.command :git_status do |context|
  context.pane.send_text("git status\n")
end

Toyoterm.configure do |config|
  config.keys { ctrl_shift("g").command(:git_status) }
end
```

Command names must be non-empty and unique. A command callback receives a
`CommandContext` with `pane`. Its queued mutations are rolled back if it raises.

## Runtime events

Register handlers with `Toyoterm.on(name) { |event| ... }`. `Toyoterm::Event`
exposes `name`, `workspace`, `window`, `tab`, `pane`, `title`, `cwd`, and
`exit_status`; unrelated fields are `nil`.

| Event | Populated fields |
| --- | --- |
| `app_started` | `pane` |
| `config_reloaded` | `pane` |
| `workspace_changed` | `workspace` |
| `window_created`, `window_closed` | `window` |
| `tab_created`, `tab_closed` | `tab` |
| `pane_created`, `pane_closed`, `pane_focused` | `pane` |
| `title_changed` | `pane`, `title` |
| `cwd_changed` | `pane`, `cwd` |
| `command_started` | `pane` |
| `command_finished` | `pane`, `exit_status` when reported |
| `bell` | `pane` |

```ruby
Toyoterm.on :cwd_changed do |event|
  event.pane.badge = event.cwd
end
```

`command_started` and `command_finished` require OSC 133 shell integration.
`command_finished.exit_status` is `nil` when the shell emits a completion marker
without a valid decimal status. Badge changes become visible after a successful
callback and are discarded if it raises. Badge text is drawn in the pane's
upper-right corner; assigning `nil` removes it, and configuration reload clears
badges owned by the replaced VM.

Closed-object events retain the deleted object's typed ID, but dereferencing it
raises `Toyoterm::InvalidHandleError`. Events are processed in FIFO order. One
handler runs to completion and its commands are applied before the next event;
callbacks are never entered recursively. If a handler raises, its queued
commands are discarded.

## Status callback

`Toyoterm.status(interval: 1.0) { |context| ... }` configures the status bar.
Only one callback may be registered, and the finite numeric interval must be at
least 0.1 seconds. `Toyoterm::StatusContext` exposes `workspace`, `window`,
`tab`, and `pane`. The result is converted to a string.

```ruby
Toyoterm.status(interval: 1.0) do |context|
  [context.workspace.name, context.pane.cwd].compact.join(" | ")
end
```

The bar is hidden when no callback is configured. Commands queued by a status
callback are always discarded.

## Platform, clipboard, environment, files, and processes

Configuration and plugins are trusted code. These APIs are intentionally not
sandboxed and carry the authority of the toyoterm process.

- `Toyoterm.clipboard.read` returns a copy of the text clipboard snapshot and
  raises `RuntimeError` if the clipboard is unavailable.
- `Toyoterm.clipboard.write(text)` queues a write, returns the clipboard object,
  and rejects NUL bytes. A callback error rolls the write back.
- `Toyoterm.platform` returns the host platform as `:linux`, `:macos`, or
  `:windows`. Targets outside those three return `:other`.
- `Toyoterm.env` returns a copy of the environment captured when the VM was
  created. Non-UTF-8 entries are omitted; changing the Hash affects no process.
- `Toyoterm.read_file(path)` returns a byte-preserving String. The UTF-8 path
  must not contain NUL; I/O failures raise `RuntimeError`.
- `Toyoterm.spawn(program, *args)` runs synchronously on the script thread and
  captures byte-preserving output. Arguments are stringified and cannot contain
  NUL. Launch failures raise `RuntimeError`; nonzero exit is a normal result.

`Toyoterm::ProcessResult` exposes `stdout`, `stderr`, `exit_status`, and
`success?`. A process terminated without a portable exit code reports `-1`.

```ruby
Toyoterm.configure do |config|
  config.window.decorations = false if Toyoterm.platform == :linux
end

result = Toyoterm.spawn("git", "status", "--short")
warn result.stderr unless result.success?
```

Long-running host calls delay later Ruby callbacks, but not PTY parsing or
rendering.

## Plugins and themes

toyoterm loads `*.rb` directly inside the default plugins directory (`$XDG_CONFIG_HOME/toyoterm/plugins/` or `~/.config/toyoterm/plugins/` on Linux/Unix, `%APPDATA%\toyoterm\plugins` on Windows) in
lexicographic filename order. Additional files can be requested with
`Toyoterm.plugin(path)`. Relative paths resolve from the declaring file, `~/`
expands to the home directory, and a canonical path is loaded only once.
`Toyoterm.plugins` returns loaded definitions; `Toyoterm.themes` returns theme
names.

Each plugin file must define exactly one plugin:

```ruby
Toyoterm::Plugin.define "git-tools" do |plugin|
  plugin.version = "0.1.0"
  plugin.requires = ">= 0.1.0, < 0.2.0"

  plugin.command(:git_root) do |context|
    context.pane.send_text("git rev-parse --show-toplevel\n")
  end
  plugin.on(:bell) { |event| event.pane.badge = "bell" }
  plugin.bind("CTRL+G") { |context| context.pane.send_text("git status\n") }
  plugin.keys { ctrl_shift("G").command(:git_root) }
end
```

The name must be non-empty and unique, and `version` is required. `requires` is
optional and constrains plugin API version `0.1.0` with comma-separated `=`,
`<`, `<=`, `>`, or `>=` clauses. Invalid metadata, incompatible requirements,
duplicate registrations, unreadable files, and Ruby exceptions disable only
that plugin and roll back its registrations.

Plugins can register named color themes:

```ruby
Toyoterm::Plugin.define "moon-theme" do |plugin|
  plugin.version = "0.1.0"
  plugin.theme "moon" do |colors|
    colors.background = "#10131a"
    colors.foreground = "#d8dee9"
    colors.cursor = "#88c0d0"
  end
end
```

A theme starts with the default colors and accepts every `config.colors` field.
Select it with `config.theme = "moon"`; later explicit color assignments
override it. Duplicate theme names disable the later plugin, while an unknown
selected theme rejects the config.

Plugins share the main configuration's VM and filesystem, process, environment,
and clipboard authority. Loading a plugin is equivalent to allowing its source
to execute as the toyoterm process.

## Live Ruby console

Connect to the running GUI's persistent VM with:

```sh
toyoterm ruby console
```

The console supports multiline input, `:history`, and `exit`.
`Toyoterm.configure` changes are validated and applied immediately. If an
evaluation leaves the config invalid, the whole evaluation transaction is
rolled back.

## Callback execution model

The main thread sends an immutable object-model and clipboard snapshot to the
single script thread. Ruby returns values and queued native commands; it never
mutates the mux directly. Requests are serialized, so a slow or stuck callback
delays later Ruby work but does not stop PTY output parsing or frame scheduling.

There is no forced callback timeout. Callbacks taking at least 100 ms are logged
as slow under `toyoterm::script`. Avoid blocking or unbounded work in bindings,
event handlers, commands, and status callbacks.
