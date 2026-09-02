# 0001: Use Rust for the native application

- Status: Accepted
- Date: 2026-09-02

## Context

toyoterm combines a native event loop, PTYs, a terminal state machine, GPU
resources, and an embedded language runtime. These resources have strict
ownership and lifecycle requirements, and the application must run on Linux,
macOS, and Windows without introducing a garbage-collected runtime into the
rendering and input paths.

## Decision

Implement the native application and its internal contracts in stable Rust.
Keep platform-specific code behind crate or module boundaries and use C only
for the narrow mruby embedding shim.

## Alternatives considered

- C or C++ would provide direct access to the same platform APIs, but would
  make ownership mistakes across callbacks, threads, and shutdown paths easier.
- A managed desktop stack would reduce some UI work, but would add another
  runtime and make direct PTY, winit, and wgpu integration less predictable.

## Consequences

- Rust ownership and `Send`/`Sync` constraints encode important threading and
  resource-lifetime rules.
- Cross-platform behavior still requires platform testing; the language does
  not remove PTY, IME, DPI, or graphics-driver differences.
- Unsafe code and C interop must stay localized and expose typed Rust APIs.
- Workspace crate boundaries and dependency direction are part of the design
  contract described in [crate architecture](../architecture.md).

