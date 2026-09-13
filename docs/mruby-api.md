# mruby configuration DSL and API

toyoterm embeds mruby for configuration, key bindings, runtime events, status
text, commands, and local plugins. This document is the reference for the Ruby
surface intended for configuration and plugin authors.

The runtime is mruby, not CRuby. CRuby gems, native extensions, and the complete
CRuby standard library are unavailable unless toyoterm explicitly bundles them.
Methods beginning with `__` are host integration details and are not public API.

See the [usage guide](usage.md) for CLI commands and troubleshooting, or the
[documentation index](README.md) for all guides.

- [Loading configuration](#loading-configuration)
- [Bundled Ruby libraries](#bundled-ruby-libraries)
- [Configuration DSL](#configuration-dsl)
- [Key bindings](#key-bindings)
- [Commands and object model](#commands-and-object-model)
- [Selection overlays](#selection-overlays)
- [Runtime events](#runtime-events)
- [Window bars](#window-bars)
- [Host APIs](#platform-clipboard-environment-files-and-processes)
- [Plugins and themes](#plugins-and-themes)
- [Live Ruby console](#live-ruby-console)
- [Callback execution model](#callback-execution-model)

## Bundled Ruby libraries

The embedded runtime includes `mruby-error` and the `stdlib`, `stdlib-ext`,
`math`, and `metaprog` core gemboxes. Their APIs are available directly in
configuration, callbacks, commands, plugins, and the live console; no `require`
call is needed.

| Group | Included capabilities |
| --- | --- |
| Collections and iteration | `Set`, `Enumerator`, `Enumerator::Lazy`, `Enumerator::Chain`, and extended `Array`, `Hash`, `Enumerable`, and `Range` methods |
| Objects and control flow | `Fiber`, `ObjectSpace`, `catch` / `throw`, and extended `Object`, `Kernel`, `Module`, `Class`, `Numeric`, `Symbol`, and top-level methods |
| Standard data types | `Struct`, `Data`, `Time`, `Random`, `Array#pack`, `String#unpack`, and `sprintf` |
| Math | `Math`, `Rational`, `Complex`, and multi-precision integers |
| Metaprogramming | source compilation and `eval`, `Binding`, `Proc#binding`, reflection, `Method`, and `UnboundMethod` |

This includes runtime method definition and reflection such as
`define_singleton_method`, `send`, `public_send`, `methods`, `instance_methods`,
`instance_variable_get` / `instance_variable_set`, `class_exec`, `module_exec`,
`Class#subclasses`, `Object#method`, `Module#instance_method`, `Method`, and
`UnboundMethod`. It also includes object helpers such as `tap`, `then`,
`itself`, and `instance_exec`. These are mruby APIs and can differ from the
corresponding CRuby version.

For example, a plugin can generate methods and retain a callable method object:

```ruby
class Greeting
  define_method(:call) { |name| "hello #{name}" }
end

greeting = Greeting.new
greeting.define_singleton_method(:excited) { |name| call(name).upcase }
callback = greeting.method(:excited)
callback.call("toyoterm") # => "HELLO TOYOTERM"
```

Ruby exceptions raised by these APIs follow the same atomic reload and callback
rollback rules as the rest of the scripting API. `Method#source_location` is
available, but may return `nil` because toyoterm does not enable mruby debug
information in production builds.

OS-dependent `mruby-io`, `mruby-dir`, `mruby-socket`, and `mruby-task` are not
bundled. Neither are `mruby-sleep`, `mruby-exit`, command binaries, or test gems.
Use the [host APIs](#platform-clipboard-environment-files-and-processes) for
files and processes. Keeping platform HAL gems out of the amalgamation preserves
one runtime build across Linux, macOS, and Windows; it also avoids introducing
blocking socket or task-scheduler behavior into the single script thread.

## Loading configuration

toyoterm selects one configuration source in this order:

1. The path passed with `--config`
2. `TOYOTERM_CONFIG_FILE`
3. Platform default config path:
   - Linux / macOS / Unix: `$XDG_CONFIG_HOME/toyoterm/config.rb` (falls back to `~/.config/toyoterm/config.rb` if unset or empty)
   - Windows: `%APPDATA%\toyoterm\config.rb` (falls back to `%USERPROFILE%\.config\toyoterm\config.rb` if it does not exist)

The default path is optional. A path selected explicitly must exist and contain
valid Ruby. The GUI recovers from a missing or invalid selected file by showing
an error banner and keeping defaults, with the path retained for a later reload.
An empty `TOYOTERM_CONFIG_FILE` is ignored. Start with a particular file using:

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

[examples/default_config.rb](../examples/default_config.rb) is a complete starting point. toyoterm has no
built-in GUI key bindings, so copy the bindings you want into your config.

Configuration reload is atomic. A candidate file is evaluated and validated in
a fresh VM; if it fails, the previous configuration remains active. Run
`Toyoterm.reload_config`, use a binding whose action is `reload_config`, or run
`toyoterm reload` to reload the selected file.

## Configuration DSL

### Design conventions and migration

- Settings use property getters and `=` setters. Section blocks receive an
  explicit object; `config.keys` additionally supports the concise block form.
- Setting setters validate their individual type and range immediately. Values
  shared with the VM are copied, and configuration strings are frozen; invalid
  cross-field combinations are rejected when the request commits.
- Configure both native actions and Ruby callbacks under `config.keys`:
  `keys.ctrl("t").new_tab` or `keys.ctrl("h").run { |context| ... }`.
- Built-in action names and argument validation are identical in
  bindings and `Toyoterm.action`. Native bindings still bypass Ruby on key press.
- Use `activate` on any workspace, window, tab, or pane. Use `new_window` and
  `new_tab` to create children, and `split` to create a pane.
- Dynamic actions are bound to the callback's object snapshot. If focus changes
  while Ruby is running, the action still targets the originating context.
- Identifiers accept only String or Symbol. Text, paths, process programs, and
  process arguments accept only String; the API does not silently call `to_s`.
- Key, command, and bar contexts all expose `workspace`, `window`, `tab`, and
  `pane`. Events retain their event-specific fields; absent fields remain `nil`.

The API is pre-release and does not retain compatibility aliases. Replace
`config.bind(chord)` / `plugin.bind(chord)` with `config.keys.key(chord).run` /
`plugin.keys.key(chord).run`, `focus` with `activate`, and `create_window` with
`new_window`. Set themes with `config.theme = name`; `config.theme` is a getter
only and passing an argument raises `ArgumentError`.

Removed action names map to the following canonical names:

| Removed name | Canonical name |
| --- | --- |
| `toggle_pane_zoom` | `toggle_zoom` |
| `enter_visual_mode` | `start_visual_mode` |
| `toggle_visual_selection` | `toggle_visual_mode` |
| `select` | `select_visual_selection` |
| `exit_visual_mode` | `end_visual_selection` |
| `visual_move` | `move_visual_selection` |
| `copy_visual_selection` | `yank_selection` |

Removed methods raise `NoMethodError`; removed action names passed to `action`
raise `ArgumentError` before registering or queuing anything. Exceptions follow
the normal configuration and callback rollback rules.

`Toyoterm.configure` and the `font`, `colors`, `window`, `ui`, and `behavior`
section methods now return their configuration object instead of the last block
expression. Code that used that expression as the result should capture it
explicitly. `configure` requires a block and raises `ArgumentError` without one;
section getters may omit it. Blocks retain their caller's `self`. Exceptions
propagate and normal configuration/callback transaction rules apply.

The removed `Toyoterm.workspace` and `Toyoterm.switch_workspace` methods are
replaced by `Toyoterm.find_workspace` and `Toyoterm.open_workspace`. The latter
name makes its activate-or-create behavior explicit. The removed
`Toyoterm::Window` class is now `Toyoterm::MuxWindow`, making clear that it is
not an operating-system window.

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
| `family` | `"monospace"` | Non-empty font family name; installation is not validated. |
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
| `opacity` | `1.0` | Finite number, clamped to 0 (transparent) through 1 (opaque), applied to the default terminal background. Text, UI chrome, and explicit terminal background colors retain their own opacity. |
| `width` | `960` | Positive finite initial logical width. |
| `height` | `600` | Positive finite initial logical height. |
| `min_width` | `320` | Positive finite minimum logical width. |
| `min_height` | `180` | Positive finite minimum logical height. |
| `decorations` | `true` | Boolean. |
| `resizable` | `true` | Boolean. |
| `always_on_top` | `false` | Boolean. |
| `title` | `"toyoterm"` | Non-empty string. |

`window.image` returns a `Toyoterm::ImageConfig`. With a block,
`window.image { |image| ... }` yields that same object and returns it regardless
of the block's last expression. The block keeps its caller's `self`; exceptions
propagate through the normal configuration/callback transaction boundary.
The image object cannot be replaced with `window.image = ...`.

`window.image.path` / `path=(path)` reads or sets a PNG/JPEG
path String, or `nil` (the default) to disable the image. Assignment returns the
assigned value; the getter returns a frozen copy of the path or `nil`. Non-String
values raise `TypeError`; empty paths and NUL bytes raise `ArgumentError`.
Relative paths resolve from the main config file's directory (the process working
directory when no config file is selected); `~/` expands to the home directory.
Windows paths can use forward slashes, for example `"C:/Pictures/wallpaper.jpg"`.

`window.image.opacity` / `opacity=(value)` reads or
sets the image's blend strength over `colors.background`: a finite number from
0 through 1, default `1.0`. Assignment returns the assigned value. Non-numbers
raise `TypeError`; non-finite or out-of-range values raise `ArgumentError`.

The image covers the whole window, centered and cropped with its aspect ratio
preserved, behind text, UI chrome, and explicit terminal cell backgrounds.
PNG alpha blends with the background color. `window.opacity` applies to the
combined color/image background; text and UI chrome retain their own opacity.
Images are limited to 8192 pixels per axis with a 256 MiB decoder allocation
budget. Unsupported, unreadable, corrupt, or oversized images reject the config
transaction and preserve the previous settings and image.

Images are decoded on the script thread and shared as immutable pixels with the
renderer. Reload rereads the file, including changes at the same path. Live
configuration supports changing or clearing the path and changing blend strength;
unchanged paths reuse the loaded pixels, so use reload to refresh an edited file.
Callback failures roll back both image settings together with other changes.

```ruby
Toyoterm.configure do |config|
  config.window.image.path = "images/wallpaper.jpg"
  config.window.image.opacity = 0.25
end
```

The equivalent section form is:

```ruby
config.window.image do |image|
  image.path = "images/wallpaper.jpg"
  image.opacity = 0.25
end
```

The former `window.background_image` and `window.background_image_opacity`
getters/setters have been removed; use `window.image.path` and
`window.image.opacity` respectively. Old calls raise `NoMethodError`.

Initial dimensions apply when the window is created. Mutable window properties
are also applied after a successful reload.

`config.window.opacity` returns the stored numeric value.
`config.window.opacity=(value)` accepts a finite Numeric and stores and returns
its value clamped to `0.0..1.0`. Ruby assignment expressions themselves return
the original right-hand value. Non-numeric values raise `TypeError`; NaN and
infinity raise `ArgumentError`, leaving the stored value unchanged. Clamping
applies during startup, reload, and callbacks. Runtime changes reach the native
window only after the request succeeds and the complete configuration passes
validation; a failed transaction restores the previous opacity as well.

All other scalar setting assignments perform the table's type and range checks
at assignment time. `colors.ansi` is a validation-aware array, so indexed
assignment also rejects invalid colors immediately. Returned setting strings
and font fallback arrays are frozen to prevent mutation outside their setters.

These bindings stop at each limit, even with repeated presses, and immediately
respond to a press in the opposite direction:

```ruby
config.keys.key("CTRL+[").run do
  config.window.opacity -= 0.1
end
config.keys.key("CTRL+]").run do
  config.window.opacity += 0.1
end
```

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
| `allow_osc52_copy` | `false` | Boolean opt-in allowing terminal output to replace the system clipboard through OSC 52, one-shot iTerm2 OSC 1337 `Copy=:`, or a general `CopyToClipboard=` / `EndCopy` text capture. Clipboard reads and iTerm2's named `rule`/`find`/`font` buffers remain disabled. Decoded or captured payloads over 64 KiB are rejected. Applies to existing panes after a successful reload and cancels an active capture when disabled. |
| `allow_osc_notifications` | `false` | Boolean opt-in for OSC 9, OSC 99, and OSC 777 desktop notifications. OSC 99 occasions, urgency, `system`/`silent` sound selection, and Linux/macOS explicit close and guaranteed positive expiry are honored; Linux also supports standard named sounds and safe named icons. Windows close is a no-op and expiry is best effort, so neither capability is advertised there. Assembled title/body fields over 4 KiB or containing control characters are rejected; each pane is limited to one notification every two seconds. Applies immediately after a successful reload. |
| `allow_osc_attention_requests` | `false` | Boolean opt-in for OSC 1337 `RequestAttention=yes`, `once`, and `no`. These map to the platform's indefinite, one-shot, and cancel attention hints. `fireworks` is ignored because toyoterm has no cursor-local animation surface. Applies immediately after a successful reload. |
| `allow_osc_open_url` | `false` | Boolean opt-in allowing OSC 1337 `OpenURL=:` to launch a base64-encoded URL without a user gesture. Only `https`, `http`, and `mailto` URLs accepted by the normal 2,048-byte URL validator are allowed, and requests are limited to one per pane every two seconds. Applies immediately after a successful reload. |

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
| `next_prompt`, `previous_prompt` | None; cyclically reveals retained OSC 133 `A` markers in the active pane. No marker is a no-op. |
| `next_mark`, `previous_mark` | None; cyclically reveals retained iTerm2 OSC 1337 `SetMark` locations in the active pane. No mark is a no-op. |
| `select_last_command_output` | None; selects the most recent complete OSC 133 `C`–`D` range. No complete range is a no-op. |
| `select_next_command_output`, `select_previous_command_output` | None; cyclically selects complete OSC 133 `C`–`D` ranges. No complete range is a no-op. |
| `copy_selection`, `paste_clipboard` | None |
| `start_visual_mode`, `toggle_visual_mode`, `start_visual_selection`, `select_visual_selection`, `end_visual_selection` | None |
| `move_visual_selection(direction)` | `:left`, `:right`, `:up`, `:down`, `:line_start`, or `:line_end` |
| `yank_selection` | None |
| `command(name)` | Name registered with `Toyoterm.command` |

Configure a leader with a positive timeout in milliseconds:

```ruby
config.leader key: "b", mods: "CTRL", timeout: 1000
config.keys { leader("v").toggle_visual_mode }
```

The leader prefix is consumed. An unmatched or expired suffix continues through
normal input handling. IME activity, focus loss, and configuration reload clear
leader state.

Prefix repeat events are consumed without extending the original timeout.

OSC 133 prompt navigation and OSC 1337 `SetMark` navigation share a limit of
4,096 semantic markers per terminal.
Markers follow scrollback movement and are discarded when terminal resizing
can reflow lines. The default configuration uses
`leader("[").previous_prompt`, `leader("]").next_prompt`,
`leader(",").previous_mark`, `leader(".").next_mark`,
`leader("p").select_previous_command_output`,
`leader("n").select_next_command_output`, and
`leader("o").select_last_command_output`. Each method returns the binding
object during configuration. Navigation is a no-op without a prompt marker;
output selection is a no-op without a complete `C`–`D` range and otherwise
uses the normal selection/copy pipeline.

### Visual selection

`toggle_visual_mode` enters visual mode without selecting text. Move to the
desired position, then use `select_visual_selection` and movement actions to
extend the selection; `yank_selection` copies it. Movement and selection actions
are inactive in normal mode, so bindings for `h/j/k/l` can coexist with ordinary
shell input. Use a leader chord to enter visual mode without intercepting `v`.

```ruby
config.leader key: "b", mods: "CTRL", timeout: 1000
config.keys do
  leader("v").toggle_visual_mode
  key("SPACE").select_visual_selection
  key("h").move_visual_selection(:left)
  key("j").move_visual_selection(:down)
  key("k").move_visual_selection(:up)
  key("l").move_visual_selection(:right)
  key("y").yank_selection
end
```

### Dynamic bindings

`config.keys.key(chord).run { |context| ... }` invokes Ruby with a
`Toyoterm::CallbackContext`. `Toyoterm::KeyBindingContext` and
`Toyoterm::CommandContext` are aliases of the same class. They expose
`context.workspace`, `context.window`, `context.tab`, and
`context.pane`, captured when the callback starts. The first three identify the
current objects in the request snapshot; `pane` is the callback's target pane.
They are snapshot handles, not live mutable native objects.

`context.actions` exposes the same built-in methods as a static binding, plus
`action(name, argument = nil)`. These actions are tied to this captured context.
`Toyoterm.action` also captures the current callback context.

Every key helper supports `run`, including `primary`, `leader`, and `physical`.
`run { |context| ... }` registers the block and returns the binding object;
omitting the block raises `ArgumentError`. Static and dynamic bindings share
duplicate detection: registering the same chord more than once raises
`ArgumentError`. Callback failures discard their queued commands and clipboard
writes. `plugin.keys` supports the same syntax and plugin registration rollback.
Use `config.keys.unbind(chord)` to remove an existing static or dynamic binding;
it returns whether a binding was removed. Successful live-console registry
changes are mirrored to the native key resolver immediately.

`binding.action(name, argument = nil)` registers a static built-in action and
returns the binding object. It uses the same names, accepted arguments,
and `ArgumentError` validation as `Toyoterm.action(name, argument = nil)` below.
For example, `keys.ctrl("z").action(:toggle_zoom)` and
`keys.ctrl("z").toggle_zoom` are equivalent. Invalid arguments register nothing.
Each binding may register only one action or callback.

```ruby
config.keys.key("CTRL+SHIFT+H").run do |context|
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
| `Toyoterm.current_window` | Current `Toyoterm::MuxWindow` |
| `Toyoterm.current_tab` | Current `Toyoterm::Tab` |
| `Toyoterm.current_pane` | Current `Toyoterm::Pane` |
| `Toyoterm.workspaces` | All workspaces, ordered by ID |
| `Toyoterm.windows` | All windows, ordered by ID |
| `Toyoterm.find_workspace(name)` | Matching workspace, or `nil` |
| `Toyoterm.open_workspace(name)` | Queues activation or creation of a named workspace and returns `nil` |
| `Toyoterm.action(name, argument = nil)` | Queues a built-in action and returns `nil` |

All native objects inherit from `Toyoterm::NativeHandle`. They expose a
non-negative integer `id`, equality and hashing by class and ID, `valid?`, and
`validate!`. A handle saved across callbacks can become stale. Accessing state
or enqueuing a mutation through a stale handle raises
`Toyoterm::InvalidHandleError`, whose `kind` and `id` identify the object.

Mutating methods enqueue native work. Commands are applied only after the Ruby
callback returns successfully, so the callback continues to see its input
snapshot.

`Toyoterm.open_workspace(name)` accepts a String or Symbol and rejects an empty
name or NUL byte. If the name already exists it is activated; otherwise a
complete workspace, window, tab, and pane hierarchy is created and activated.
The new objects are not visible in the current callback snapshot. The command
and its resulting `workspace_changed` and focus events are discarded if the
callback raises before returning.

```ruby
Toyoterm.command :backend do
  Toyoterm.open_workspace(:backend)
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
callback succeeds, and are discarded if it raises. They resolve against the
workspace, mux window, tab, and pane captured when the callback began; a stale
target rejects the operation. Actions that depend
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
| `windows` | Child `MuxWindow` handles. |
| `activate` | Queues activation and returns `self`. |
| `new_window(command: nil, cwd: nil, env: nil)` | Queues a new mux window in this workspace and returns `nil`. |

### `Toyoterm::MuxWindow`

`MuxWindow` is a mux object. The GUI displays the active mux window in a single OS
window; `Workspace#new_window` does not create another OS window. Multiple OS
windows remain deferred.

| Member | Result |
| --- | --- |
| `tabs` | Child `Tab` handles. |
| `new_tab(command: nil, cwd: nil, env: nil)` | Queues a new tab in this mux window and returns `nil`. |
| `close` | Queues closing this window and returns `self`. |
| `activate` | Queues activation and returns `self`. |

### `Toyoterm::Tab`

| Member | Result |
| --- | --- |
| `title` | Current title. |
| `panes` | Child `Pane` handles. |
| `zoomed?` | Whether this tab is currently zoomed. |
| `close` | Queues closing this tab and returns `self`. |
| `activate` | Queues activation and returns `self`. |

### `Toyoterm::Pane`

| Member | Result |
| --- | --- |
| `title` | Current terminal title. |
| `icon_title` | Latest OSC 1 icon title, or `nil`. This read-only metadata is independent of the displayed terminal title, limited to 1 KiB, and rejects invalid UTF-8 or control characters. |
| `cwd` | Working directory or `nil`; requires OSC 7 reporting. |
| `remote_host` | Latest `user@host` report from OSC 1337 `RemoteHost=`, or `nil`. The read-only value is limited to 1 KiB; reports with a missing host, invalid UTF-8, or control characters are ignored. |
| `shell_integration_version` | Latest numeric version from OSC 1337 `ShellIntegrationVersion=`, or `nil`. Malformed or out-of-range versions are ignored. |
| `shell_integration_shell` | Shell name accompanying the latest OSC 1337 `ShellIntegrationVersion=` report, or `nil` for the deprecated version-only form. Names are limited to 64 UTF-8 bytes and cannot contain control characters or semicolons. |
| `user_vars` | A detached hash of OSC 1337 `SetUserVar` metadata. Names are limited to 128 bytes, UTF-8 values to 4 KiB, and each pane to 64 distinct names; malformed reports are ignored. Mutating the returned hash does not change pane state. |
| `pid` | Child process ID or `nil`. |
| `command_running?` | Whether shell integration reports an active command. |
| `last_exit_status` | Last reported exit status or `nil`. |
| `screen_text` | Visible terminal rows joined with newlines. |
| `zoomed?` | Whether this pane is the tab's current zoom target. |
| `split(direction, command: nil, cwd: nil, env: nil)` | Queues `:left`, `:right`, `:up`, or `:down`; returns `nil`. |
| `close` | Queues closing the pane and returns `self`. |
| `activate` | Queues activation and returns `self`. |
| `send_text(text)` | Queues text for the PTY and returns `self`; rejects NUL bytes. |
| `search(query, direction: :next)` | Queues a literal scrollback search and returns `self`. |
| `badge` / `badge=` | Reads or queues trusted pane-corner display text. Assign `nil` to clear it. A Ruby-set badge takes display precedence over an OSC 1337 `SetBadgeFormat` badge. |

`Workspace#new_window`, `Window#new_tab`, and `Pane#split` accept an optional
launch specification:

```ruby
Toyoterm.command :dev_layout do |context|
  Toyoterm.current_workspace.new_window(command: "btop")
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

`Tab#zoomed?` and `Pane#zoomed?` return Boolean values from the callback's
object-model snapshot. The tab method is true when that tab has a zoom target;
the pane method is true only for that target pane. A one-pane tab is not
implicitly zoomed.

```ruby
Toyoterm.command :copy_screen do |context|
  Toyoterm.clipboard.write(context.pane.screen_text)
end
```

`pane.chdir` is intentionally absent because the shell owns its working
directory. If needed, send a correctly escaped shell command with `send_text`.
See [shell integration](shell-integration.md) for cwd and command-status reporting.

Register a named command and bind it to a static key:

```ruby
Toyoterm.command :git_status do |context|
  context.pane.send_text("git status\n")
end

Toyoterm.configure do |config|
  config.keys { ctrl_shift("g").command(:git_status) }
end
```

Command names must be non-empty and unique. Pass `replace: true` to intentionally
replace one. `Toyoterm.command` returns a `Toyoterm::Registration`; call
`remove` to unregister it and use `active?` to inspect the handle. Replacing a
command makes its previous registration handle inactive. A callback receives a
`CommandContext` with `workspace`, `window`, `tab`, and `pane`.
Its queued mutations are rolled back if it raises.

## Selection overlays

`Toyoterm.select(title: "Select", items:) { |selection, context| ... }` opens a
searchable selection overlay centered in the application window. `items` must
be a non-empty Array containing no more than 4,096 non-empty Strings. Each item
is limited to 4 KiB and all items together are limited to 4 MiB. The optional
String title is limited to 256 bytes. Titles
and items reject NUL bytes and line breaks. The method requires a block, queues
the overlay, and returns `nil`.

Typing filters items by a case-insensitive substring match. Up/Down move through
the filtered results with wraparound, PageUp/PageDown move by ten entries, and
Home/End select the first or last result. Enter closes the overlay and invokes
the block with the selected String. Escape cancels it and invokes the block with
`nil`. The optional second block argument is the `CallbackContext` captured when
the overlay was requested:

```ruby
Toyoterm.command :choose_theme do
  Toyoterm.select(title: "Select theme", items: Toyoterm.themes) do |theme, context|
    next if theme.nil?

    Toyoterm.configure { |config| config.theme = theme }
    context.pane.badge = theme
  end
end
```

Only one selection may be pending in a VM. A second call raises `RuntimeError`.
`Toyoterm.select` is a runtime operation and cannot be called while a plugin
file itself is loading; register it inside a command, key, event, or asynchronous
callback instead.
The overlay captures keyboard and IME input while open, so typed characters are
not sent to the PTY. Reloading configuration closes an open overlay because it
replaces the VM and its callback. The result callback runs later on the single
script thread; its configuration changes and native commands use the normal
atomic callback transaction. A callback exception discards its queued work and
is logged without stopping the terminal. Use
`Toyoterm.supports?(:select_overlay)` when supporting older toyoterm builds.

## Runtime events

Register handlers with `Toyoterm.on(name) { |event| ... }`. `Toyoterm::Event`
exposes `name`, `workspace`, `window`, `tab`, `pane`, `title`, `cwd`, and
`exit_status`; unrelated fields are `nil`.

Unknown event names raise `ArgumentError` during registration. `Toyoterm.on`
returns a `Toyoterm::Registration` supporting `active?` and `remove`.
`event.context` returns the common callback context, while `event.subject`
returns the most specific populated native handle.

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
| `prompt_started` | `pane` |
| `command_line_started` | `pane` |
| `command_started` | `pane` |
| `command_finished` | `pane`, `exit_status` when reported |
| `bell` | `pane` |

```ruby
Toyoterm.on :cwd_changed do |event|
  event.pane.badge = event.cwd
end
```

`prompt_started`, `command_line_started`, `command_started`, and
`command_finished` require the corresponding OSC 133 shell-integration marker.
`command_finished.exit_status` is `nil` when the shell emits a completion marker
without a valid decimal status. Badge changes become visible after a successful
callback and are discarded if it raises. Badge text is drawn in the pane's
upper-right corner; assigning `nil` removes it, and configuration reload clears
badges owned by the replaced VM.

Closed-object events retain the deleted object's typed ID, but dereferencing it
raises `Toyoterm::InvalidHandleError`. Events are processed in FIFO order and
callbacks are never entered recursively. Each handler has its own transaction.
If one raises, its configuration, commands, badges, and newly queued
asynchronous work are rolled back, the error is logged, and later handlers for
the same event still run.

Events without a registered handler are skipped before invoking Ruby. Delivery
is limited to 1,024 events per application turn to bound self-generated loops.
While Ruby is slower than event production, at most 1,024 event requests wait
behind the active callback. Queued `title_changed`, `cwd_changed`,
`pane_focused`, and `workspace_changed` events for the same native object are
coalesced to their newest snapshot. If the event portion of the queue is still
full, newer events are dropped and a rate-limited warning is logged. Key
bindings, named commands, configuration reloads, and live-console evaluations
are lossless and are not subject to the event limit.

## Window bars

`config.window.bar(position, interval: 1.0) { |bar| ... }` configures a window
bar. `position` accepts only `:top` or `:bottom`; any other value raises
`ArgumentError`. One bar may be registered at each position, and registering a
duplicate raises `ArgumentError`. The finite numeric interval must be at least
0.1 seconds; other values fail configuration validation. Both bars use
`config.ui.status_bar_height`. The method returns the configured
`Toyoterm::BarConfig` object. Omitting the block raises `ArgumentError`.

`bar.section(alignment, separator: " | ") { |section| ... }` appends an aligned
section. `alignment` accepts `:left`, `:center`, or `:right`. A section contains
one or more synchronous values or asynchronous processes and joins its non-empty
values with `separator`. Multiple sections with the same alignment are rendered
in registration order, separated by one space. The method returns the configured
`Toyoterm::BarSection`. Invalid alignments, a missing block, and separators
containing NUL raise `ArgumentError`; a non-String separator raises `TypeError`.

```ruby
Toyoterm.configure do |config|
  config.window.bar :bottom, interval: 1.0 do |bar|
    bar.section(:left) { |section| section.add { |context| context.workspace.name } }
    bar.section(:center) { |section| section.add("toyoterm") }
    bar.section(:right) { |section| section.add { |context| context.pane.cwd } }
  end
end
```

`Toyoterm::BarSection#add(value = nil) { |context| ... }` appends a fixed value
or a synchronous block and returns the section. Supply either a non-`nil` value
or a block, not both. `Toyoterm::BarContext` exposes `workspace`, `window`,
`tab`, and `pane`; each result is converted to a string. `nil` and empty results
are omitted without leaving a separator. Text containing NUL raises
`ArgumentError` when the bar is rendered.

A position is hidden when no bar is configured for it. Each bar keeps its own
interval and is run serially on the script thread. Commands and pane badge
changes queued by widget callbacks are always discarded. If any widget raises,
the bar keeps its previous rendered content and is retried after its configured
interval.

Widgets can launch background asynchronous tasks via `Toyoterm.async` without
blocking bar evaluation. Asynchronous tasks queued during bar rendering are
retained when the widget callback succeeds (unlike native mutations, which are
discarded). Keep the returned task in the bar closure and read its result when
it completes; toyoterm immediately schedules a bar refresh after completion:

```ruby
Toyoterm.configure do |config|
  config.window.bar :bottom, interval: 1.0 do |bar|
    weather_task = nil
    bar.section(:right) do |section|
      section.add do
        weather_task ||= Toyoterm.async("curl", "-s", "https://wttr.in/Tokyo?format=1")
        if weather_task.complete? && weather_task.success?
          weather_task.result.stdout.strip
        elsif weather_task.complete?
          "Weather: unavailable"
        else
          "Weather: fetching..."
        end
      end
    end
  end
end
```

A section accepts static values, synchronous blocks, and asynchronous processes
in registration order. Each `add_async` call owns its result, refresh schedule,
and one in-flight process, so no user-side cache variables are required:

```ruby
bar.section(:right, separator: " | ") do |section|
  section.add_async("date", "+%Y-%m-%d %H:%M:%S",
                    interval: 1.0, initial: "clock...") do |result|
    result.success? ? result.stdout.strip : ""
  end

  section.add_async("git", "branch", "--show-current",
                    interval: 2.0, cwd: ->(ctx) { ctx.pane.cwd },
                    initial: "branch...") do |result|
    result.success? ? "\u{e725} #{result.stdout.strip}" : ""
  end

  section.add do |_ctx|
    battery = Toyoterm.read_file("/sys/class/power_supply/BAT0/capacity").strip
    battery.empty? ? "" : "#{battery}%"
  rescue
    ""
  end
end
```

`BarSection#add_async(program, *args, interval: 1.0, initial: "", cwd: nil) { |result| ... }`
registers one independent asynchronous process and returns the section. Its
optional block formats the resulting
`ProcessResult`; without a block, stdout is displayed. `cwd` may be a string,
`nil`, or a context lambda. A new process is not started while the previous
process for that item is still running. `interval` is the minimum delay between
process starts and must be at least 0.1 seconds. `initial` and the separator
must be Strings and may not contain NUL bytes. `nil` and empty displayed results
are omitted without leaving a separator. Use `Toyoterm.supports?(:bar_sections)`
to detect this API.


## Platform, clipboard, environment, files, and processes

Configuration and plugins are trusted code. These APIs are intentionally not
sandboxed and carry the authority of the toyoterm process.

- `Toyoterm.version` and `Toyoterm.api_version` return the application and Ruby
  API versions. `Toyoterm.supports?(capability)` performs feature detection.
- `Toyoterm.config` returns a deeply copied, frozen Hash snapshot of the active
  Ruby configuration.
- `Toyoterm.log(level, message)` sends `:debug`, `:info`, `:warn`, or `:error`
  output through the native `toyoterm::script::ruby` logger.

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
- `Toyoterm.spawn(program, *args, cwd: nil)` runs synchronously on the script
  thread and captures byte-preserving output. The program, arguments, and a
  non-`nil` `cwd` must be Strings and cannot contain NUL; `cwd` must not be empty. When supplied,
  `cwd` is the child process's working directory. Launch failures, including a
  missing or inaccessible working directory, raise `RuntimeError`; nonzero exit
  is a normal result.
- `Toyoterm.async(program, *args, cwd: nil) { |result| ... }` (aliased as
  `Toyoterm.async_spawn`) executes a child process asynchronously in a
  background worker thread without blocking the script thread or GUI. It returns
  a `Toyoterm::AsyncTask`; the block is optional. Omitting the block lets a
  widget retain the task in a local closure and inspect its result without
  global state. Supplying an empty `program`, passing
  NUL bytes, or passing an empty `cwd` raises `ArgumentError` immediately before
  scheduling. Upon completion, the block is invoked on the script thread with a
  `Toyoterm::ProcessResult` and the task's originating `CallbackContext`; a
  one-argument block may ignore the context. Launch failures report
  an exit status of `-1` and capture the error in `stderr` rather than raising a
  fatal exception. Exceptions raised inside the callback roll back any native
  commands or badge mutations queued by that callback. If configuration is
  reloaded while an asynchronous task is in flight, its callback is discarded
  safely without error.

`Toyoterm::AsyncTask` exposes `id`, `context`, `pending?`, `complete?`,
`cancelled?`, `cancel`, `result`, `error`, `value!`, and `success?`. `cancel`
prevents callback delivery and discards the result; a process already executing
may continue in its worker until it exits. Cancellation is a terminal task state,
so `complete?` becomes true while `success?` remains false. Completed tasks remove their runtime
registry entry, so periodic bar tasks do not accumulate results. `result` is
`nil` while the process is pending and otherwise is a
`Toyoterm::ProcessResult`. `Toyoterm::ProcessResult` exposes `stdout`,
`stderr`, `exit_status`, `error_kind`, `launch_error?`, and `success?`.
`error_kind` is `:launch` when the child could not be started. A process
terminated without a portable exit code reports `-1` without a launch error.

```ruby
Toyoterm.configure do |config|
  config.window.decorations = false if Toyoterm.platform == :linux
end

result = Toyoterm.spawn("git", "branch", "--show-current", cwd: "/path/to/repository")
warn result.stderr unless result.success?

task = Toyoterm.async("curl", "-s", "https://wttr.in/Tokyo?format=1")
if task.complete?
  warn task.result.stderr unless task.success?
  puts "Weather: #{task.result.stdout.strip}" if task.success?
end
```

Long-running synchronous host calls delay later Ruby callbacks, but not PTY
parsing or rendering. Use `Toyoterm.async` for network I/O, ping, or other
potentially slow commands to prevent blocking the script thread.

## Plugins and themes

At startup and reload, toyoterm loads `*.rb` directly inside the default plugins
directory in lexicographic filename order. The main config is evaluated first,
then automatic plugins load, followed by explicitly requested plugins in
declaration order. `Toyoterm.plugin(path)` queues loading rather than immediately
evaluating the file; plugin definitions are therefore not available while the
main config is being evaluated. Theme selection is resolved during validation
after plugins load.
Linux/macOS/Unix use `$XDG_CONFIG_HOME/toyoterm/plugins/`, falling back to
`~/.config/toyoterm/plugins/` when the variable is unset or empty. Windows checks
`%APPDATA%\toyoterm\plugins` then `%USERPROFILE%\.config\toyoterm\plugins` and
uses the first existing path (or the first available candidate if neither exists).
This discovery is independent of the selected configuration file. Additional files can be requested with
`Toyoterm.plugin(path)`. Relative paths resolve from the declaring file, `~/`
expands to the home directory, and a canonical path is loaded only once.
`Toyoterm.plugins` returns loaded definitions; `Toyoterm.themes` returns theme
names.

Each plugin file must define exactly one plugin:

```ruby
Toyoterm::Plugin.define "git-tools" do |plugin|
  plugin.version = "0.1.0"
  plugin.api_requirement = ">= 0.1.0, < 0.2.0"

  plugin.command(:git_root) do |context|
    context.pane.send_text("git rev-parse --show-toplevel\n")
  end
  plugin.on(:bell) { |event| event.pane.badge = "bell" }
  plugin.keys.ctrl("g").run { |context| context.pane.send_text("git status\n") }
  plugin.keys { ctrl_shift("G").command(:git_root) }
end
```

The name must be non-empty and unique, and the String `version` is required.
`api_requirement` is an optional String and constrains `Toyoterm.api_version` with comma-separated `=`,
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
to execute as the toyoterm process. Each file is evaluated under a generated
`Toyoterm::PluginNamespaces` module to prevent accidental top-level constant and
class collisions; explicit mutation of global objects remains possible because
plugins are trusted. Plugins may register commands, events, keys, and themes,
but extending the configuration DSL is not a supported contract: the main
configuration is evaluated before plugins are loaded.

## Live Ruby console

Connect to the running GUI's persistent VM with:

```sh
toyoterm ruby console
```

The console uses mruby's parser to recognize multiline input, including blocks,
methods, strings, and heredocs. Top-level local variables persist between
entries. It also supports `:history` and `exit`.
`Toyoterm.configure` changes are validated and applied immediately. If an
evaluation leaves the config invalid, the whole evaluation transaction is
rolled back.

Commands, event handlers, and key bindings added, removed, or replaced by a
successful console evaluation are mirrored to the native dispatcher in the
same transaction.

Live setting changes update the current window, renderer, and terminals without
rewriting the config file. Initial window dimensions only apply at creation;
`default_shell` only affects new sessions. Reloading evaluates a fresh VM and
replaces live changes with the file's settings.

```ruby
Toyoterm.configure do |config|
  config.font.size = 16
  config.window.opacity = 0.9
end
```

## Callback execution model

The main thread sends an immutable object-model and clipboard snapshot to the
single script thread. Ruby returns values and queued native commands; it never
mutates the mux directly. Requests are serialized, so a slow or stuck callback
delays later Ruby work but does not stop PTY output parsing or frame scheduling.

There is no forced callback timeout. Callbacks taking at least 100 ms are logged
as slow under `toyoterm::script`. Avoid blocking or unbounded work in bindings,
event handlers, commands, and bar widgets.
