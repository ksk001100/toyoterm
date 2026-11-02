# Crate architecture

The cross-cutting choices behind this structure are recorded in the
[architecture decision record index](adr/README.md).

toyoterm is a Cargo workspace. Each crate owns one runtime responsibility:

- `toyoterm-api`: stable IDs, native commands, events, and handles
- `toyoterm-mux`: workspaces, windows, tabs, panes, and split trees
- `toyoterm-terminal`: VT state, snapshots, selection, and input encoding
- Terminal image decoding also belongs to `toyoterm-terminal`: bounded Sixel,
  Kitty, and OSC 1337 payloads become immutable RGBA snapshots using `image`,
  `base64`, and `flate2`. `toyoterm-render` uploads and caches their GPU textures.
  These external dependencies add no internal crate edges.
- `toyoterm-pty`: process spawning, PTY I/O, resize, and child lifecycle
- `toyoterm-render`: layout plus GPU and text rendering
- `toyoterm-config`: configuration values and path discovery
  It depends on `toyoterm-api` for shared immutable `ImagePixels` storage.
  Large compressible wallpaper pixels use lossless zlib storage; the shared
  type owns compression/expansion through the existing `flate2` dependency.
  This adds the `toyoterm-config -> toyoterm-api` edge, not a render/config edge.
- `toyoterm-script`: mruby ownership, DSL evaluation, callbacks, and typed API conversion
- `toyoterm-ipc`: the internal local transport shared by the app and CLI
- `toyoterm-app`: window lifecycle and coordination of the native subsystems
- `toyoterm-cli`: the `toyoterm` executable and command-line entry points

Production dependencies point toward lower-level contracts. In particular,
`toyoterm-script` depends on `toyoterm-api` and never on `toyoterm-mux`;
`toyoterm-mux` implements commands from `toyoterm-api` without depending on the
script runtime. The app is the composition root that applies commands to the
mux. `toyoterm-ipc` keeps transport code out of both the app and CLI, avoiding a
dependency cycle between those two entry-point crates.

## Dependency contract

`fuzz/` is an explicitly excluded, independent cargo-fuzz workspace. Its only
dependencies are `toyoterm-terminal` and `libfuzzer-sys`; the architecture checker
enforces this separate allowlist. It does not change the production graph or
link the app's filesystem/event consumers. Normal terminal tests reuse its
public-boundary harness to replay committed seeds without libFuzzer.

The production dependency graph has three roles:

- contract and leaf crates: `toyoterm-api`, `toyoterm-config`, `toyoterm-pty`,
  and `toyoterm-terminal`
- subsystem crates: `toyoterm-ipc`, `toyoterm-mux`, `toyoterm-render`, and
  `toyoterm-script`
- composition roots: `toyoterm-app` and `toyoterm-cli`

Subsystem crates may depend on contract or leaf crates, but not on a composition
root. `toyoterm-app` may assemble every subsystem. `toyoterm-cli` may depend on
the app and on lower-level crates needed by its diagnostic subcommands. The
production graph must remain acyclic.

Run `python3 scripts/check-crate-architecture.py` to validate the exact internal
dependency allowlist, the small allowlist of test-only dependencies, and cycle
freedom. CI runs this check on Linux, macOS, and Windows. When adding a crate or
dependency, update the script and this document in the same change so the new
direction is an explicit design decision.

Within `toyoterm-app`, `ToyotermApplication` is the composition coordinator,
not the owner of every individual field. `PlatformState` owns the window,
renderer, clipboard, and notification service; `TerminalRuntime` owns pane and
PTY runtimes; `ScriptRuntimeState` owns the bounded request/event queues and
the script-thread endpoint; and `UiState` owns transient layout and interaction
state. A pane runtime is further split into terminal VT state, native process
lifecycle, pane metadata, and protocol/session metadata. Dropping its
`ProcessRuntime` retains the PTY termination guarantee.

Native commands keep a small top-level domain boundary (`Mux`, `Action`,
`Pane`, `Window`, `Ui`, `Clipboard`, `Script`, and `Config`). `NativeCommand` is a
transport envelope: adding an operation within one domain does not grow an
unrelated application-wide command enum or put its implementation in the
application root. Ruby callbacks, IPC mutations, and native keybindings
converge on `apply_control_command`. An ingress policy first validates which
domains an origin may use, then the action resolver turns every user-level
`NativeAction` into an ID-resolved domain command. Context-bound script actions
validate and reactivate their captured hierarchy before the resolved command is
dispatched. Origin and `NativeAction` are not passed to domain handlers. Pane,
window/platform, UI, clipboard, and script handlers receive only the subsystem
state they mutate; pane badges belong to `UiCommand`, and config reload remains
an application-level effect. The top-level dispatcher therefore performs
routing and cross-domain coordination without routing PTY bytes through a
generic intent layer.

Native key actions with registered action advice are an optional script ingress.
The immutable registry mirrors advised action names. Matching key/leader actions
submit a typed `ScriptInvocation::Action` with their captured hierarchy before
native mutation; unadvised actions retain direct dispatch. The script-owned
`actions.rb` layer composes advice and returns ordinary action commands. Their
script origin prevents re-entering advice during native application. Domain
commands and PTY protocol processing remain outside this mechanism.

