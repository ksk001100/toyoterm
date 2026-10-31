# Steel configuration DSL and runtime API

toyoterm embeds Steel 0.8.3, a Scheme implementation in Rust. Ruby source is
not supported. The terminal hot path remains native. Names beginning with `__`
are private implementation details. Public procedures below are available in
configuration, imported libraries, callbacks and the live console.

## Loading configuration

Selection order is `--config PATH`, nonempty `TOYOTERM_CONFIG_FILE`, then:

- Unix/macOS: `$XDG_CONFIG_HOME/toyoterm/config.scm`, falling back to
  `~/.config/toyoterm/config.scm`.
- Windows: `%APPDATA%\toyoterm\config.scm`, falling back to
  `%USERPROFILE%\.config\toyoterm\config.scm` when the first file is absent.

An explicitly selected file must exist. The optional default may be absent.
The GUI recovers from a broken file with defaults and an error banner, retaining
its path for another reload. A reload evaluates and validates a fresh VM before
replacing the active VM; failure preserves the complete previous generation.
Use `toyoterm reload` or `(reload-config!)`.

Configuration and libraries are trusted code with the permissions of the
process, including filesystem and process access. This is not a sandbox.
Steel's ordinary Scheme functions, closures, macros, collections, ports,
filesystem functions and module system are available. `(require "lib.scm")`
loads modules relative to the declaring source; `(provide name ...)` exports
intentional library APIs. Config and user `lib/` directories are added to module
search directories. Required sources are not scanned automatically. A failed
import rejects the candidate configuration together with its registrations.

Steel collects automatically as allocations require it, on the script owner
thread. The host does not force full collection after callbacks or reloads and
does not guarantee immediate memory reclamation when a callback returns.
Transaction checkpoints and retained callbacks remain rooted during collection.
Native config/registration snapshots are revalidated when their settings,
themes or registrations change. Ordinary widget-cache and process-queue
updates reuse the previous validated snapshot; failed validation still rolls
back the complete application-visible transaction.

```scheme
(config-set! 'font.family "monospace")
(config-set! 'font.size 14)
(bind-action! "CTRL+SHIFT+T" 'new-tab)
(bind! "CTRL+SHIFT+H"
  (lambda (ctx) (pane-send-text! (context-ref ctx 'pane) "echo hello\n")))
```

## Settings

`(config) -> immutable-hash` returns an effective settings snapshot.
`(config-ref key) -> value` reads a setting.
`(config-set! key value) -> value` validates and assigns a setting.
`(configure thunk) -> immutable-hash` invokes a zero-argument procedure and
returns the effective snapshot. These procedures have the same transaction
semantics whether called at startup, from a callback, or from the console.

Keys are symbols or strings containing dotted paths, for example `font.size`,
`window.image.path`, `ui.active-pane-border-width`. Hyphens in setting paths
are normalized to underscores. Unknown settings and wrong types raise a Steel
error. Native cross-field validation occurs before committing a request.
Settings retain the native defaults listed below; `#f` represents absence.

Colors are `#RRGGBB` strings; `colors.ansi` is an immutable list of exactly 16
colors. `(ansi-color-ref index)` reads an entry; `(ansi-color-set! index color)`
sets an entry (index 0..15), preserving individual overrides across theme switches.
A whole-list `config-set!` explicitly overrides the full palette. Font fallback
is a list of at most 32 unique nonempty family names, excluding the primary
family. Font weight is an integer in 1..1000. Font sizes and dimensions must be
positive finite numbers; padding and border widths may be zero. Booleans require
`#t` or `#f`. Scrollback is a nonnegative integer.

Window opacity clamps finite values to 0..1. Wallpaper opacity requires 0..1.
`window.image.path` is a nonempty PNG/JPEG path or `#f`; relative paths resolve
beside the config, and `~/` expands to home. Images retain the 8192-pixel-axis,
256 MiB decoder-allocation limits and immutable native image storage. Reload
rereads images; unchanged live paths reuse decoded pixels. Bad image data rejects
the transaction. Window dimensions apply at creation, and default shell changes
only affect new sessions.

`(osc-ref feature) -> boolean | string | #f` and
`(osc-set! feature value) -> value` group terminal permissions:
`clipboard`, `notifications`, `attention`, `open-url`, `focus` take booleans;
`downloads`, `uploads`, `background-image` take an absolute fixed-root directory
or `#f` to disable. Directory setters update both the path and permission.
The corresponding `behavior.*` fields remain independently accessible.

