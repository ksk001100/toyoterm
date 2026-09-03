# AGENTS.md

This file gives coding agents the project-specific context needed to change
toyoterm safely. It applies to the whole repository.

## Project overview

toyoterm is an experimental terminal emulator written in Rust (edition 2024).
The terminal hot path is native; embedded mruby is reserved for trusted
configuration, callbacks, plugins, and commands. The primary development
platform is Linux, but production code is expected to remain portable to macOS
and Windows.

The repository is a Cargo workspace. Its default member and executable are
`crates/toyoterm-cli` / `toyoterm`.

## Repository map

- `crates/toyoterm-api`: stable IDs, events, handles, and native command types
- `crates/toyoterm-mux`: workspace, window, tab, pane, and split-tree state
- `crates/toyoterm-terminal`: VT state, terminal input, selection, and snapshots
- `crates/toyoterm-pty`: platform PTY spawning, I/O, resizing, and child lifecycle
- `crates/toyoterm-render`: layout, GPU rendering, and text rendering
- `crates/toyoterm-config`: configuration values and config-path discovery
- `crates/toyoterm-script`: mruby VM, DSL, plugins, callbacks, and API conversion
- `crates/toyoterm-ipc`: local IPC transport shared by the app and CLI
- `crates/toyoterm-app`: GUI lifecycle and native subsystem coordination
- `crates/toyoterm-cli`: executable entry point and CLI subcommands
- `shell-integration/`: bash, zsh, fish, and PowerShell integrations
- `docs/adr/`: accepted architecture decisions
- `scripts/`: architecture, license, packaging, and smoke-test tooling
- `vendor/mruby/`: vendored mruby amalgamation; avoid incidental edits

Read `docs/architecture.md` before changing crate dependencies and
`docs/threading.md` before changing runtime ownership or message flow. Relevant
feature-specific documents under `docs/` are part of the implementation
contract, not background material.

`docs/mruby-api.md` is the canonical user-facing reference for the mruby
configuration DSL and runtime API. Read it before changing Ruby-visible
behavior.

## Architectural invariants

- Keep the production crate graph acyclic and within the allowlist enforced by
  `scripts/check-crate-architecture.py`.
- Lower-level crates must not depend on a composition root. In particular,
  `toyoterm-script` depends on `toyoterm-api`, not `toyoterm-mux`; the app
  applies script-produced `NativeCommand`s to the mux.
- The winit event loop, terminal backends, mux, renderer, and PTY handles belong
  to the main thread. PTY reader workers own only blocking readers.
- The single mruby VM belongs to the named script thread. Do not make
  `MrubyRuntime` cross threads or bypass the request/completion boundary.
- Ruby callbacks receive snapshots and return inspected values and native
  commands. They must not directly mutate native application state.
- Preserve ordering when touching script requests, command application, PTY
  reconciliation, or event dispatch. Slow Ruby may delay Ruby work, but must not
  block PTY parsing or rendering.
- Configuration and local plugins are trusted code, not sandboxed code. Do not
  claim or imply a security boundary that the implementation does not provide.
- Keep platform-specific behavior behind existing abstractions and `cfg`
  boundaries. Do not fix one desktop backend by silently breaking another.

If a change intentionally alters one of these decisions, add or supersede an
ADR instead of quietly weakening the invariant. New ADRs use the next
four-digit number; accepted ADRs are not rewritten to hide later reversals.

## Working conventions

- Make the smallest coherent change and preserve existing public behavior unless
  the task explicitly changes it.
- Follow the local Rust style and let `rustfmt` format Rust code. Treat all
  Clippy warnings as errors.
- Prefer typed commands, events, snapshots, and errors over ad-hoc coupling
  between crates.
- Add tests alongside the owning crate. Use integration tests for behavior that
  crosses a public boundary and focused unit tests for local state transitions.
- Rendering snapshot changes (`*.snap` and `*.ppm`) must be intentional and
  visually or semantically reviewed; do not refresh them merely to make a test
  pass.
- Do not edit generated output in `target/` or packaged output in `dist/`.
- Do not modify `vendor/mruby/` unless the task is specifically an mruby vendor
  update or requires a reviewed change to the embedded runtime.
- When user-facing behavior, configuration, or CLI syntax changes, update both
  `README.md` and `README.ja.md`, relevant examples, and focused docs together.
- When adding, removing, renaming, or changing any Ruby-visible setting, DSL
  method, callback context, object-model member, event, command, plugin hook, or
  host API, update `docs/mruby-api.md` in the same change. Include signatures,
  accepted values, return values, errors, execution/rollback semantics, and an
  example where useful. Do not document methods beginning with `__` as public
  API.
- When adding a workspace crate or internal dependency, update both
  `scripts/check-crate-architecture.py` and `docs/architecture.md`.
- Keep dependency versions centralized in the root `Cargo.toml` when they are
  shared. Use locked dependency resolution for verification.

## Validation

Run focused tests while iterating, for example:

```sh
cargo test -p toyoterm-terminal
cargo test -p toyoterm-script --test callback_isolation
```

Before handing off a normal code change, run the checks relevant to the touched
area. The full CI-equivalent baseline is:

```sh
cargo fmt --check
python3 scripts/check-crate-architecture.py
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
sh scripts/check-licenses.sh
```

Useful native smoke tests are:

```sh
cargo run --locked -- pty-demo
cargo run --locked -- screen-demo
cargo run --locked -- gui-smoke-test
```

The GUI smoke test requires a working display and GPU/software-rendering stack.
On Linux CI, X11 runs under Xvfb and Wayland uses
`scripts/wayland-smoke-test.sh`. If the environment cannot support an applicable
GUI or platform test, report that explicitly rather than treating it as passed.

Packaging and release work has additional requirements in `docs/packaging.md`,
`docs/platform-validation.md`, and `docs/releasing.md`. Do not create or verify a
release from the generic validation list alone.

## Definition of done

A change is ready when it preserves the architecture and threading contracts,
has regression coverage appropriate to its risk, passes the applicable checks,
and keeps English/Japanese user documentation synchronized. In the handoff,
state what changed, which commands were run, and any platform or GUI validation
that remains outstanding.