Inside `toyoterm-script`, `ConfigManager` remains the transactional coordinator:
fresh-VM load and validation precede the active-runtime swap. Registry decoding
belongs to `registry`, launch-command conversion belongs to `command_collector`,
immediate `require` resolution and registration transaction state belong to the
Ruby DSL, and VM mechanics remain in `runtime`.
The built-in `keymaps.rb` layer extends those transactions with named keymaps,
mode instances, and owned registration cleanup. Registry decoding mirrors their
effective global/Pane-local tables and callback-owner IDs. The app resolves
these immutable tables natively and sends typed keymap callback requests back
to the script owner; no new crate edge or shared mutable VM state is introduced.
The `transient_keymaps.rb` layer owns exclusive temporary activations and their
mode resources. Each effective scope mirrors a priority-ordered list of immutable
activation IDs, one-shot flags and key names. The native key resolver handles
prefixes and immediate retirement, then submits `TransientKeymap` requests;
Ruby dispatches to the original keymap/command/action APIs. Consumed IDs remain
retired across older snapshots until removal is acknowledged. This introduces
no new dependency or editor state.
The `prompts.rb` layer owns single-line input validation, captured result
callbacks, and bounded named histories. Input models travel as typed UI commands
and share the native selector's input/rendering path, without calling Ruby for
filtering or input. Accepted histories and follow-up prompts use the existing
transaction boundary. This adds no crate dependency.
The `command_arguments.rb` layer validates typed command schemas and collects
arguments through those existing prompts. Metadata and command argument stacks
remain script-owned; native code receives only the ordinary prompt and terminal
commands. Keyboard and palette invocation share this path with direct Ruby
command invocation, with no editor model or additional crate edge.
The `command_help.rb` layer presents bounded command/schema/key/advice metadata
through the existing selection overlay and supplies immutable definition
locations to an optional trusted callback. Loading retains a separate source
label for the main configuration, leaving `require_relative` path resolution
unchanged. No source files are opened by help and no native editor state is
introduced.
The `panels.rb` layer adds bounded read-only output views with update/close
handles. It shares the single overlay slot and mode-owned lifetime mechanism;
typed `OpenPanel`/`UpdatePanel` commands carry copied lines to the native
selector renderer. Panel updates preserve the native filter and never return
line data as user input. Dismissal reuses the existing typed callback boundary.
There is no editable text model, file persistence, additional crate edge or VM
access on the rendering/input thread.
The `timers.rb` layer adds bounded timer registrations, mode-owned task cleanup,
and transactional task cancellation. Timer descriptors are mirrored in the
immutable script registry; the app's `timers` module schedules monotonic native
deadlines and returns typed callback requests. Async workers carry a VM
generation to prevent post-reload callback ID collisions. These changes preserve
the existing crate graph and script-thread ownership boundary.
The `cooperative_work.rb` layer uses one-shot timers to process Enumerator items
in item/time-bounded slices, preserving the originating target and mode resources.
A host `Instant` function supplies monotonic elapsed time through the existing C
shim. Failure finalization occurs after request rollback so consumed iterator
input is not retried; mutable progress/diagnostics are retained without native
application mutation. The VM, Enumerator and callbacks stay on their owner
thread. This layer adds no vendored runtime edits or new crate edges.
The separate `instruction_budgets.rb` guard enables mruby's supported
`MRB_USE_DEBUG_HOOK` build option consistently in the runtime and C shim.
Per-VM C-owned counter frames run only for explicitly guarded blocks; the hook
is null outside guards. `mrb_protect_error` restores the linked frame stack
before rethrowing normal exceptions/nonlocal exits, without crossing Rust with
longjmp. Ruby checkpoints provide host-effect rollback. The VM remains confined
to its script owner; see [ADR 0008](adr/0008-opt-in-instruction-budgets.md).
The `resource_scopes.rb` layer groups VM-owned removable handles independently
of modes. Scoped command/event wrappers and existing task callbacks re-enter
the owner stack; newly enabled mode instances use activation-specific handles.
Mode lifecycle entry and explicit scope entry follow lexical precedence.
Scope/child resource arrays participate in registration checkpoints; iterative
reverse cleanup returns normal typed UI/task commands. No ownership object or
Ruby reference crosses the script-thread boundary, and no crate edge changes.
Pane runtime lifecycle observations use existing typed `RubyEvent` fields:
`pane_ready` and `pane_resized` carry Pane IDs and cell dimensions. Main-thread
PTY synchronization generates them only after success, with last-reported-grid
state suppressing unchanged sizes and preserving failed-resize retries. They
reuse asynchronous event dispatch, model caching and per-handler rollback;
no new crate edge or script/native ownership transfer is introduced.

Dynamic completion providers run on the script thread under a transaction that
discards host-managed effects. The main thread debounces input and accepts
results only for the matching prompt ID and query revision. Mode-owned prompt
lifetimes share the registration cleanup mechanism. Prompt input uses the
centralized `unicode-segmentation` dependency for grapheme editing and the
existing centralized `unicode-width` dependency for display-column clipping.
The app maps the available text width through current cell metrics and bounds
selection highlights to the overlay. Clipped input carries its caret column
separately from display text; the same render view supplies a bounded IME cursor
area to winit while overlays are open. No internal crate edge changes.
`tempfile` is used only by script tests, not production file persistence.
