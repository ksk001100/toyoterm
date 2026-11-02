# mruby configuration DSL and API

toyoterm embeds mruby for configuration, key bindings, runtime events, status
text, commands, local Ruby libraries, and themes. This document is the
reference for the public Ruby surface.

The runtime is mruby, not CRuby. CRuby gems, native extensions, and the complete
CRuby standard library are unavailable unless toyoterm explicitly bundles them.
Methods beginning with `__` are host integration details and are not public API.

See the [usage guide](usage.md) for CLI commands and troubleshooting, or the
[documentation index](README.md) for all guides.

- [Loading configuration](#loading-configuration)
- [Bundled Ruby libraries](#bundled-ruby-libraries)
- [Filesystem APIs](#filesystem-apis)
- [Configuration DSL](#configuration-dsl)
- [Key bindings](#key-bindings)
- [Keymaps and modes](#keymaps-and-modes)
- [Commands and object model](#commands-and-object-model)
- [Native action discovery and advice](#native-action-discovery-and-advice)
- [Selection overlays](#selection-overlays)
- [Read-only panels](#read-only-panels)
- [Text input and completion](#text-input-and-completion)
- [Runtime events](#runtime-events)
- [Extension hooks and command advice](#extension-hooks-and-command-advice)
- [Window bars](#window-bars)
- [Host APIs](#platform-clipboard-environment-and-processes)
- [Timers and task ownership](#timers-and-task-ownership)
- [Resource scopes](#resource-scopes)
- [Cooperative work](#cooperative-work)
- [Loading Ruby libraries](#loading-ruby-libraries)
- [Themes](#themes)
- [Live Ruby console](#live-ruby-console)
- [Callback execution model](#callback-execution-model)

## Bundled Ruby libraries

The embedded runtime includes `mruby-error`, `mruby-errno`, `mruby-io`, and
`mruby-dir`, plus the `stdlib`, `stdlib-ext`, `math`, and `metaprog` core
gemboxes. Their APIs are available directly in configuration, callbacks,
commands, required libraries, and the live console; no `require` call is needed.

| Group | Included capabilities |
| --- | --- |
| Collections and iteration | `Set`, `Enumerator`, `Enumerator::Lazy`, `Enumerator::Chain`, and extended `Array`, `Hash`, `Enumerable`, and `Range` methods |
| Objects and control flow | `Fiber`, `ObjectSpace`, `catch` / `throw`, and extended `Object`, `Kernel`, `Module`, `Class`, `Numeric`, `Symbol`, and top-level methods |
| Standard data types | `Struct`, `Data`, `Time`, `Random`, `Array#pack`, `String#unpack`, and `sprintf` |
| Filesystem | `IO`, `File`, and `Dir`, including file I/O, path helpers, directory listing, and globbing |
| Math | `Math`, `Rational`, `Complex`, and multi-precision integers |
| Metaprogramming | source compilation and `eval`, `Binding`, `Proc#binding`, reflection, `Method`, and `UnboundMethod` |

This includes runtime method definition and reflection such as
`define_singleton_method`, `send`, `public_send`, `methods`, `instance_methods`,
`instance_variable_get` / `instance_variable_set`, `class_exec`, `module_exec`,
`Class#subclasses`, `Object#method`, `Module#instance_method`, `Method`, and
`UnboundMethod`. It also includes object helpers such as `tap`, `then`,
`itself`, and `instance_exec`. These are mruby APIs and can differ from the
corresponding CRuby version.

For example, a Ruby library can generate methods and retain a callable method object:

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

`mruby-socket` and `mruby-task` are not bundled. Neither are `mruby-sleep`,
`mruby-exit`, command binaries, or test gems. The vendored POSIX and Windows
amalgamations use mruby's corresponding filesystem HAL. Socket access and an
additional task scheduler remain outside the runtime model.

## Filesystem APIs

General-purpose filesystem work uses Ruby APIs rather than application-specific
wrappers. The following methods are available to configuration, required libraries,
callbacks, commands, and the live console:

- `File.read(path, length = nil, offset = 0, mode: "r")`
- `File.write(path, data, offset = nil, mode: "w")`
- `File.open`, `File.delete`, `File.rename`, `File.exist?`, `File.file?`,
  `File.directory?`, `File.size`, and `File.mtime`
- `File.basename`, `File.dirname`, `File.extname`, `File.join`,
  `File.expand_path`, and `File.realpath`
- `Dir.entries`, `Dir.children`, `Dir.glob`, `Dir.pwd`, `Dir.mkdir`,
  `Dir.chdir`, and `Dir.rmdir`

`Dir.glob(pattern)` accepts a String or an Array of Strings and supports `*`,
`**`, `?`, and bracket character classes. It returns sorted paths; with a block,
it yields each path and returns `nil`. mruby 4.0 does not provide `File::Stat`,
so use `File.size`, `File.mtime`, and the file/directory predicates instead.
I/O errors use mruby's `SystemCallError` / `Errno` exceptions.

```ruby
directories = Dir.glob(File.join(context.pane.cwd || Dir.pwd, "*"))
                 .select { |path| File.directory?(path) }

unless directories.empty?
  Toyoterm.select(title: "Terminal directory", items: directories) do |path, context|
    context.pane.split(:right, cwd: path) if path
  end
end
```

Ruby configuration and required libraries are trusted local code. They can read, write,
and delete files and launch processes with the same permissions as the
toyoterm process. They are not sandboxed.

File and directory methods are synchronous. Reading from a network filesystem,
walking a very large tree, or doing frequent filesystem work in bar or event
callbacks blocks the single scripting runtime until the operation completes.
It does not block PTY parsing or rendering, but it delays later Ruby work.

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
`config.bind(chord)` with `config.keys.key(chord).run`, `focus` with `activate`,
and `create_window` with
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
| `key_sequence_timeout_ms` | `1000` | Integer from 1 through 600,000 milliseconds between accepted chords in a multi-step key sequence. `nil` waits until completion or cancellation. Changing the registry/configuration cancels a pending sequence. |
| `theme` / `theme=` | `nil` | Name registered with `Toyoterm.theme`. An unknown or empty name rejects the configuration. |

### `config.font`

| Setting | Default | Validation |
| --- | --- | --- |
| `family` | `"monospace"` | Non-empty font family name; installation is not validated. |
| `fallback` | `[]` | Array of at most 32 non-empty, unique family names; it must not repeat `family`. |
| `size` | `14.0` | Positive finite number. |
| `weight` | `400` | Integer from 1 through 1000. |

Unknown font families are skipped by the font system. Use exact installed family
names, particularly for Nerd Fonts.
The generic `monospace` family resolves to the installed platform default before
shaping. For missing glyphs, configured fallback families are tried in order,
then the platform's script-specific and common fallbacks. Japanese locales such
as `ja-JP` use Japanese CJK fallbacks. Terminal cell positions remain fixed even
when a fallback font has different glyph advances.

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
renderer. Large compressible wallpaper pixels are held in lossless compressed
CPU storage and expanded in bounded row chunks directly into one GPU upload
buffer; no full decoded CPU image is retained. Resolution, color, and PNG alpha
are preserved. Reload rereads the file, including changes at the same path. Live
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

### `config.osc`

OSC permissions are grouped under `config.osc`. It accepts a block or returns
the same object for direct assignment:

```ruby
config.osc do |osc|
  osc.notifications = true
  osc.downloads = "/absolute/path/to/downloads"
  osc.uploads = "/absolute/path/to/shared-files"
end
```

| Setting | Default | Accepted values and effect |
| --- | --- | --- |
| `clipboard` | `false` | Boolean; enables OSC 52 and iTerm2 clipboard writes. |
| `notifications` | `false` | Boolean; enables OSC 9, 99, and 777 desktop notifications. |
| `attention` | `false` | Boolean; enables iTerm2 attention requests. |
| `open_url` | `false` | Boolean; enables iTerm2 URL launches. |
| `focus` | `false` | Boolean; enables iTerm2 focus requests. |
| `downloads` | `nil` | Existing absolute directory as a String enables iTerm2 and Kitty downloads; `nil` disables them. |
| `uploads` | `nil` | Existing absolute, non-symlink directory as a String enables Kitty uploads; `nil` disables them. |
| `background_image` | `nil` | Existing absolute, non-symlink directory as a String enables iTerm2 wallpaper changes; `nil` disables them. |

Boolean settings reject other types with `TypeError`. Directory settings reject
non-String, non-`nil` values with `TypeError`, an empty String with
`ArgumentError`, and relative paths during config validation. A failed
configuration evaluation or validation rolls back the whole change. Successful
reloads apply the same runtime behavior and limits described below. The
`config.behavior` settings remain available; both APIs read and write the same
underlying values, and the last assignment to a value takes effect. The
directory getters return `nil` when their permission is disabled.

### `config.behavior`

| Setting | Default | Validation |
| --- | --- | --- |
| `scroll_lines` | `3` | Positive finite number of lines per mouse-wheel step. |
| `copy_on_select` | `false` | Boolean. |
| `allow_osc52_copy` | `false` | Boolean opt-in allowing terminal output to replace the system clipboard through OSC 52, one-shot iTerm2 OSC 1337 `Copy=:`, or a general `CopyToClipboard=` / `EndCopy` text capture. Clipboard reads and iTerm2's named `rule`/`find`/`font` buffers remain disabled. Decoded or captured payloads over 64 KiB are rejected. Applies to existing panes after a successful reload and cancels an active capture when disabled. |
| `allow_osc_notifications` | `false` | Boolean opt-in for OSC 9, OSC 99, and OSC 777 desktop notifications. OSC 99 occasions, urgency, every standard sound, the eight universal named icons, up to three bounded buttons, bounded transmitted PNG/JPEG/GIF icons, explicit close, and guaranteed positive expiry are honored on Linux, macOS, and Windows. Universal named icons use the Linux icon theme or built-in 48×48 fallbacks on macOS/Windows; other safe names are forwarded only on Linux. Transmitted icons are limited to 1 MiB, 512×512 pixels, and a 16-entry cache. All three desktop platforms report requested activation, button, and close events to the originating pane with a 128-listener bound and suppress replacement close reports; macOS programmatic closure explicitly cancels its listener. Assembled title/body fields over 4 KiB or containing control characters are rejected; each pane is limited to one notification every two seconds. Applies immediately after a successful reload. |
| `allow_osc_attention_requests` | `false` | Boolean opt-in for OSC 1337 `RequestAttention=yes`, `once`, `no`, and `fireworks`. The first three map to the platform's indefinite, one-shot, and cancel attention hints; `fireworks` renders a 350 ms cursor-local burst. Applies immediately after a successful reload. |
| `allow_osc_open_url` | `false` | Boolean opt-in allowing OSC 1337 `OpenURL=:` to launch a base64-encoded URL without a user gesture. Only `https`, `http`, and `mailto` URLs accepted by the normal 2,048-byte URL validator are allowed, and requests are limited to one per pane every two seconds. Applies immediately after a successful reload. |
| `allow_osc_file_downloads` | `false` | Boolean opt-in for iTerm2 OSC 1337 `File=inline=0` / multipart downloads and Kitty OSC 5113 simple send sessions for regular files, directories, symbolic links, and hard links. It has no effect unless `osc_download_directory` is also set. Applies immediately after a successful reload and drops active Kitty sessions when disabled. |
| `osc_download_directory` | `""` | Empty disables the destination. A nonempty value must be an absolute path to an existing directory. Download names have path components and portable filename hazards removed, existing files are never overwritten, each decoded transfer is limited to 32 MiB, and at most 16 writes can wait on the single background worker. Kitty filesystem trees are staged and then published below a collision-safe `Transferred files` directory. Link targets are restricted to that transferred tree. |
| `allow_osc_file_uploads` | `false` | Boolean opt-in allowing Kitty OSC 5113 receive sessions to read local files. It has no effect unless `osc_upload_directory` is also set. Applies immediately after a successful reload and drops active receive sessions when disabled. |
| `osc_upload_directory` | `""` | Empty disables uploads. A nonempty value must be an absolute path to an existing, non-symbolic-link directory. Remote absolute and `~/` names are rebased below this fixed root. Traversal, platform prefixes, absolute or escaping symbolic links, special files, unknown data requests, and paths changed to escape the root are rejected. Listings are limited to 64 entries and 32 MiB per session; reads and optional zlib compression run on the bounded background transfer worker in 4 KiB protocol chunks. |
| `allow_osc_background_image` | `false` | Boolean opt-in allowing iTerm2 OSC 1337 `SetBackgroundImageFile=` to replace or clear the window wallpaper. It has no effect unless `osc_background_image_directory` is also set. Reloading configuration restores the configured wallpaper. |
| `osc_background_image_directory` | `""` | Empty disables OSC wallpaper reads. A nonempty value must be an absolute path to an existing, non-symbolic-link directory. Decoded relative, `/...`, and `~/...` protocol paths are rebased below this fixed root. Traversal, platform prefixes, directories, missing files, and symbolic links resolving outside the root are rejected. PNG/JPEG decoding uses the bounded background transfer worker and the normal 8192×8192 / 256 MiB decoded-image limits. |
| `allow_osc_focus_requests` | `false` | Boolean opt-in allowing iTerm2 OSC 1337 `StealFocus` and `Disinter` to ask the platform window manager to focus the toyoterm window. Requests are limited to one per pane every two seconds, and the platform or window manager may reject them. Applies immediately after a successful reload. |

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
bindings take priority over logical bindings. For a shifted symbol such as `$`,
`key("$")` matches the produced logical symbol; an explicit `SHIFT+$` binding
still takes priority.

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
| `move_visual_selection(direction)` | `:left`, `:right`, `:up`, `:down`, `:line_start`, `:line_end`, `:word_forward`, or `:word_backward` |
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

### Multi-step key sequences and inspection

`config.keys.sequence(*chords)` returns the same binding object as the key
helpers and supports all static actions, `command(name)`, and `run` callbacks.
Provide between 2 and 32 String chords, for example:

```ruby
Toyoterm.configure do |config|
  config.key_sequence_timeout_ms = nil
  config.keys.sequence("CTRL+X", "CTRL+T").new_tab
  config.keys.sequence("CTRL+X", "R", "B").split(:right)
  config.keys.sequence("CTRL+X", "?").run { Toyoterm.key_help(prefix: "CTRL+X") }
  config.keys.sequence("CTRL+H", "B").run { Toyoterm.key_help }
end
```

Chord names are case-insensitive and use the same modifier/key notation as
single-key bindings, including `PHYSICAL:KEYH`. Each chord is nonempty, at most
128 UTF-8 bytes, and contains no ASCII whitespace/control characters. Use
`SPACE` for the space key. Leader chords (`LEADER+...`) cannot be embedded in
these sequences. `ESCAPE` and `CTRL+G` are reserved for cancellation after the
first chord and cannot be registered as suffixes. Invalid chord types raise
`TypeError`; invalid values or lengths raise `ArgumentError` before registration.

Multiple sequences can share a prefix. An exact binding cannot also be a
prefix of another binding: defining `CTRL+X` and `CTRL+X F`, or `CTRL+X R` and
`CTRL+X R B`, raises `ArgumentError` in either registration order. A sequence
whose first chord equals the configured leader also raises `ArgumentError` in
either order. Unbind a conflicting definition before replacing it. These
checks cover static and Ruby bindings together.

The main thread resolves prefixes and static completions without invoking Ruby.
Each accepted nonfinal chord is consumed and restarts the configured interval.
An invalid suffix, Escape, or Ctrl+G cancels the pending sequence and is consumed.
After a timeout, the next key goes through ordinary input/binding handling.
Repeated key events cannot advance a sequence or extend the interval. Keys
consumed by the sequence remain suppressed through repeats until release.
Physical candidates retain priority over logical candidates, including when
one resolves to a single-key binding and the other to a sequence prefix.

Focus loss, a mouse click, IME preedit/commit/disable, Pane/Workspace activation,
opening a selection/search overlay, or applying a changed configuration/registry
cancels pending sequence state. A pending sequence also checks its originating
Pane before accepting a suffix. The existing leader behavior remains separate:
an unmatched leader suffix continues through normal input handling.

`config.keys.unbind(["CTRL+X", "CTRL+T"])` removes a sequence and returns
whether it existed. A space-separated normalized String is also accepted by
`unbind`, such as `"CTRL+X CTRL+T"`. Successful live edits are mirrored to the
native resolver in the enclosing request's normal transaction. Exceptions and
invalid configuration restore the previous bindings and timeout setting.

| Method | Return value |
| --- | --- |
| `Toyoterm.key_bindings(pane: Toyoterm.current_pane)` | Frozen effective binding Hash for a Pane; `pane: nil` selects globals |
| `config.keys.bindings` | Frozen Hash of the base configuration's own bindings, in sorted order |
| `Toyoterm.describe_key(key, pane: Toyoterm.current_pane)` | Effective description Hash for the selected scope, or `nil` when unbound |
| `config.keys.describe(key)` | Base configuration's own description, or `nil`; accepts a chord/space-separated String or an Array of 2–32 chords, as does `Toyoterm.describe_key` |
| `Toyoterm.command_keys(name)` | Frozen sorted Array of keys directly bound to the named Ruby command; unknown names return an empty Array |
| `Toyoterm.key_help(prefix: nil)` | Opens a searchable, read-only key-binding selector; returns `nil` |

Descriptions have `:key` and `:kind`. A `:native` or `:command` entry also has
`:action` (a Symbol, `:user_command` for a named Ruby command) and `:argument`.
A `:ruby` entry has `:source` and `:line`, each potentially `nil` without mruby
debug information. Description strings are frozen. `command_keys` only finds
direct `.command(name)` bindings; it does not inspect arbitrary Ruby code.
Names for that method accept Strings or Symbols with the usual identifier
validation. Lookup keys accept Strings or sequence Arrays; invalid inputs use
the same `TypeError` / `ArgumentError` rules as registration.

`key_help` optionally filters to an exact normalized key prefix and its
descendants. `prefix` must be a nonempty String, or `nil`. Entries show the
native action, named command and its description, or a Ruby callback label.
Newlines in command descriptions are displayed as spaces. Choosing an entry
only closes help and never runs its action. Empty results are a no-op. The
selection overlay's loading restriction, item/size limits, input handling, and
reload lifecycle apply. Feature detection uses `:key_sequences` and
`:key_inspection` with `Toyoterm.supports?`.

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
  key("w").move_visual_selection(:word_forward)
  key("b").move_visual_selection(:word_backward)
  key("y").yank_selection
end
```

### Dynamic bindings

`config.keys.key(chord).run { |context| ... }` invokes Ruby with a
`Toyoterm::CallbackContext`. `Toyoterm::KeyBindingContext` and
`Toyoterm::CommandContext` are aliases of the same class. They expose
`context.workspace`, `context.window`, `context.tab`, and `context.pane`,
captured when the callback starts. The first three identify the
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
writes. Use `config.keys.unbind(chord)` to remove an existing static or dynamic binding;
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

Outside an active transient keymap, unmatched keys bypass mruby and go directly
to the terminal input encoder. If a
callback raises, all commands and clipboard writes queued by that callback are
discarded and the terminal keeps running.

## Keymaps and modes

Named keymaps layer reusable bindings over the base `config.keys`. They remain
ordinary Ruby objects loaded through `require`; no plugin loader is involved.
Use `Toyoterm.supports?(:keymaps)` and `Toyoterm.supports?(:modes)` for feature
detection. The [navigation example](../examples/modes.rb) toggles a global
minor mode with Alt+N and exits through an Escape binding in that mode.

### Keymap definitions and activation

`Toyoterm.keymap(name, parent: nil) { ... }` defines and returns a
`Toyoterm::Keymap`. The block uses the same binding DSL as `config.keys` and
may accept a `keys` argument. `parent` accepts an existing Keymap or its
String/Symbol name. `Toyoterm.keymap(name)` retrieves a definition;
`Toyoterm.keymaps` returns a frozen sorted Array of names. Names use ordinary
nonempty String/Symbol identifier validation. Duplicate names, unknown names,
an unregistered parent, or a parent supplied without a block raise
`ArgumentError`. A definition block's failure rolls back its host-managed
effects even if its caller rescues the exception.

| Keymap method | Result |
| --- | --- |
| `name`, `id`, `parent` | Frozen name, monotonic VM-local ID, parent Keymap or `nil` |
| `keys { ... }` / `keys` | Edit or retrieve its own `KeysConfig`; supports unbinding and multi-step sequences |
| `bindings` | Frozen effective description Hash including inherited entries |
| `activate(pane: nil)` | A removable `Toyoterm::Registration` of kind `:keymap`; defaults to global; supply a Pane for local activation |
| `activate_transient(pane: Toyoterm.current_pane, once: true)` | Runtime-only exclusive transient activation; removable `Registration` of kind `:transient_keymap` |

Removing an activation reveals the layers below it. Multiple activations of the
same map have independent handles. Maps can be edited after activation; changes
become native in the enclosing successful request. A child overrides the exact
key and any conflicting ancestor/descendant binding in its parent, while
unrelated suffixes of a shared prefix are inherited. The same rule applies
between active layers. For example, a child `CTRL+X` masks all parent `CTRL+X …`
bindings, while a child `CTRL+X F` preserves a parent `CTRL+X R` binding.
Within one map, ambiguous definitions still raise the normal prefix-conflict
error. Named layers can override the base leader chord with a binding or a
sequence prefix; removing that activation restores the base leader. Once a
base leader sequence has started, its suffix is resolved before a new prefix
can start. The base `config.keys` leader-conflict check remains unchanged.

Binding descriptions from named maps also have `:keymap` and `:keymap_id`,
identifying the original definition even when inherited. Effective descriptions
have `:layer`, a positive rank for an active map; the base configuration has no
layer rank. These are immutable description snapshots. Key help and
command-to-key lookup use the current Pane unless `pane:` explicitly selects
a Pane or `nil` for global layers. Wrong target types raise `TypeError`, stale
Panes raise `Toyoterm::InvalidHandleError`, and the startup placeholder raises
`RuntimeError`. Activate Pane-local maps in runtime commands/events rather
than while evaluating configuration. Global activation is valid in configuration.
Reload replaces definitions and activations; failed reload preserves them.

```ruby
base = Toyoterm.keymap(:tools) { key("z").toggle_zoom }
tools = Toyoterm.keymap(:extra_tools, parent: base) do
  key("c").copy_selection
end

activation = tools.activate # global
# Later: activation.remove
```

### Transient keymaps

Capability: `Toyoterm.supports?(:transient_keymaps)`.
`map.activate_transient(pane: Toyoterm.current_pane, once: true)` temporarily
captures keyboard commands using a registered Keymap and its inherited
bindings. `pane:` accepts a live Pane or `nil` for global activation; `once:`
must be Boolean. Activation returns a `Toyoterm::Registration` with `id`,
`name`, `kind: :transient_keymap`, `active?`, and `remove`. Removal returns true
only while the activation exists. Activations are independent even when they
share a map. At most 64 can exist in a VM.

Local transients take precedence over global transients. Within each scope,
newest activation wins. Only the top applicable transient receives input;
unassigned keys do not fall through to lower maps or to the terminal. A finished
map reveals the next map or normal bindings. Panes without that local map use
their own stack; global maps follow focus across Panes. Search and selection/
input overlays retain input precedence over transients.

With `once: true`, a completed binding consumes the activation before dispatch.
With `once: false`, bindings can repeat until cancellation or manual removal.
Escape, Ctrl+G, or an unassigned nonmodifier key closes the top map and consumes
that key without sending it to the shell or dispatching a lower binding. A
standalone modifier press preserves the map/prefix unless explicitly bound by
its physical key. Auto-repeat does not dispatch transient operations or extend
sequence deadlines. Prefixes use `key_sequence_timeout_ms`; expiration resets
the prefix and evaluates the next key from the map root. Focus loss, Pane
switches and registry changes clear pending prefixes; focus loss leaves the
activation itself in place. Physical candidates precede logical candidates in
the same map.

Empty maps and bindings starting with Escape/Ctrl+G raise `ArgumentError`.
Unknown/foreign maps follow normal Keymap validation. Wrong target/Boolean
types raise `TypeError`, stale Panes raise `InvalidHandleError`, and startup
placeholder Panes or activation during source loading raise `RuntimeError`.
Define maps while loading, then activate them from runtime commands, keys,
events or tasks. Editing an active map updates its native keys on successful
request completion; making it empty or adding a reserved cancellation binding
fails the request and restores its prior definition.

`Toyoterm.transient_keymaps(pane: Toyoterm.current_pane)` returns a frozen Array
of frozen `{id:, keymap:, pane:, once:}` descriptions in priority order.
`keymap` is its frozen definition name, and `pane` is a numeric Pane ID or nil.
Use `pane: nil` to inspect just globals. `Toyoterm.key_bindings`, `describe_key`,
`command_keys`, `action_keys`, and `key_help` inspect the top transient's
bindings while active. Those descriptions include `:transient` (activation ID)
and normal definition/source metadata. `map.bindings` always describes its own
inherited definition, independently of activation.

Native matching consumes one-shots immediately, so delayed Ruby work or older
snapshots cannot re-enable them. A typed request delivers the activation ID,
completed key (or cancellation), and originating Pane to the script owner.
Native actions pass through action advice; named commands use command advice
and argument collection; Ruby callbacks retain the defining keymap, including
inherited callback owners. Removed registrations and requests from an old VM
after successful reload are ignored. A closed originating Pane does not run
its callback; accepted one-shots are still consumed. Failed reload preserves
the stack.

Creation, manual removal, edits and callback effects use the normal host
transaction. Keyboard consumption happens before that checkpoint: a failed
one-shot callback does not resurrect the map, while a failing repeated
operation leaves its map active. Callback host effects and any maps it creates
are rolled back on failure. Ruby objects and external effects remain outside
rollback. Mode-owned activations are removed on disable/close/reload; their
callbacks inherit the owner's resource scope, including typed command prompts,
timers, async tasks and registrations they create. Pending delivery after
removal is ignored. Other activations remain independent.

```ruby
split_keys = Toyoterm.keymap(:choose_split) do
  key('j').split(:down)
  key('l').split(:right)
end
Toyoterm.command(:choose_split) { split_keys.activate_transient }
Toyoterm.configure { |c| c.keys.alt('s').command(:choose_split) }
# Alt+S, then j/l chooses one split. Escape cancels.
```

The [complete example](../examples/transient_keymaps.rb) also uses a global
`once: false` map for repeated Pane focus/zoom operations with Alt+R.

### Major and minor modes

`Toyoterm.mode(name, keymap:, kind: :minor) { |enabled, context| ... }` defines
and returns a frozen `Toyoterm::Mode`. The lifecycle block is optional.
`keymap:` accepts a registered Keymap or its name. `kind` must be `:major` or
`:minor`; other values, duplicate names, and unknown definitions raise
`ArgumentError`. Identifier types follow the keymap rules. `Toyoterm.mode(name)`
retrieves a definition and `Toyoterm.modes` returns frozen sorted names.

| Mode method | Result |
| --- | --- |
| `name`, `keymap`, `kind` | Definition name, Keymap, and `:major` / `:minor` |
| `enable(pane: Toyoterm.current_pane)` | `true` on activation, `false` if already enabled in that exact scope |
| `disable(pane: Toyoterm.current_pane)` | `true` on deactivation, `false` if already disabled in that scope |
| `enabled?(pane: Toyoterm.current_pane)` | Boolean for the exact scope |
| `Toyoterm.active_modes(pane: Toyoterm.current_pane)` | Frozen sorted Array of names enabled in the exact scope; defaults to the current Pane; use `pane: nil` for globals |
| `Pane#enable_mode(name)`, `Pane#disable_mode(name)`, `Pane#modes` | Pane-local convenience methods for these operations |

Minor modes can be global (`pane: nil`) or Pane-local. Major modes require a
runtime Pane, and at most one major mode is enabled per target. Enabling a new
major mode disables the previous one in the same transaction. Minor modes can
coexist. Global minor modes apply across Pane switches; Pane-local modes stay
with their Pane. Definitions are independent of their instances.

From lowest to highest, resolution uses base bindings, global layers, then
Pane-local layers. Within each scope the order is major mode, minor modes, then
explicit Keymap activations. Within each category, later activations win.
Layer rank wins before physical/logical priority; physical candidates win ties.
Shared prefixes can fall back to unrelated suffixes in lower layers. Search and
selection/input overlays own their input while open. Lookup remains native.

The lifecycle block receives `true` on enable and `false` on disable, plus a
captured `CallbackContext`. Its return value is ignored. Reentrant transitions
of the same instance raise `RuntimeError`. An exception restores all
host-managed changes and the previous enabled modes, including a major mode
being replaced. Arbitrary Ruby object mutations and external side effects
remain outside rollback.

Registrations created directly during a mode's lifecycle block (commands,
events, hooks, advice, explicit/transient Keymap activations, timers, cooperative work, async tasks, and input/selection/panel overlays) are owned by that mode
instance and removed in reverse order when it disables. Registrations created
inside an explicit `ResourceScope#within` belong to that scope instead. A new
scope created inside the lifecycle is itself mode-owned; a pre-existing scope
retains its independent lifetime when re-entered. Registrations created
during its disable callback are also removed. Cleanup does not restore a
command replaced with `replace: true`. Registrations created later by an event
or command callback are independent; nested mode instances are independent as
well. A mode-owned timer or async completion inherits its owner's resource
scope, including registrations/tasks created by that callback. Disabling the
mode cancels pending timer/async delivery and removes those descendants. An
already-running child process may continue until it exits. Keymap edits,
arbitrary Ruby object changes, and external effects remain outside ownership.

Closed-Pane instances and activations are pruned on the next script registry
refresh, including their owned command/event registrations in that same native
snapshot. Their disable block is not called because its target is already stale.
Successful reload drops all mode state with the old VM; failed reload preserves
it. Scope changes and new registry snapshots cancel pending key sequences.

```ruby
tools = Toyoterm.keymap(:pane_tools) do
  key("z").toggle_zoom
  key("ESCAPE").run { |context| context.pane.disable_mode(:pane_tools) }
end
Toyoterm.mode(:pane_tools, keymap: tools) do |enabled, context|
  context.pane.badge = enabled ? "TOOLS" : nil
end
Toyoterm.configure do |config|
  config.keys.alt("t").run { |context| context.pane.enable_mode(:pane_tools) }
end
```

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
`:down`; `move_visual_selection` requires one of its eight documented motions.
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

`MuxWindow` is a mux object. Each GUI process displays the active mux window in
one OS window; `Workspace#new_window` does not create another OS window in that
process. Running `toyoterm` again starts a separate GUI process and OS window.
Multiple OS windows within one GUI process remain deferred.

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

`Workspace#new_window`, `MuxWindow#new_tab`, and `Pane#split` accept an optional
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

### Command discovery, composition, and palette

`Toyoterm.command(name, replace: false, description: nil, arguments: []) { |context, values| ... }`
also accepts an optional String description (including an empty String).
Names accept Strings or Symbols. Invalid description types raise `TypeError`;
NUL bytes raise `ArgumentError` before replacing or registering a command.

| Method | Return value |
| --- | --- |
| `Toyoterm.commands` | Frozen Array of registered Ruby command names, sorted lexically |
| `Toyoterm.command_info(name)` | Frozen Hash with `:name`, `:description`, `:source`, `:line`, and `:arguments`, or `nil` for an unknown name |
| `Toyoterm.command_definition(name)` | Frozen `{name:, source:, line:}` Hash, or `nil` for an unknown command or unavailable source |
| `Toyoterm.call_command(name, context: nil, arguments: {})` | Validates arguments and returns the registered block's return value |
| `Toyoterm.prompt_command(name, context: nil, arguments: {})` | Collects missing arguments interactively, then invokes the command; returns `nil` |
| `Toyoterm.command_arguments` | Current command's frozen argument Hash, or a frozen empty Hash outside a command |
| `Toyoterm.command_palette` | Opens a searchable command selector; returns `nil` |

Metadata strings are frozen. Description defaults to `nil`. Source identifies
the main configuration or required Ruby file while it loads, even without mruby
debug information. In-memory configurations retain their virtual filename, such
as `(config)`. Runtime-generated commands can have nil source; line may be nil
without debug information and is never guessed. Discovery covers Ruby commands, not native
actions; native actions remain available through `context.actions`.

`call_command` executes immediately on the script thread. Nested calls inherit
the calling command's context by default; otherwise the current Pane context
is used. An explicit `context:` must be a `Toyoterm::CallbackContext` or raises
`TypeError`. Stale context handles raise `Toyoterm::InvalidHandleError`.
An unknown command raises `ArgumentError`; nesting beyond 64 active calls raises
`RuntimeError`. The command stack is restored after success or failure.

Each call checkpoints host-managed configuration, registrations, queued native
commands, badges, bars, async requests, and logs. A raised exception restores
that checkpoint even if the caller rescues it. Native commands are still applied
only after the enclosing host request succeeds. Arbitrary Ruby objects, file
I/O, and external process side effects are not rolled back.

The palette uses the selection overlay's size limits and runtime-only loading
restriction. An empty command registry is a no-op. Escape cancels without
executing a command. The chosen command receives the context captured when the
palette opened; stale handles are rejected. Removing or replacing a displayed
command before choosing it raises `ArgumentError` instead of running a different
registration. Reload closes the palette along with other selection overlays.
Use `Toyoterm.supports?(:command_palette)` for feature detection.

```ruby
Toyoterm.command(:git_status, description: "Show Git status in this pane") do |context|
  context.pane.send_text("git status\n")
end
Toyoterm.command(:project_status) { Toyoterm.call_command(:git_status) }

Toyoterm.configure do |config|
  config.keys.alt("x").run { Toyoterm.command_palette }
end

# In the live console:
Toyoterm.commands
Toyoterm.command_info(:git_status)
```

### Command help and definitions

Capability: `Toyoterm.supports?(:command_help)`.
`Toyoterm.command_help(name = nil) { |definition, context| ... }` returns nil
and opens searchable help through the native selection overlay. With a
String/Symbol name, it opens that command's details directly. With nil, it first
lists registered Ruby commands in lexical order, with numbered labels and
descriptions; choosing one opens its details. An empty registry is a no-op.
Unknown names raise `ArgumentError`; invalid name types raise `TypeError`.
It is runtime-only and follows `select`'s single-pending-overlay and size limits.
Source-loading calls, including calls in the main configuration, raise
`RuntimeError`. Define a command or binding that opens help instead.

Details show the description, effective direct keys for the originating Pane,
argument types/requirements/defaults/ranges/choices/completion/history, advice
registration IDs/kinds, and definition location. Filtering, movement and paging
stay native. Opening/reading help never invokes the inspected command or its
advice. Escape cancels; selecting an ordinary detail row closes help. Details
are a metadata snapshot. Source locations can be virtual labels and line
numbers can be nil; source availability does not imply that a file still exists.

An optional block runs only when a known Definition row is selected. It receives
the full frozen `command_definition` Hash and originating `CallbackContext`.
Core help does not open, read, or modify that file. A trusted library can use
the callback for its navigation workflow. Names and source strings are frozen;
the block receives untruncated values. Without a block, selecting the row just
closes help. Unknown/unavailable source does not invoke it.

Before opening details from the list, and before calling the definition handler,
the command must still be the original registration; removal/replacement raises
`ArgumentError`. Captured handles are validated, so a closed target raises
`InvalidHandleError`. Handler host effects participate in ordinary callback
rollback; arbitrary Ruby/external effects do not. Help is consumed before
delivery, so a failed handler does not resurrect it. Mode-owned help is closed
on disable/close/reload using the normal overlay lifecycle.

Display values use escaped String representations and at most 160 characters
each, with an ellipsis when shortened. Details show at most 32 keys, 32 advice
entries and 16 choices per field, with notices for omitted entries. Full metadata
remains available from `command_info`, `command_keys`, `command_advice`, and
`command_definition`. The help selector's aggregate limits still apply.

```ruby
Toyoterm.configure do |config|
  config.keys.alt('h').run do
    Toyoterm.command_help do |definition, context|
      location = definition[:source]
      location += ':' + definition[:line].to_s if definition[:line]
      Toyoterm.clipboard.write(location)
      context.pane.badge = 'Definition location copied'
    end
  end
end
# Runtime direct help/inspection:
Toyoterm.command_help(:label_pane)
Toyoterm.command_definition(:label_pane)
```

The [runnable example](../examples/command_help.rb) also registers a terminal
status command so its definition, description and key can be inspected.

### Typed command arguments

Capability: `Toyoterm.supports?(:command_arguments)`. Declare up to 16 fields
with `arguments: [{name: :label, type: :string}, ...]`. Definition keys are
Symbols. Names accept nonempty Strings/Symbols and must be unique after String
normalization. Invalid schemas fail before registering/replacing a command.

| Field option | Accepted values and default |
| --- | --- |
| `name:` | Required name; UTF-8, no controls, at most 128 bytes |
| `type:` | `:string` (default), `:integer`, `:number`, or `:boolean` |
| `prompt:` | Single-line UTF-8 prompt title, at most 256 bytes; defaults to name |
| `required:` | Boolean; defaults to true unless `default:` is present |
| `default:` | A value of the declared type; validated against choices/range at registration |
| `choices:` | Array of 1..4096 typed values, deduplicated; accepted values must be included |
| `min:`, `max:` | Finite numeric inclusive bounds, for integer/number fields only; minimum must not exceed maximum |
| `history:` | Named input history (String/Symbol), or `nil` (default) |
| `completer:` | Callable receiving `query, context`, returning candidate Strings; string fields only, mutually exclusive with choices |
| `require_match:` | Boolean for dynamic completion, default false; static choices and booleans always require a candidate match interactively |

Strings are copied/frozen, limited to 4096 UTF-8 bytes and exclude controls.
Integer fields accept only Integers; number fields accept finite Numeric values;
booleans accept only true/false. Optional fields accept `nil`. A missing value
uses its default, or `nil` if optional; missing required values raise
`ArgumentError`. Wrong types raise `TypeError`; unknown options, duplicate
names, invalid ranges/choices and prompt limits raise `ArgumentError`.
Static candidate display also follows `read_string`'s 4096-candidate/4 MiB
limits, including the blank candidate for optional fields.

`command_info[:arguments]` is a frozen Array of frozen normalized field Hashes,
including `:dynamic` (Boolean), excluding the completer callable. This metadata
retains defaults, choices, ranges, prompt and history settings.

`call_command(arguments: hash)` accepts String/Symbol keys, rejects duplicate
normalized or unknown keys, and never prompts. It validates before advice or
body execution. Commands with fields receive `|context, values|`; values is a
frozen Hash keyed by Strings. Commands with no fields still receive exactly
one context argument, preserving strict lambda arity. Nested calls inherit
context, but receive their own values/defaults. `command_arguments` exposes the
current values to before/around/after advice without changing advice signatures;
the previous values are restored on return or exception. Programmatic calls
validate types, ranges and static choices without running dynamic completers.

Key `.command(name)` bindings and both palettes use `prompt_command`. Supplied
partial `arguments:` are validated and skipped; remaining fields are prompted
in declaration order, including fields with defaults. Defaults appear as
initial text. Integer/number inputs use `Integer(text, 10)`/`Float(text)`;
boolean inputs use `true`/`false`. Blank optional input yields `nil` (or an
explicit empty-string/nil default). Clear the initial text to choose nil;
required string fields may accept an empty String. Invalid parsed values log a
warning and reopen the same field with the attempted text; previous values
remain collected. Static/dynamic match rejection follows `read_string` rules;
a dynamic provider must include an empty candidate to allow blank input with
`require_match: true`.

Escape cancels without calling advice/body. No command effects are queued
until all fields are valid. Each input is a separate host transaction: accepted
raw input may enter its named history even if parsing triggers a retry or a
later field is cancelled; body failure rolls back host effects and the final
input's history change. Earlier successful input requests remain committed.
Arbitrary Ruby objects and external effects retain the ordinary trusted-code
semantics. Only one selector/input may be pending, and interactive collection
is runtime-only (not during source loading).

Collection retains the originating context; stale handles or a removed/replaced
command raise instead of invoking another registration. A defining mode, or
the calling owned task/transient callback if there is no defining mode, owns
the prompts and closes them on disable/close/reload. Dynamic providers receive
the originating context and follow the provider-effect rollback rules.

```ruby
Toyoterm.command(:label_pane, arguments: [
  {name: :label, prompt: 'Pane label', history: :labels},
  {name: :zoom, type: :boolean, default: false}
]) do |context, values|
  context.pane.badge = values['label']
  context.actions.toggle_zoom if values['zoom']
end
# Runtime direct call; no input opens:
Toyoterm.call_command(:label_pane, arguments: {label: 'Build'})
# Runtime interactive call; skips label and asks about zoom:
Toyoterm.prompt_command(:label_pane, arguments: {label: 'Build'})
```

## Native action discovery and advice

The named operations available through `config.keys` and `context.actions` can
be inspected and wrapped by independent libraries. Capability checks are
`Toyoterm.supports?(:action_inspection)`, `:action_advice`, and `:operation_palette`.
The [action-policy example](../examples/actions.rb) toggles a Pane's split policy
with Alt+M, requests a right split with Alt+D, and opens the combined operation
palette with Alt+O. Its policy changes that Pane's split direction to down and
suppresses its close action while shell integration reports a running command.

| Method | Return value and behavior |
| --- | --- |
| `Toyoterm.actions` | Frozen sorted Array of native action name Strings |
| `Toyoterm.action_info(name)` | Frozen metadata Hash, or `nil` for an unknown name |
| `Toyoterm.action_keys(name)` | Frozen sorted key Array for that action in the current Pane's effective keymaps; unknown names raise `ArgumentError` |
| `Toyoterm.advise_action(name, kind: :before) { ... }` | Removable `Registration` of kind `:action_advice` |
| `Toyoterm.action_advice(name)` | Frozen Array of frozen `{ id:, kind:, source:, line: }` descriptions in registration order |
| `Toyoterm.operations` | Frozen metadata Array combining native actions (`kind: :action`) and Ruby commands (`kind: :command`), sorted by name then kind |
| `Toyoterm.invoke_operation(kind, name, argument: nil, arguments: {}, context: nil)` | Invokes through the action/advice or Ruby command/advice path; kind must be `:action` or `:command`; native actions use singular `argument:`, Ruby commands use typed `arguments:` |
| `Toyoterm.operation_palette` | Queues a selection of both kinds and returns `nil`; native/Ruby arguments are collected through follow-up input prompts |

Action names accept nonempty Strings/Symbols, case-insensitively. Wrong name
types raise `TypeError`; unsupported names in advice or `action_keys` raise
`ArgumentError` (`action_info` returns `nil` for unknown names). Advice needs a block and a kind of `:before`, `:after`, or
`:around`, otherwise `ArgumentError` is raised without registration. The special
key-binding operation `user_command` is represented through the Ruby command
catalog and `advise_command`, rather than as a native action definition.

`action_info` includes `name` (String), `kind` (`:action`), `description` (String),
`argument_required` (Boolean), `arguments` (frozen Array of accepted Symbols),
`scope` (`:pane` or `:application`, following the existing global-action rules),
and `advice` (the description Array). Native definitions have no Ruby source;
advice descriptions include the declaring source/line when available. Ruby
operation entries retain `command_info` metadata, including typed argument
schemas; `argument_required` is true when any required field lacks a default.
Passing a non-nil singular argument to a Ruby command, or nonempty command
arguments to a native action, raises `ArgumentError`. Native/Ruby definitions
may share a name because their kinds differ.
Mutating descriptions cannot change registrations or keys.

Before/after blocks receive a frozen `Toyoterm::ActionCall` with `name`,
`argument`, and `context`. Name and non-`nil` argument are normalized frozen
Strings. Context is the captured `CallbackContext` with snapshot handles.
Around blocks receive `|proceed, call|`. Calling `proceed.call` runs the next
layer; `proceed.call(replacement)` replaces the argument after normal action
validation. Omitting it suppresses the base operation. To replace an operation
with a different action, queue that other action instead of proceeding.

Before handlers run in registration order, then around handlers nest in that
order (first is outermost), then after handlers run in registration order.
Before/after handlers describe the original request; an inner around handler
sees an argument replaced by an outer layer. After handlers run after the around
chain returns, including when it suppresses the base operation. Return values
are ignored, and action calls return `nil`. The dispatch takes a registration
snapshot: adding/removing handlers affects future invocations, not the current
chain. Advice is global; filter `call.context` to implement a Pane-specific
policy, as the example does. Mode ownership controls registration lifetime.

Continuation calls are synchronous and once-only. Reusing a continuation or
calling it after its block returns raises `RuntimeError`. Calling a new action
from advice enters that action's advice normally. Nesting is limited to 64 active
advised action calls; exceeding it raises `RuntimeError`. Call `proceed` to
continue the same operation without recursively re-entering its advice. The
continuation is not a deferred confirmation handle.

Each advised call checkpoints host-managed settings, registrations, queued
commands, UI, tasks and logs. An exception restores the checkpoint even if the
caller rescues it; queued base work is discarded too. Arbitrary Ruby mutations
and external file/process effects cannot be undone. A failed native-key advice
request is logged and does not fall back to executing the unadvised operation.
Removing advice reveals the original behavior. Registrations created during a
mode lifecycle or owned task are released with that mode; failed transitions
restore them, successful reload replaces them, and failed reload preserves them.

Advice applies to named action calls from Ruby and native key/leader bindings.
The native registry mirrors only which names need advice. A binding without
advice keeps its native dispatch path; an advised action submits a typed request
to the script thread with its captured hierarchy. Ruby returns ordinary typed
action commands, which are validated/applied on the main thread without running
advice again. A changed active Pane does not retarget the queued action; stale
non-global targets fail validation. Slow advice delays Ruby work, while PTY
parsing/rendering remain native.

Advice wraps queuing, so an after handler does not mean the native mutation has
already applied. Use native events to observe the resulting state. Direct
`Pane`/mux methods, IPC domain commands, terminal protocol processing, OS window
close and automatic child exit do not pass through these named action hooks.
These extension points are trusted customization, not a security boundary.

`invoke_operation` defaults to the enclosing command's context or the current
Pane context; an explicit non-`CallbackContext` raises `TypeError`. Native
arguments use normal action validation. Ruby commands currently accept no
operation argument; a non-`nil` one raises `ArgumentError`. Native invocation
returns `nil`; Ruby invocation returns the command's result. Unknown kinds and
commands raise `ArgumentError`. The operation palette shares the runtime-only
selection/input slot, limits, cancellation and reload behavior. It preserves
the originating context and rejects a Ruby command removed/replaced while the
palette was open. Choosing an argument is cancellable and invokes no action
until accepted. The existing `command_palette` continues listing Ruby commands.

```ruby
policy = Toyoterm.advise_action(:split, kind: :around) do |proceed, call|
  proceed.call(:down)
end

Toyoterm.advise_action(:new_tab, kind: :before) do |call|
  Toyoterm.log(:info, "New tab requested from pane #{call.context.pane.id}")
end

# Later: policy.remove
Toyoterm.action_info(:split)
Toyoterm.action_advice(:split)
Toyoterm.operations
```

## Selection overlays

`Toyoterm.select(title: "Select", items:) { |selection, context| ... }` opens a
searchable selection overlay centered in the application window. `items` must
be a non-empty Array containing no more than 4,096 non-empty Strings. Each item
is limited to 4 KiB and all items together are limited to 4 MiB. The optional
String title is limited to 256 bytes. Titles
and items reject NUL bytes and line breaks. The method requires a block, queues
the overlay, and returns `nil`.

Typing filters items by a case-insensitive substring match. Filter input is
limited to 4 KiB; insertions containing control characters or exceeding that
limit are ignored as a whole. Backspace removes a Unicode grapheme. Up/Down move through
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
`Toyoterm.select` is a runtime operation and cannot be called while a required
Ruby source itself is loading; register it inside a command, key, event, or asynchronous
callback instead.
The overlay captures keyboard and IME input while open, so typed characters are
not sent to the PTY. Reloading configuration closes an open overlay because it
replaces the VM and its callback. The result callback runs later on the single
script thread; its configuration changes and native commands use the normal
atomic callback transaction. A callback exception discards its queued work and
is logged without stopping the terminal. Use
`Toyoterm.supports?(:select_overlay)` when supporting older toyoterm builds.

## Read-only panels

Capability: `Toyoterm.supports?(:text_panels)`.
`Toyoterm.show_panel(title: 'Panel', lines:)` opens a read-only modal overlay
and returns a frozen `Toyoterm::Panel` handle. It is for extension output such
as logs, task results and terminal metadata; it does not load files or provide
editable documents. Only one panel, selection or input overlay can be pending.
Creation requires a runtime request and is forbidden while loading Ruby source.
Violations raise `RuntimeError` before opening a panel.

`title:` is a UTF-8 String of at most 256 bytes. `lines:` is an Array of at most
4096 UTF-8 Strings, each at most 4096 bytes, totaling at most 4 MiB. Empty arrays,
empty lines and duplicate lines are preserved. Strings are copied/frozen and
must not contain controls (C0, DEL or C1); supply one String per logical line.
Wrong types raise `TypeError`; invalid encoding, controls and exceeded limits
raise `ArgumentError`. Empty titles render with the native title `Panel`.

| Handle member | Behavior |
| --- | --- |
| `id` | Monotonic VM-local overlay ID |
| `active?` | Boolean indicating that the script runtime still knows this panel is open |
| `update(lines:, title: nil)` | Replaces all displayed lines; optional non-nil title replaces the title. Returns true when queued, false if already inactive |
| `close` / `remove` | Queues closure and releases the pending slot; returns true once, false if inactive |

Updates validate/copy their data before queuing and keep the native filter query
and selected index (clamped to remaining matches). An inactive update returns
false without validating its payload or reopening the panel. All native updates
are ID- and panel-kind checked, so late results cannot overwrite another
overlay. Delivery is serialized with other script commands.

Typing edits only the filter query. Up/Down, PageUp/PageDown, Home and End browse
matching lines; Backspace edits the query. Native filtering and paging never
call Ruby. Enter, Escape or Ctrl+G closes the panel; Enter while an IME preedit
is active follows normal overlay composition behavior. Closing sends only a
dismissal through the typed callback boundary, never the highlighted line.
There is no user result callback. `active?` may remain true until that dismissal
reaches the script owner; updates in the meantime are ignored by the closed
native overlay. The view retains ordinary overlay viewport truncation rules.

Creation, updates and explicit closure participate in host rollback. An
exception discards queued updates and restores any existing panel registration;
arbitrary Ruby objects and external effects remain outside rollback. User
dismissal is consumed before the callback checkpoint and cannot be resurrected.
A mode owns panels created during its lifecycle or owned task callbacks and
closes them on disable/Pane closure. Successful reload closes overlays and drops
their old-VM handles; failed reload preserves them.

```ruby
Toyoterm.command(:task_result) do
  panel = Toyoterm.show_panel(title: 'Task output', lines: ['Waiting…'])
  Toyoterm.after(1) do
    panel.update(title: 'Task complete', lines: ['Done', 'No errors'])
  end
end
```

The [live monitor example](../examples/panels.rb) updates terminal snapshot data
once per second inside a Pane-local mode. User closure disables that mode and
cancels its timer on the next tick; mode disable/Pane closure also closes the
panel. Snapshot strings are escaped for display before passing line validation.

## Text input and completion

`Toyoterm.read_string(title: 'Input', initial: '', completions: [],
completer: nil, require_match: false, history: nil) { |text, context| ... }` opens a single-line
input prompt and returns `nil`. It requires a block (`ArgumentError` without
one) and a runtime command, key, event, asynchronous callback, or console request.
Calling it in configuration or while loading a required source raises
`RuntimeError`. Feature detection uses `Toyoterm.supports?(:input_prompt)`.

| Argument | Accepted values and behavior |
| --- | --- |
| `title` | UTF-8 String, at most 256 bytes; empty uses the native default title |
| `initial` | UTF-8 String, at most 4 KiB; point starts at the end |
| `completions` | Array of at most 4,096 UTF-8 Strings, each at most 4 KiB and at most 4 MiB in total after deduplication; empty Strings and an empty Array are allowed |
| `require_match` | Boolean; requires a candidate or a dynamic provider; the accepted result must be one of its candidates |
| `completer` | Callable object or `nil`; receives a frozen query and captured context, returning an Array of candidate Strings; exclusive with nonempty `completions` |
| `history` | Nonempty String/Symbol identifying a VM-local history, or `nil` to disable history |

Strings reject NUL and Unicode control characters, including tabs and line
breaks. Wrong types raise `TypeError`; invalid encoding, controls, lengths,
counts, or options raise `ArgumentError`. Prompt options and candidate Strings
are copied when requested, so later mutation of the source Array does not
change the native prompt. Static candidates filter entirely in native code.
Dynamic providers run on the script thread through the request boundary described below.

Typing filters candidates using case-insensitive substring matching. An exact
input match is listed first. Up/Down or Ctrl+P/N select a candidate; PageUp/Down
move by ten. Tab replaces the input with the highlighted candidate. Enter
accepts the typed String, including an empty String, when `require_match` is
false; use Tab first to accept a completion in this mode. When `require_match`
is true, Enter accepts the highlighted candidate and stays open if there are
no matches. Escape or Ctrl+G cancels with `nil`.

Left/Right and Ctrl+B/F move over Unicode graphemes; Home/End and Ctrl+A/E move
to input edges. Backspace/Delete remove a grapheme. Ctrl+U clears text before
point and Ctrl+K clears text after it. IME commits and Primary+V insert at point;
Enter during IME preedit does not submit the prompt. Insertions containing
controls or exceeding the 4 KiB input limit are ignored as a whole. Long input
scrolls around point using Unicode display-column widths and the current cell
metrics, preserving grapheme boundaries for Japanese text, combining marks and
emoji sequences. Continuation ellipses indicate clipped text; extremely narrow
views retain only the caret (or no input when there is no available column).
Candidate highlights are omitted when there is insufficient overlay space for
their row. While an extension overlay is open, the host supplies its visible
input cursor as the IME candidate anchor, accounting for horizontal clipping,
preedit and scaled cell metrics. The anchor is bounded to the overlay; after
dismissal, normal terminal-cursor anchoring resumes. The operating system/IME
controls the final placement of the candidate window.
Input is visible; this API has no password/secret mode.

Alt+Up/Down browse the captured history and restore the in-progress draft when
moving past its newest entry. `Toyoterm.prompt_history(name)` returns a frozen
Array of frozen Strings, oldest first; an unknown name returns an empty Array.
`Toyoterm.clear_prompt_history(name)` returns whether a stored history existed.
Both methods validate names with the usual String/Symbol identifier rules.
There are at most 64 named histories, each retaining the last 128 distinct
accepted values. A repeated value moves to the end. Cancellation adds nothing;
accepting an empty String records that String. History is in memory only and
is cleared on successful VM reload; failed reload preserves it.

Prompts, `Toyoterm.select` and read-only panels share one pending slot. Opening another while one
is pending raises `RuntimeError`. The native UI captures keyboard, paste, and
IME input; text does not reach the PTY. Reload closes it. Confirmation
consumes the callback once, then runs it on the script thread with the frozen String
or `nil` and the originating `CallbackContext`, including its captured Pane.
The callback may open a follow-up prompt because the slot is released first.
Callbacks ignore their return values. A raised exception rolls back host-managed
changes, queued work, follow-up prompts, and the history insertion; it does not
reopen the consumed prompt or undo external file/process effects. History reads
inside a successful callback already include the accepted value.

```ruby
Toyoterm.command(:name_pane) do
  Toyoterm.read_string(title: 'Pane name', history: :pane_names) do |text, context|
    context.pane.badge = text unless text.nil?
  end
end

Toyoterm.command(:choose_theme) do
  Toyoterm.read_string(title: 'Theme', completions: Toyoterm.themes,
                      require_match: true, history: :themes) do |theme|
    Toyoterm.configure { |c| c.theme = theme } unless theme.nil?
  end
end
```

### Dynamic completion providers and prompt lifetime

`completer.call(query, context)` runs after the prompt opens and after input
changes. Changes debounce for 80 ms; IME preedit postpones requests until commit.
Only one provider request is outstanding per prompt; intervening edits coalesce.
A slow provider delays other Ruby work, while native terminal input, PTY parsing,
and rendering continue. Providers are ordinary trusted Ruby code and have no
execution-time limit or preemption.

The returned Array uses the same count, UTF-8, control-character and byte limits
as static candidates. It may be empty even with `require_match: true`. Native
code preserves provider results without applying a second substring filter, so
providers can implement fuzzy matching. An exact match is listed first.
Candidate updates carry the prompt ID and input revision; results for old input,
closed prompts, or earlier VMs are ignored. Editing clears previous candidates.
While a required-match prompt is waiting for candidates, Enter cannot accept an
old choice. Free-input prompts can still accept the typed text. Provider errors
are logged and show a failure notice; another edit retries without closing the
prompt. Cursor movement alone does not regenerate candidates.

Providers compute data: host-managed commands, configuration, registrations,
history changes, task creation/cancellation, and mode changes are rolled back
on success as well as failure. Arbitrary Ruby mutations and external I/O remain
outside rollback; this is not a sandbox. Result callbacks use the normal commit
transaction.

Input and selection overlays created within a mode lifecycle or an owned task
inherit that mode. Disabling/replacing the mode or closing its target closes
them without invoking their result block. Failed transitions restore the
pending overlay and discard its queued close. Result callbacks inherit the
resource owner, allowing their child tasks to be cleaned up as well. A consumed
result is not reopened by callback failure. Use
`Toyoterm.supports?(:dynamic_completion)` and `Toyoterm.supports?(:mode_prompts)`
for capability checks.

```ruby
Toyoterm.command(:choose_theme) do
  provider = proc { |query, _context| Toyoterm.themes.select { |name| name.downcase.include?(query.downcase) } }
  Toyoterm.read_string(title: "Theme", completer: provider, require_match: true) do |theme|
    Toyoterm.configure { |c| c.theme = theme } unless theme.nil?
  end
end
```

The [completion example](../examples/completion.rb) traverses directory
candidates and opens a terminal Pane in the chosen directory with Alt+P.

## Runtime events

Register handlers with `Toyoterm.on(name) { |event| ... }`. `Toyoterm::Event`
exposes `name`, `workspace`, `window`, `tab`, `pane`, `title`, `cwd`,
`exit_status`, `width`, `height`, `columns`, and `rows`; unrelated fields are
`nil`.

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
| `window_resized` | `window`, `width`, `height`, `columns`, `rows` |
| `tab_created`, `tab_closed` | `tab` |
| `pane_created`, `pane_closed`, `pane_focused` | `pane` |
| `pane_ready`, `pane_resized` | `pane`, `columns`, `rows` |
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

Toyoterm.on :window_resized do |event|
  Toyoterm.log :info, "window resized to #{event.width}x#{event.height} (#{event.columns} cols, #{event.rows} rows)"
end
```

`prompt_started`, `command_line_started`, `command_started`, and
`command_finished` require the corresponding OSC 133 shell-integration marker.
`command_finished.exit_status` is `nil` when the shell emits a completion marker
without a valid decimal status. Badge changes become visible after a successful
callback and are discarded if it raises. Badge text is drawn in the pane's
upper-right corner; assigning `nil` removes it, and configuration reload clears
badges owned by the replaced VM.

### Pane readiness and grid changes

Capability: `Toyoterm.supports?(:pane_lifecycle_events)`.
`pane_ready` is queued once for each successfully started PTY-backed Pane runtime,
after terminal initialization and its first successful size synchronization.
Its dimensions are the layout-adjusted grid at that point; a failed spawn/resize
does not report readiness. Mux creation events are collected first, so a new
Pane's `pane_created` precedes its `pane_ready`. Initial readiness is queued
before `app_started`. Register handlers while loading configuration if they
must observe the initial Pane. Reload does not re-emit readiness for an already
running PTY. This event does not mean that a shell has displayed its first prompt;
use the shell-integration `prompt_started` marker for that.

`pane_resized` is queued after a subsequent successful PTY size synchronization
changes columns or rows. This includes native window/DPI/font, split, zoom and
layout changes. Repeated synchronization with unchanged row/column counts,
pixel-only changes, missing/exited PTYs, or failed resizes emit no grid event.
Failure preserves the last reported size, allowing a later successful retry to
report the change. This reports the PTY grid rather than full-window dimensions;
`width`, `height`, `window` and unrelated event fields are nil.

Events carry copied dimensions and a typed Pane handle. The context cache is
invalidated when the runtime/grid changes, so handlers receive a fresh native
model. `event.context` for all Pane events derives missing hierarchy members
from the reported Pane, rather than the currently focused tab/window. Focus
changes while a notification waits do not redirect its actions to another Pane.
The Pane may already have closed by delivery; guard `event.pane.valid?` before
dereferencing it. Notification dimensions remain the recorded values even if
the native grid has since changed.

Per-Pane resize notifications coalesce while Ruby is busy, preserving the latest
dimensions for each target independently. Readiness is not coalesced. Both follow
the existing runtime-event queue limits, overload/drop policy and per-handler
rollback; neither waits for Ruby before completing native geometry or PTY work.
There is no cancellation/veto result. Handler return values are ignored.

```ruby
[:pane_ready, :pane_resized].each do |name|
  Toyoterm.on(name) do |event|
    next unless event.pane.valid?
    event.pane.badge = "#{event.columns} cols x #{event.rows} rows"
  end
end
```

The [runnable example](../examples/pane_events.rb) also binds Alt+D to split right.

### Delivery and rollback

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
`pane_focused`, `pane_resized`, `workspace_changed`, and `window_resized` events for the same
native object are coalesced to their newest snapshot. If the event portion of
the queue is still full, newer events are dropped and a rate-limited warning is
logged. Key bindings, named commands, configuration reloads, and live-console
evaluations are lossless and are not subject to the event limit.

## Extension hooks and command advice

These APIs let ordinary required Ruby libraries publish extension points and
adapt commands without replacing their registered blocks. They run on the
same script thread as commands and events. Feature detection uses
`Toyoterm.supports?(:named_hooks)` and `Toyoterm.supports?(:command_advice)`.

### Named hooks

- `Toyoterm.add_hook(name, priority: 0, label: nil) { |*arguments| ... }` returns a removable
  `Toyoterm::Registration` of kind `:hook`.
- `Toyoterm.hooks` returns a frozen, sorted Array of hook names that have
  registered handlers.
- `Toyoterm.hook_handlers(name)` returns a frozen Array of frozen descriptions
  in execution order: `{id:, label:, priority:, source:, line:}`. It exposes no
  callable. Unknown names return an empty Array.
- `Toyoterm.run_hook(name, *arguments)` returns a frozen Array of handler
  results in execution order. Result objects themselves are not frozen.
  An unknown hook returns an empty Array.
- `Toyoterm.run_hook_until_success(name, *arguments)` returns the first truthy
  handler result without freezing/copying it, or nil if none succeeds. Later
  handlers do not run. Both nil and false mean no result; 0 and empty Strings/
  Arrays are truthy Ruby values.
- `Toyoterm.run_hook_until_failure(name, *arguments)` returns false on the first
  nil/false handler result and stops; returns true if all handlers are truthy,
  including an empty/unknown hook. It does not return individual handler values.

Names accept nonempty Strings or Symbols; wrong types raise `TypeError`, and
empty names or NUL bytes raise `ArgumentError`. Registration without a block
raises `ArgumentError`. Multiple handlers for the same name are supported.
Priorities must be Integers within -32768..32767; wrong types raise `TypeError`
and out-of-range values raise `ArgumentError` before registration. Lower values
run earlier. Equal priorities run in registration order, preserving existing
behavior with the default 0. A label accepts a nonempty String/Symbol or nil,
is copied/frozen, and is descriptive rather than unique identity: remove a
specific subscription through its registration handle. Duplicate labels are
allowed. Source/line follow command metadata rules (including virtual main
configuration labels and potentially nil runtime/debug locations).
Capabilities are `:hook_inspection`, `:hook_priority`, and `:hook_strategies`.
Arguments are ordinary Ruby objects passed by reference. Handler registration
or removal during dispatch takes effect on the next invocation; the current
invocation uses a snapshot of the handler list. Removing the last registration
removes its name from `hooks`.

A handler exception stops the remaining handlers, rolls back host-managed
changes from the entire hook call, and propagates to the caller. All three
execution strategies use the same checkpoint. A normal early stop commits
effects of handlers that ran, subject to the enclosing host request. This
differs from native runtime events, whose handlers are isolated individually.
Nesting beyond 64 hook invocations across all strategies raises `RuntimeError`; depth is restored
after exceptions. Mutations to arbitrary Ruby argument objects and external
side effects are not rolled back.

```ruby
# A library publishes a hook; another library can subscribe to it.
subscription = Toyoterm.add_hook(:project_opened) do |name, context|
  context.pane.badge = name
end

Toyoterm.command(:open_backend) do |context|
  Toyoterm.run_hook(:project_opened, "backend", context)
end

# Later: subscription.remove
```

Independent libraries can export provider, policy, and observation hooks:

```ruby
Toyoterm.add_hook(:terminal_directory, priority: 100, label: :fallback) { Dir.pwd }
Toyoterm.add_hook(:terminal_directory, priority: -10, label: :project) do |context|
  path = context.pane.cwd
  path if path && File.directory?(path)
end
# In a runtime command:
path = Toyoterm.run_hook_until_success(:terminal_directory, context)
allowed = Toyoterm.run_hook_until_failure(:allow_terminal_directory, path, context)
context.pane.split(:right, cwd: path) if path && allowed
```

These policies are trusted application conventions, not access control over
Ruby libraries. See [the runnable composition example](../examples/hook_composition.rb)
and [the library guide](extension-libraries.md) for explicit loading, inspection,
cleanup and compatibility conventions.

### Command advice

`Toyoterm.advise_command(name, kind: :before) { ... }` returns a
`Toyoterm::Registration` of kind `:advice`. `name` uses the same validation as
command names. `kind` must be the Symbol `:before`, `:after`, or `:around`;
other values and missing blocks raise `ArgumentError`. Advice may be registered
before its command exists. It targets the command name, survives command
replacement/removal within the VM, and applies if that name is registered again.
It covers registered Ruby commands, not direct calls to native actions.

| Kind | Block signature | Behavior |
| --- | --- | --- |
| `:before` | `{ |context| ... }` | Runs before the around chain; its return value is ignored |
| `:around` | `{ |proceed, context| ... }` | Wraps the next advice or command; its return value becomes that operation's result |
| `:after` | `{ |context, result| ... }` | Runs after successful completion of the around chain; its return value is ignored |

Before and after handlers run in registration order. Around handlers nest with
the first registered handler outermost. An around handler can skip
`proceed.call` to suppress the command and inner around handlers; the before
handlers have already run, and after handlers still receive its returned result.
After handlers do not run when a before/around handler or the command raises.

`proceed` takes no arguments and may be called at most once, synchronously
before its around handler returns. Repeated or retained calls raise
`RuntimeError`. Advice changes during execution take effect on the next command
invocation. All advice shares the command's context and rollback checkpoint;
an exception from any stage rolls back the entire command's host-managed work.

`Toyoterm.command_advice(name)` returns a frozen Array of frozen Hashes with
`:id` and `:kind`, in registration order. Unknown names return an empty Array.
The corresponding registration handle exposes `active?` and `remove`.
Hooks and advice participate in configuration and callback transactions:
failed transactions restore registrations, successful reloads replace them,
and failed reloads preserve the existing VM's registrations.

```ruby
Toyoterm.advise_command(:git_status, kind: :around) do |proceed, context|
  if context.pane.cwd
    proceed.call
  else
    Toyoterm.log(:warn, "No working directory was reported")
    nil
  end
end
```

## Window bars

`config.window.bar(position) { |bar| ... }` configures a window
bar. `position` accepts only `:top` or `:bottom`; any other value raises
`ArgumentError`. One bar may be registered at each position, and registering a
duplicate raises `ArgumentError`. Both bars use `config.ui.status_bar_height`.
The method returns the configured
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
  config.window.bar :bottom do |bar|
    bar.section(:left) do |section|
      section.add(interval: 1.0) { |context| context.workspace.name }
    end
    bar.section(:center) { |section| section.add("toyoterm") }
    bar.section(:right) do |section|
      section.add(interval: 5.0) { |context| context.pane.cwd }
    end
  end
end
```

`Toyoterm::BarSection#add(value = nil, interval: 1.0) { |context| ... }` appends
a fixed value or a synchronous block and returns the section. Supply either a
non-`nil` value or a block, not both. Each block keeps its last result and is
evaluated independently when its interval expires; other widgets continue to
display their cached values. The finite numeric interval must be at least 0.1
seconds. Fixed values do not schedule refreshes. `Toyoterm::BarContext` exposes
`workspace`, `window`, `tab`, and `pane`; each result is converted to a string.
`nil` and empty results are omitted without leaving a separator. Text containing
NUL raises `ArgumentError` when the bar is rendered.

A position is hidden when no bar is configured for it. Widget callbacks run
serially on the script thread. The host schedules each bar for its nearest
widget deadline, while widgets that are not due reuse their cached values.
Commands and pane badge changes queued by widget callbacks are always discarded.
If any widget raises, the bar keeps its previous rendered content and retries
after one second.

Widgets can launch background asynchronous tasks via `Toyoterm.async` without
blocking bar evaluation. Asynchronous tasks queued during bar rendering are
retained when the widget callback succeeds (unlike native mutations, which are
discarded). Keep the returned task in the bar closure and read its result when
it completes; completion wakes the bar scheduler, and the block observes the
result when its own `add` interval next expires:

```ruby
Toyoterm.configure do |config|
  config.window.bar :bottom do |bar|
    weather_task = nil
    bar.section(:right) do |section|
      section.add(interval: 1.0) do
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
    battery = File.read("/sys/class/power_supply/BAT0/capacity").strip
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


## Platform, clipboard, environment, and processes

Configuration and required Ruby libraries are trusted code. These APIs are intentionally not
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
`cancelled?`, `cancel`, `active?`, `remove`, `result`, `error`, `value!`, and `success?`.
`active?` aliases `pending?` and `remove` aliases `cancel`. `cancel`
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

## Timers and task ownership

`Toyoterm.after(seconds) { |timer, context| ... }` schedules a single callback;
`Toyoterm.every(seconds) { |timer, context| ... }` schedules repeated callbacks.
Both return a frozen `Toyoterm::Timer` and require a block (`ArgumentError`
without one). They are runtime operations: use a command, key, event, task
callback, or console request. Configuration and required-source evaluation
raise `RuntimeError`; use `Toyoterm.on(:app_started)` for startup scheduling.
Feature detection uses `Toyoterm.supports?(:timers)` and
`Toyoterm.supports?(:mode_tasks)`.

`seconds` must be a finite Numeric. `after` accepts 0 through 31,536,000 seconds
(365 days); `every` accepts 0.01 through that maximum. Wrong types raise
`TypeError`; invalid values raise `ArgumentError`. Delays round upward to whole
milliseconds. At most 1,024 timers may be active per VM; exceeding that bound
raises `ArgumentError`. Validation and failed transactions schedule no native
timer. IDs increase monotonically within a VM and are not reused after rollback.

| Timer member | Return value / behavior |
| --- | --- |
| `id` | VM-local Integer ID |
| `context` | Captured `CallbackContext` |
| `delay` | Normalized initial delay in seconds as a Float |
| `interval` | Normalized repeat interval in seconds, or `nil` for a single callback |
| `active?` | Whether the timer remains registered in this VM |
| `cancel` / `remove` | `true` if registration was removed, `false` if already inactive |
| `Toyoterm.timers` | Frozen Array of active Timer handles sorted by ID; handles are inspected/cancelled through these methods |

Deadlines use the native monotonic clock, starting when the successful request's
timer registry reaches the main thread. A zero delay means the next available
script dispatch, never synchronous invocation. An expired timer waits while the
script thread is occupied; its callback uses a fresh native snapshot. There is
at most one queued/running invocation per timer. Repeating timers wait their
full interval after the main thread receives completion, including after a
callback error. Missed ticks are coalesced instead of replayed. These are
cooperative timers, not real-time guarantees or interrupts for executing Ruby.
PTY parsing and rendering never invoke the timer block.

The callback receives its Timer and originating context; one-argument blocks
can ignore the context. A one-shot is consumed before calling Ruby, so it is
already inactive inside its block and stays consumed if the block raises.
A repeating timer remains active until cancelled. Exceptions are logged and
roll back host-managed changes, task creation/cancellation, configuration,
registrations, and queued work. The next repeat is still scheduled; cancelling
then raising rolls the cancellation back. Return values are ignored. Already
completed process results and one-shot delivery are not resurrected by rollback.
Arbitrary Ruby mutations and external file/process effects cannot be undone.

Standalone timers capture the current Pane. When a mode lifecycle or mode-owned
task schedules a timer/async task, its concrete Pane target is used in the new
task context. Timers whose captured Pane closes are pruned on the next script
request without invoking their block. Mode-owned tasks and their descendants
are cancelled when the mode disables, is replaced, or loses its target. Owned
callbacks may disable their own mode. Inactive task handles are pruned from the
mode's resource list as new resources are registered, preventing accumulation
of completed polling tasks. Other event/command callbacks are independent unless
called within that owned task's execution.

Successful configuration reload clears timer schedules and queued old-VM task
callbacks. Async worker results carry a VM generation, so a result from an old
configuration cannot invoke a new callback with a reused task ID. Failed reload
preserves the old timers/tasks. Cancellation of an async task suppresses its
callback/result and is transactional; it does not terminate a child already
running in a worker. Completed tasks retain the result through their handle
without retaining their runtime registry entry.

```ruby
Toyoterm.on(:app_started) do
  Toyoterm.after(5) { |_timer, context| context.pane.badge = 'Ready' }
end

Toyoterm.command(:start_counter) do
  count = 0
  Toyoterm.every(1) do |timer, context|
    count += 1
    context.pane.badge = count.to_s
    timer.cancel if count == 10
  end
end
```

The [timer example](../examples/timers.rb) adds a Pane-local clock mode, toggled
with Alt+T and disabled with Escape, plus a one-shot reminder on Alt+R. Its timer
is automatically released when clock mode disables.

## Resource scopes

Capability: `Toyoterm.supports?(:resource_scopes)`.
`Toyoterm.resource_scope(name: nil, pane: nil) { |scope| ... }` creates and returns
a frozen `Toyoterm::ResourceScope`. It groups removable resources without a
mode/keymap definition. The optional block runs immediately inside the scope;
its return value is ignored by the factory. Without a block, use `within` later.
Global scopes may be created while loading configuration/libraries; their blocks
still obey individual API restrictions on runtime-only tasks/overlays.

`name:` accepts a nonempty String/Symbol or nil and is copied/frozen. It is a
descriptive label, not unique identity; duplicate names are allowed. `pane:`
accepts nil (global) or a live Pane. Wrong types raise `TypeError`, invalid names
raise `ArgumentError`, stale Panes raise `InvalidHandleError`, and the startup
placeholder Pane raises `RuntimeError`. At most 256 scopes may be active in a
VM; exceeding the bound raises `ArgumentError` without partial installation.
IDs are VM-local monotonic Integers and are not reused after rollback.

| Scope member | Behavior |
| --- | --- |
| `id`, `name`, `pane` | Identity, optional frozen name and Pane/nil lifetime target |
| `active?` | Whether this exact scope remains registered |
| `resources` | Frozen Array of currently active owned handles, in creation order; expired handles are pruned. Closed scopes return an empty Array |
| `within { |scope| ... }` | Runs with this ownership and returns the block's value; missing blocks raise `ArgumentError`, closed scopes raise `RuntimeError`; a live target is validated |
| `wrap { |*arguments, **keywords| ... }` | Returns a Proc that re-enters `within` on invocation, preserving positional/keyword arguments and return value. Missing blocks raise `ArgumentError`; after closure it returns nil without running the block |
| `close` / `remove` | Removes all owned resources and descendants in reverse creation order; true once, false when already closed |
| `Toyoterm.resource_scopes` | Frozen Array of active scope handles sorted by ID; prunes closed-Pane scopes |

Closure marks the scope inactive before removing its resources; wrappers tied
to it no longer run, including during mode teardown.

Tracked resources include commands, events, hooks/advice, explicit and transient
keymap activations, timers, async tasks, cooperative work, input/selection/panel
lifetimes, and child resource scopes. Mode activations newly enabled within a
scope are also tracked as `Registration` handles of kind `:mode_activation`,
with mode name and activation ID. Their `active?`/`remove` apply only to that
exact instance. Already-enabled modes are not adopted; disabling/re-enabling a
mode later does not let an old scope's handle disable the new instance.
Mode enable/disable return values remain Boolean.
Explicit scope closure uses the mode's ordinary disable lifecycle. Automatic
Pane/ancestor pruning removes owned mode instances without their lifecycle
callbacks, including modes targeting another still-live Pane, so teardown does
not run for an expired owner.

Definitions of keymaps/modes, edits to existing keymaps/settings, arbitrary
terminal mutations and external effects are not owned resources. Removing a
replacement command or a newly selected major mode does not restore an earlier
command/major mode. Scope closure does not unload `require`d files or execute
arbitrary finalizers. Async cancellation suppresses delivery and does not kill
an already-running process.

The innermost executing mode lifecycle or explicit scope owns new registrations.
A mode enabled in a scope owns its own lifecycle-created resources by default; the scope
owns that mode activation. A scope created in a mode is a child resource of the
mode. Closing a parent closes child scopes even if a child is global. Pane-local
scopes close on the next request/registry refresh after their Pane disappears;
they do not run user teardown callbacks. Their task contexts use the declared
Pane. Global scopes use the current Pane when a new task/overlay is created.

Owned timer/async/cooperative/transient/prompt callbacks propagate their scope
automatically. Ordinary command/event/hook callbacks are not implicitly wrapped
just because their registration belongs to a scope: pass `scope.wrap` when their
later-created resources must belong to it. Typed command prompts already use
the defining owner. A synchronously executing callback can finish after closing
its scope, but registering new resources under that expired owner removes the
new resource and raises `RuntimeError`. A saved wrapped callback becomes a no-op.
Already-accepted ordinary keymap callbacks and hook/event dispatch snapshots
retain their normal delivery rules; wrapping makes their body lifetime explicit.

Factory blocks, `within`, and `close` each checkpoint host-managed state, even
if an outer Ruby caller rescues an error. A failed install/close restores the
previous resources and pending tasks/overlays. They also participate in the
enclosing host transaction; failure later in that request can restore a closed
scope. Ruby objects, module variables and external effects are not rewound.
Inspect the retained handle or `resource_scopes` after errors instead of assuming
a caller's Ruby variable was restored. Successful reload replaces all scopes;
failed reload preserves the old ones. These are trusted lifecycle groups, not
a permission boundary or implicit extension loader.

```ruby
scope = Toyoterm.resource_scope(name: 'example.status')
scope.within do
  Toyoterm.command(:status, &scope.wrap do |context|
    panel = Toyoterm.show_panel(lines: ["Pane: #{context.pane.id}"])
    Toyoterm.after(1) { panel.update(lines: ['Updated']) }
  end)
end
# Later: scope.close removes the command and pending tasks/overlays.
```

The [runnable library example](../examples/resource_scopes.rb) uses Alt+S for
a scope-owned live panel. Close the panel, then Alt+M toggles the entire library.
Its namespaced scope lookup is an application convention; scope names remain
descriptive and are not enforced unique by the host.

## Instruction budgets

Capability: `Toyoterm.supports?(:instruction_budgets)`.
`Toyoterm.with_instruction_budget(instructions:) { ... }` returns the block's
value on success. `instructions:` must be an Integer within 1..1_000_000_000;
other types raise `TypeError`, and out-of-range values or a missing block raise
`ArgumentError`. At most 64 nested guards are supported; another raises
`RuntimeError`. Available during configuration loading and runtime callbacks.
Ordinary callbacks remain unlimited unless explicitly wrapped.

The guard counts mruby opcode fetches in the block, including Ruby callees and
Ruby executed by resumed Fibers. Nested guards count toward every active parent;
the smallest remaining allowance can expire first. Compilation and C/native
functions, synchronous I/O and external process waiting are not opcode-counted.
The exact count depends on the embedded compiler/runtime; it is neither a
wall-clock duration nor a stable source-line count. Unused allowance is discarded.

Exhaustion raises `Toyoterm::InstructionBudgetError < RuntimeError` with message
`instruction budget exhausted`. An exhausted guard stops checking so Ruby
`rescue`/`ensure` cleanup can execute. At guard exit, exhaustion takes precedence
over a rescued result, nonlocal `return`/`break`, or another cleanup exception.
Normal return/break semantics are preserved when the budget has not expired.
Yielding a Fiber across the guard's C frame raises mruby's `FiberError`; resuming
a separate Fiber inside a guard is supported. The active counter is restored
before any exception or nonlocal exit leaves the C boundary.

Any exception rolls back the block's host-managed registrations, configuration,
commands, badges, tasks, scopes and overlays to its entry checkpoint, including
host effects emitted by `ensure`. This rollback occurs even when the caller
rescues the exception. Success commits to the enclosing callback transaction;
normal nonlocal exits retain ordinary callback semantics. Arbitrary Ruby objects,
iterator progress, filesystem writes and external process effects are not
rewound. The exhausted guard remains subject to the enclosing guard while
cleanup runs, and an enclosing failure restores its own entry checkpoint.

This is a trusted-code diagnostic guard, not a security boundary or guaranteed
preemption: code that rescues exhaustion and never returns can still delay the
script owner; C/native work can also delay it. Use small units with `work_each`
for scheduling between items and `async` for blocking external operations.
Neither counting nor unwinding runs on the main PTY/rendering thread.

```ruby
begin
  value = Toyoterm.with_instruction_budget(instructions: 100_000) do
    expensive_ruby_analysis(Toyoterm.current_pane.screen_text)
  end
rescue Toyoterm::InstructionBudgetError
  Toyoterm.log(:warn, 'Terminal analysis exceeded its instruction allowance')
end
```

The [bounded terminal-summary example](../examples/instruction_budgets.rb) uses
Alt+I to show the most frequent words or a budget-exhaustion message in a read-only
panel. See [the measured hook overhead](performance-baseline.md#instruction-budget-hook).

## Cooperative work

Capabilities: `Toyoterm.supports?(:cooperative_work)` and `:monotonic_clock`.
`Toyoterm.work_each(items, name: nil, batch_size: 32, budget_ms: 4, interval: 0.01)
{ |item, context, task| ... }` returns a `Toyoterm::CooperativeTask`. It captures
an Enumerator from `items.to_enum(:each)` without synchronously processing it.
The first slice is a zero-delay single-shot timer; each successful continuation
uses another single-shot timer after `interval`. Existing queued script requests
can run between slices. The original Pane/context is retained across focus
changes, with fresh native snapshot data for each delivery.

| Option | Accepted value |
| --- | --- |
| `items` | Object responding to `each`; each `Enumerator#next` value is passed as one item |
| `name:` | Optional nonempty String/Symbol; copied/frozen descriptive name, not unique identity |
| `batch_size:` | Integer within 1..4096; maximum successfully processed items per slice |
| `budget_ms:` | Finite Numeric within 0.1..100; elapsed-time target checked between items |
| `interval:` | Finite Numeric within 0.01..60 seconds between continuations; native delays round upward to milliseconds |

Missing blocks raise `ArgumentError`. Invalid object/option types raise
`TypeError`; invalid values/bounds raise `ArgumentError`. Creation is runtime-only
and source-loading calls raise `RuntimeError`. Up to 256 cooperative tasks may
be active; each also uses one of the VM's 1024 timer registrations. Internal
drive timers appear in `Toyoterm.timers`; cancel the work handle to control the
whole task. Cancelling its drive timer also prevents further progress.

The block receives exactly item, captured `CallbackContext`, and its task handle.
Blocks may ignore arguments; strict lambdas must accept all three. Each slice
starts at most `batch_size` items and checks its time target after at least one
item. Reaching either bound queues the next slice. Exhaustion marks completion;
an additional empty slice may be needed when exhaustion is discovered at the
start of the next delivery. Return values of item blocks are ignored.

This is cooperative scheduling, not opcode preemption or a time guarantee.
`Enumerator#next`, an item callback or synchronous I/O can exceed the whole
budget or stall the script thread. The next check occurs only after it returns.
Choose small units of work, keep expensive setup outside the GUI hot path,
and use asynchronous processes for blocking external operations. PTY parsing
and rendering remain native throughout.

| Task member | Behavior |
| --- | --- |
| `id`, `name`, `context` | VM-local work ID, descriptive name and original callback context |
| `batch_size`, `budget_ms`, `interval` | Configured slice limits/delay |
| `state` | `:pending`, `:running`, `:completed`, `:failed` or `:cancelled` |
| `active?` | True while pending/running with a registered drive timer |
| `cancel` / `remove` | Removes pending continuation; true once, false if inactive. From an item callback it stops after that item returns |
| `error` | Frozen host error String for failure, otherwise nil |
| `statistics` | Frozen Hash with `id`, `name`, `state`, `processed`, `slices`, `total_ms`, `last_ms`, `overruns`, `batch_size`, `budget_ms` (Symbol keys) |
| `Toyoterm.work_tasks` | Frozen Array of active handles, sorted by work ID |

`processed` counts item callbacks that returned normally; `slices` includes empty
and failed deliveries. Timing covers iterator/callback execution, excluding
the wait between slices. An overrun is a slice whose elapsed time exceeds its
configured budget. Progress/timing and the iterator cursor are Ruby execution
state and are not rewound by host rollback. Therefore a failed slice can report
successful earlier callbacks even though that slice's host effects were undone.

Each slice uses the normal host transaction. Exceptions from enumeration/items,
or host configuration validation, roll back that slice's host-managed effects
and terminally fail the task after rollback. Failed deliveries never retry
already-consumed input; previous committed slices remain committed. Errors also
follow ordinary native timer reporting. Cancellation of pending work and creation
are transactional; failed cancellation restores the prior pending schedule.
Arbitrary Ruby mutations, iterator side effects and external I/O cannot be undone.

Mode-owned work and its callback-created registrations/tasks inherit the mode's
resource scope. Disable/replacement/Pane closure cancels its continuation and
cleans up descendants. Other work remains independent. Closed originating Panes
are pruned without running more items. Successful reload drops old schedules;
failed reload preserves pending work. Terminal tasks release their iterator/
callback references while retained handles keep diagnostics. This does not
promise to execute an iterator's cleanup/ensure on cancellation; callers own
external resources and should close them explicitly.

`Toyoterm.monotonic_time` returns finite nondecreasing Float seconds from a
process-local unspecified origin, using Rust's `Instant`. It is available during
configuration and runtime, is unrelated to calendar time, and is not reset by
configuration reload. Platform suspend behavior follows the native clock;
do not persist or compare values across processes. The work scheduler uses it
for elapsed-time targets. `Time.now` remains available for calendar displays.

```ruby
Toyoterm.command(:process_rows) do |context|
  task = Toyoterm.work_each([1, 2, 3], name: :rows, batch_size: 1) do |item, step, work|
    step.pane.badge = item.to_s
  end
  # Retain task in the calling library to inspect statistics or cancel later.
end
```

The [word-count example](../examples/cooperative_work.rb) processes terminal
snapshot lines in batches and displays progress/diagnostics in a read-only panel.
Closing it cancels work and its mode-owned timers on the next available slice
or monitor tick. No editor state or file editing is involved.

## Loading Ruby libraries

Use `require(feature) -> true | false` or
`require_relative(feature) -> true | false` to load a trusted local Ruby source
explicitly. `feature` must be a non-empty String; paths without an extension
gain `.rb`. Relative features resolve beside the declaring config or library.
`require` searches `$LOAD_PATH`, which contains the config directory, its
`lib/` directory, and the corresponding default user-library locations. `~/`
expands to the user home directory.

Sources are evaluated immediately in Ruby's top-level object context. The first
load returns `true`; a canonically duplicate path executes no code and returns
`false`. toyoterm does not scan directories or provide another implicit loading
mechanism. Unreadable sources and Ruby exceptions reject the fresh config VM and
preserve the active VM.

A required file calls the same public APIs as `config.rb` directly:

```ruby
# lib/git.rb
prefix = "git"

Toyoterm.command :git_root do |context|
  context.pane.send_text("#{prefix} rev-parse --show-toplevel\n")
end

Toyoterm.on(:bell) { |event| event.pane.badge = "bell" }

Toyoterm.configure do |config|
  config.keys.ctrl_shift("g").command(:git_root)
end
```

Local variables and closures are the preferred helpers when implementation
details should stay inside one file. For several private helpers, keep an
anonymous `Module` in a local variable. To expose an intentional API to
`config.rb`, define a named Ruby module. Avoid top-level `def` for private
helpers because it adds a private method to the global `Object` namespace.
toyoterm adds no export or private-helper mechanism.

Required sources share the configuration VM and its filesystem, process,
environment, and clipboard authority. Loading a library is equivalent to
allowing that source to execute as the toyoterm process. Successful reloads
replace the whole VM, so commands, event handlers, themes, and callback Procs
belong to the new configuration generation and do not accumulate. Failed
reloads discard the fresh VM and preserve the active configuration generation.

## Themes

Register a named color theme with `Toyoterm.theme(name) { |colors| ... }`:

```ruby
# themes/moon.rb
Toyoterm.theme "moon" do |colors|
  colors.background = "#10131a"
  colors.foreground = "#d8dee9"
  colors.cursor = "#88c0d0"
end
```

Then require and select it from the configuration:

```ruby
require_relative "themes/moon"

Toyoterm.configure do |config|
  config.theme = "moon"
  config.colors.cursor = "#ffffff"
end
```

`name` accepts a non-empty String or Symbol. The method requires a block and
returns the registered color configuration. A theme starts with default colors
and accepts every `config.colors` field, including the ANSI array.
`Toyoterm.themes` returns registered names in registration order.

Registered names also form the native color-preset registry used by iTerm2 OSC
1337 `SetColors=preset=name` and `SetProfile=name`. An exact name match applies that theme's terminal
foreground, background, cursor, selection, and 16 ANSI colors to the requesting
pane without changing `config.theme` or other panes. Unknown names are ignored.
At most 4,096 themes are mirrored to the native runtime, and names are limited to
128 UTF-8 bytes without control characters.

Theme lookup is validated after the complete configuration source and all of
its required sources have been evaluated. Duplicate theme names and unknown
selected themes raise `ArgumentError` and reject the configuration. Applying a
theme happens before explicit `config.colors` assignments, so explicit scalar
colors and individual ANSI entries override theme values. Theme registrations
participate in the same configuration and callback transactions as commands,
events, and keys: an exception rolls back partial registration, and a failed
reload leaves the previous generation's theme registry intact.

Native validation transfers the complete theme registry in one snapshot, including
changes to retained color configurations. Temporary compilation objects are
collected on the script thread after successful config loads and completed
requests; registered themes and callbacks remain live. This honors `GC.disable`
and does not run Ruby or garbage collection on the rendering thread.

## Live Ruby console

Connect to the running GUI's persistent VM with:

```sh
toyoterm ruby console
```

The console uses mruby's parser to recognize multiline input, including blocks,
methods, strings, and heredocs. Top-level local variables persist between
entries. It also supports `:history` and `exit`.
Empty lines leave the console active. On Windows, the console temporarily
enables processed, echoed line input and disables virtual-terminal input,
restoring the inherited input mode when it returns (including on I/O errors).
Redirected stdin is read as a byte stream without changing console modes.
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