## Keys, actions and registration handles

- `(leader! chord timeout-ms) -> leader-data` sets a positive integer timeout.
- `(bind! chord callback) -> registration` registers `(lambda (context) ...)`.
- `(bind-action! chord action [argument]) -> registration` compiles a native action.
- `(bind-command! chord name) -> registration` compiles a named-command binding.
- `(unbind! chord) -> void` removes dynamic and native bindings for that chord.
- `(command! name callback [replace?]) -> registration` registers a named command;
  duplicates raise unless the optional boolean is `#t`.
- `(on! event callback) -> registration` appends an event handler in registration order.
- `(registration-active? registration) -> boolean` reports generation validity.
- `(registration-remove! registration) -> boolean` removes that exact registration;
  an old handle cannot remove a newer replacement.

Chords are nonempty strings, normalized to uppercase, such as `CTRL+SHIFT+T`,
`LEADER+V`, `PHYSICAL:KEYQ` and `CTRL+PHYSICAL:KEYQ`. A logical symbol uses its
produced character, so `$` does not require a literal SHIFT prefix.
`(primary-modifier) -> string` returns `SUPER` on macOS and `CTRL` elsewhere.
Duplicate bindings and invalid actions raise before registration.

`(action! action [argument]) -> void` queues an action against the current
context. `(context-action! context action [argument]) -> void` binds it to the
explicit originating workspace/window/tab/pane IDs. The main thread validates
and activates this hierarchy before application; stale contexts fail atomically.
Names accept symbols or strings; hyphens and underscores are accepted for native
action names. `split` and `activate-pane` require `left`, `right`, `up`, `down`.
`move-visual-selection` requires `left`, `right`, `up`, `down`, `line-start`,
`line-end`, `word-forward`, or `word-backward`. Other actions take no argument:

`new-tab`, `close-pane`, `close-tab`, `new-workspace`, `reload-config`, `search`,
`maximize-window`, `toggle-maximize`, `minimize-window`, `toggle-fullscreen`,
`toggle-zoom`, `next-tab`, `previous-tab`, `next-workspace`, `previous-workspace`,
`next-prompt`, `previous-prompt`, `next-mark`, `previous-mark`,
`select-next-command-output`, `select-previous-command-output`,
`select-last-command-output`, `copy-selection`, `paste-clipboard`,
`start-visual-mode`, `toggle-visual-mode`, `start-visual-selection`,
`select-visual-selection`, `end-visual-selection`, `yank-selection`.
Named commands use `bind-command!`, not dynamic native action dispatch.

## Contexts, handles and events

`(context) -> immutable-hash` and `(context-ref context field) -> value` expose
`workspace`, `window`, `tab`, `pane`. `(current-workspace)`, `(current-window)`,
`(current-tab)`, `(current-pane)` return typed ID handles. `(handle kind id)`
constructs a nonnegative ID value for `workspace`, `window`, `tab`, `pane`.
`handle-id`, `handle-kind`, `handle-valid?` inspect it. Handles compare structurally
by type and ID. `handle-ref` validates liveness; stale operations raise
`InvalidHandleError` before queuing anything. Native state is never mutated directly.

`(handle-ref handle field)` exposes:

| Handle | Fields |
| --- | --- |
| workspace | `id`, `name`, `windows` (window handles) |
| window | `id`, `tabs` (tab handles) |
| tab | `id`, `title`, `panes` (pane handles), `zoomed?` |
| pane | `id`, `title`, `icon-title`, `cwd`, `remote-host`, `shell-integration-version`, `shell-integration-shell`, `user-vars` (string hash), `pid`, `command-running?`, `last-exit-status`, `screen-text`, `zoomed?`, `badge` |

Missing optional metadata is `#f`. `(workspaces)` and `(windows)` enumerate live
handles. `(find-workspace name)` returns a matching handle or `#f`.
The following queue commands and return void; resulting objects appear in later snapshots:

- `(activate! handle)`, `(close! handle)` (close accepts pane/tab/window).
- `(open-workspace! name)` activates or creates a named workspace.
- `(new-window! workspace [options])`, `(new-tab! window [options])`.
- `(split! pane direction [options])`.
- `(pane-send-text! pane text)`, `(pane-search! pane query [direction])` where
  direction is `next` (default) or `previous`.
