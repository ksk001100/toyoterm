# 0002: Use mruby for embedded scripting

- Status: Accepted
- Date: 2026-09-02

## Context

Configuration must support both declarative settings and user-defined runtime
behavior. The scripting engine must be embeddable, distributable with a native
binary, and independent of a system Ruby installation. It must not participate
in terminal input, PTY parsing, or rendering hot paths.

## Decision

Use a vendored mruby runtime as toyoterm's configuration and scripting engine.
Treat Ruby as a trusted control plane: scripts inspect immutable native
snapshots and produce typed commands that the native application applies after
the callback succeeds.

## Alternatives considered

- CRuby provides the full Ruby and Gem ecosystem, but is a larger external
  runtime with more complicated distribution and embedding requirements.
- Lua is small and widely embedded, but does not provide the Ruby configuration
  language that is a defining part of toyoterm.
- A data-only format would be simpler and safer, but could not express dynamic
  key bindings, event callbacks, commands, and local plugins.

## Consequences

- toyoterm owns its mruby build, C shim, typed conversions, and Ruby-facing API.
- CRuby gems and arbitrary native Ruby extensions are not available.
- The VM is kept off latency-sensitive native paths.
- Configuration and local plugins are arbitrary trusted code, not a sandbox.
- Runtime ownership and execution isolation are specified separately in
  [ADR 0006](0006-single-script-runtime.md).

