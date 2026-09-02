# Crate architecture

The cross-cutting choices behind this structure are recorded in the
[architecture decision record index](adr/README.md).

toyoterm is a Cargo workspace. Each crate owns one runtime responsibility:

- `toyoterm-api`: stable IDs, native commands, events, and handles
- `toyoterm-mux`: workspaces, windows, tabs, panes, and split trees
- `toyoterm-terminal`: VT state, snapshots, selection, and input encoding
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