- `(pane-badge-set! pane string-or-#f)` sets or clears a badge.
- `(clipboard-read) -> string` raises if unavailable;
  `(clipboard-write! string) -> void` queues a clipboard write.

Launch options are an immutable hash with optional symbol keys `program` (string),
`args` (list of strings), `cwd` (string or `#f`), `env` (string-keyed hash whose
values are strings or `#f` to remove a variable). `(command-options command)`
constructs program/args options from a program string or nonempty argv list.
Arguments are passed directly, without shell interpolation. Omitted program uses
the configured/default shell. Wrong handle kinds and invalid options raise.

Events are immutable hashes read with `context-ref`. They expose `name`, optional
`workspace`, `window`, `tab`, `pane` handles, `title`, `cwd`, `exit-status`,
`width`, `height`, `columns`, `rows`; absent payloads remain `#f`.
`(event-context event)` resolves missing hierarchy fields from the request context;
`(event-subject event)` returns the most specific present handle or `#f`.
Event names accept hyphens/underscores:
`app-started`, `config-reloaded`, `workspace-changed`, `window-created`,
`window-closed`, `tab-created`, `tab-closed`, `pane-created`, `pane-closed`,
`pane-focused`, `title-changed`, `cwd-changed`, `prompt-started`,
`command-line-started`, `command-started`, `command-finished`, `bell`, `window-resized`.
Unknown names raise. A failing event handler rolls back its own application
changes, logs the error, and lets later handlers run.

## Selection, themes and bars

`(select! title items callback) -> void` opens a searchable overlay. The callback
is `(lambda (selection origin-context) ...)`; cancellation passes `#f`.
Exactly one selection may be pending, and it cannot start during source loading.
Title is at most 256 UTF-8 bytes; items is a nonempty list of at most 4096 strings,
each at most 4096 bytes, total at most 4 MiB. NULs and line breaks are rejected.
The callback executes once on the script thread and can queue another selection.

`(theme! name colors-hash [replace?]) -> immutable-hash` registers a palette initialized
from defaults. Keys are `colors.*` settings. Duplicates raise unless the optional replacement flag is `#t`; replacement
preserves registration order. Invalid palettes always raise. `(themes) -> list-of-strings` preserves registration order.
`(config-set! 'theme name-or-#f)` selects a theme; unknown names reject commit.
Explicit scalar and ANSI palette settings override theme defaults. Registered
palettes are also mirrored to native OSC 1337 named presets. Names are limited to
128 UTF-8 bytes without control characters; at most 4096 themes are mirrored.

`(bar! position alignment separator widgets) -> registration` adds a section.
Position is `top` or `bottom`; alignment is `left`, `center`, `right`.
`widgets` is a list of `(widget string-or-context-procedure interval-seconds)`.
Intervals are finite and at least 0.1 seconds. Each widget caches independently;
values must be strings. Sections join values using the string separator.
Bar callbacks discard queued terminal commands and still support async work.
`(async-widget program args interval initial formatter [cwd-or-procedure]) -> widget` runs
periodic processes and formats `(lambda (process-result) string)` without blocking
the script thread. The optional cwd accepts a directory string, `#f`, or a
procedure `(lambda (context) directory-or-#f)` evaluated for each process launch.
Empty widget strings are omitted when joining section values. See [async_bar.scm](../examples/async_bar.scm).

## Host APIs and process tasks

`(toyoterm-version)` and `(toyoterm-api-version)` return version strings.
`(toyoterm-platform)` returns `linux`, `macos`, `windows`, or `other`.
`(toyoterm-env)` returns an immutable string-keyed environment snapshot.
`(toyoterm-supports? capability)` detects `async-process`, `bar-sections`,
`callback-context`, `registration-handles`, `select-overlay`, `targeted-actions`,
`typed-events`. `(log! level string)` accepts `trace`, `debug`, `info`, `warn`, `error`.

`(spawn program args [cwd]) -> process-result` captures a synchronous command.
`(async! program args callback-or-#f [cwd]) -> task` queues asynchronous execution.
Callbacks are `(lambda (process-result origin-context) ...)`. Results expose
`process-stdout`, `process-stderr` (byte-exact integer lists),
`process-stdout-text`, `process-stderr-text` (strict UTF-8; decoding errors raise),
`process-exit-status`, `process-launch-error?`, `process-success?`.
A launch failure is a result with launch-error true and status -1; a child without
a portable exit code also has status -1, with launch-error false.

