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
`Pane`, `Window`, `Ui`, `Clipboard`, and `Config`). Adding an operation within
one of those domains does not grow an unrelated application-wide command enum.
Ruby callbacks, IPC mutations, and native keybindings converge on
`apply_control_command`. It performs domain dispatch and returns the small set
of coordinator effects without routing PTY bytes through a generic intent
layer.

Inside `toyoterm-script`, `ConfigManager` remains the transactional coordinator:
fresh-VM load and validation precede the active-runtime swap. Registry decoding
belongs to `registry`, launch-command conversion belongs to `command_collector`,
plugin metadata validation belongs to `plugin`, immediate `require` resolution
belongs to the Ruby DSL, and VM mechanics remain in `runtime`.
