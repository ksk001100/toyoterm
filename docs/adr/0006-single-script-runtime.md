# 0006: Own one mruby runtime on a dedicated script thread

- Status: Accepted
- Date: 2026-09-02

## Context

mruby VM state is mutable and its C API is not safe to access concurrently.
Ruby callbacks may run user filesystem or child-process operations and can be
slow, while PTY reading and frame scheduling must continue. Multiple VMs would
also make configuration, plugin state, callback order, and reload semantics
ambiguous.

## Decision

Create one mruby VM on a named script thread and keep all construction,
evaluation, callback invocation, reload, and destruction on that thread. The
main thread sends owned requests and immutable snapshots; the script thread
returns owned results and typed native commands.

Serialize script requests. Deliver runtime events FIFO, appending events caused
by callbacks to the tail. Reload by building and validating a candidate VM on
the script thread, then atomically replacing the active VM only on success.

## Alternatives considered

- Running mruby on the GUI thread would simplify calls but allow slow callbacks
  to block terminal coordination and rendering.
- A VM per callback would isolate state but lose persistent configuration and
  plugin objects and add substantial setup cost.
- A pool of VMs would add concurrency, but would require state replication and
  would no longer provide deterministic callback and event ordering.
- Forcefully timing out a C call from another thread is not a safe cancellation
  mechanism.

## Consequences

- Rust's type system prevents the VM wrapper from crossing its owner thread.
- Slow Ruby delays later Ruby work but not PTY parsing or frame scheduling.
- Callback order is deterministic, at the cost of no parallel Ruby execution.
- Reload either installs a complete candidate configuration or preserves the
  previous runtime.
- A future instruction budget must use a safe mruby hook and demonstrate
  acceptable overhead; thread termination is not an acceptable timeout.
- Detailed ownership and message flow are documented in
  [threading and script execution](../threading.md).