Task procedures are `task-id`, `task-pending?`, `task-complete?`, `task-result`
(`#f` while unavailable), `task-success?`, `task-value!` (raises unless successful),
`task-cancel!`, `task-cancelled?`. Completed results live in task handles, not a
global completed-task registry. Cancellation suppresses callbacks/results; an
already launched OS process may continue. It is not a process-kill guarantee.
Filesystem work and `spawn` run synchronously on the script thread. Use `async!`
for slow or external operations.

## Console and transactions

`toyoterm steel console` connects to the GUI's persistent VM. Definitions persist
between entries; the Steel parser recognizes multiline forms, strings and comments.
`:history`, `exit` and `:q` are available. A successful evaluation prints the last
value's inspected Scheme representation and atomically mirrors setting/registry changes.
Mutable VM objects and procedure captures remain opaque when inspected;
container nesting is displayed to 128 levels. A failed request restores settings, themes, registrations, queues and application
metadata. User lexical/global variable assignments, file writes and already
executed processes are not rolled back. Reload replaces the entire VM generation.

The single VM is constructed, invoked and dropped on the named script thread.
Immutable snapshots and typed commands cross the main-thread boundary. Slow
Scheme delays later script requests, while PTY parsing and rendering continue.
There is no forced timeout; callbacks of at least 100 ms are logged as slow.

## Default setting values

| Setting path | Default |
| --- | --- |
| `font.family` | `"monospace"` |
| `font.fallback` | `'()` |
| `font.size` | `14.0` |
| `font.weight` | `400` |
| `colors.background` | `"#090b0e"` |
| `colors.foreground` | `"#dce1e8"` |
| `colors.cursor` | `"#f5f7fa"` |
| `colors.selection` | `"#375891"` |
| `colors.tab_bar` | `"#11151b"` |
| `colors.tab_active` | `"#18243a"` |
| `colors.tab_inactive` | `"#15191f"` |
| `colors.workspace_bar` | `"#0d1014"` |
| `colors.status_bar` | `"#101419"` |
| `colors.pane_border` | `"#375891"` |
| `colors.zoomed_pane_border` | `"#ffbe3a"` |
| `colors.search_match` | `"#c4972f"` |
| `colors.search_match_active` | `"#ffbe3a"` |
| `window.opacity` | `1.0` |
| `window.width` | `960` |
| `window.height` | `600` |
| `window.min_width` | `320` |
| `window.min_height` | `180` |
| `window.decorations` | `#t` |
| `window.resizable` | `#t` |
| `window.always_on_top` | `#f` |
| `window.title` | `"toyoterm"` |
| `ui.padding_x` | `8` |
| `ui.padding_y` | `8` |
| `ui.line_height` | `1.2857143` |
| `ui.tab_bar` | `#t` |
| `ui.tab_bar_height` | `30` |
| `ui.tab_width` | `160` |
| `ui.workspace_bar` | `#t` |
| `ui.workspace_bar_height` | `24` |
| `ui.workspace_width` | `160` |
| `ui.status_bar_height` | `24` |
| `ui.pane_divider_width` | `2` |
| `ui.active_pane_border_width` | `2` |
| `behavior.scroll_lines` | `3` |
| `behavior.copy_on_select` | `#f` |
| `behavior.allow_osc52_copy` | `#f` |
| `behavior.allow_osc_notifications` | `#f` |
| `behavior.allow_osc_attention_requests` | `#f` |
| `behavior.allow_osc_open_url` | `#f` |
| `behavior.allow_osc_file_downloads` | `#f` |
| `behavior.osc_download_directory` | `""` |
| `behavior.allow_osc_file_uploads` | `#f` |
| `behavior.osc_upload_directory` | `""` |
| `behavior.allow_osc_background_image` | `#f` |
| `behavior.osc_background_image_directory` | `""` |
| `behavior.allow_osc_focus_requests` | `#f` |
| `window.image.path` | `#f` |
| `window.image.opacity` | `1.0` |
| `colors.ansi` | `standard 16-color ANSI palette` |
| `default_shell` | `#f` |
| `scrollback_lines` | `10000` |
| `theme` | `#f` |
