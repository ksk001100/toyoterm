# 0005: Normalize control-plane mutations into native commands

- Status: Accepted
- Date: 2026-09-02

## Context

Mux and application state can be changed by static key bindings, Ruby
callbacks, user commands, the Ruby console, and local IPC. Letting every source
mutate state directly would duplicate validation and ordering behavior, and
would allow scripting or listener threads to violate main-thread ownership.

## Decision

Represent native mutations as typed commands in `toyoterm-api`. Normalize each
control plane into these commands, queue them across thread boundaries when
necessary, and apply them through the app coordinator and mux on the main
thread. Return typed results and emit ordered lifecycle events.

Commands requested during a Ruby callback are transactional at the callback
boundary: they are applied only after successful completion. Operations that
create native objects therefore return the receiver in Ruby; callers observe
new objects through a later snapshot or event.

## Alternatives considered

- Direct callbacks into mux state would be immediate, but would couple the Ruby
  FFI to mux internals and make thread ownership and rollback unclear.
- Separate action implementations per input source would allow source-specific
  behavior, but would duplicate authorization, validation, tests, and events.
- A string command bus would be easy to extend, but would defer type errors to
  runtime parsing and weaken API evolution.

## Consequences

- Ruby, native key bindings, and IPC share mutation semantics and test paths.
- Commands and IDs form a deliberate internal compatibility boundary.
- UI-only actions may still be handled by the app coordinator, but mux changes
  go through typed mux commands.
- Asynchronous producers cannot assume a mutation is visible until its command
  has been applied and a new snapshot or event is delivered.

