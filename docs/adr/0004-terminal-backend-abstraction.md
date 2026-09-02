# 0004: Isolate the terminal backend behind an abstraction

- Status: Accepted
- Date: 2026-09-02

## Context

Correct VT parsing includes control sequences, modes, Unicode cell widths,
scrollback, selections, alternate screens, and many compatibility edge cases.
Implementing that state machine from scratch would consume effort outside
toyoterm's differentiating mux and Ruby control-plane features. At the same
time, the rest of the application must not depend on one terminal core's
internal types.

## Decision

Define toyoterm-owned terminal input, cursor, cell, and snapshot contracts
behind `TerminalBackend`. Use `alacritty_terminal` for the current VT state
machine and adapt it to those contracts in `toyoterm-terminal`.

## Alternatives considered

- A new parser and grid implementation would provide complete control, but has
  a large compatibility and fuzzing burden.
- Depending directly on backend types throughout the app would save adapter
  code initially, but couple rendering, search, selection, and PTY coordination
  to one external implementation.

## Consequences

- App and renderer code consume stable toyoterm snapshots rather than backend
  internals.
- Backend-specific conversions and compatibility fixes remain localized.
- Replacing the terminal core is possible but not free: the replacement must
  preserve snapshot, selection, search, cursor, and input semantics.
- VT corpus tests protect the behavior expected at the abstraction boundary.

