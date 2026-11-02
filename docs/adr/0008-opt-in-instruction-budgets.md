# 0008: Provide opt-in mruby opcode budgets through the native fetch hook

- Status: Accepted
- Date: 2026-11-01

## Context

ADR 0006 confines one mruby VM to the script owner and requires a safe hook and
measured overhead before adding instruction budgets. Cooperative work yields
between items, but an expensive Ruby item can delay all later Ruby work.
Trusted libraries need an explicit way to fail bounded Ruby analyses while
preserving host transaction and unwind semantics.

## Decision

Enable the supported `MRB_USE_DEBUG_HOOK` define for both the selected vendored
runtime and shim. Do not edit the amalgamation. Provide
`Toyoterm.with_instruction_budget(instructions:)` with an integer allowance and
normal Ruby exception on exhaustion. Default callbacks remain unlimited.
The VM's auxiliary pointer owns C budget state; linked frames stay inside the
C call boundary. Install `code_fetch_hook` only while a guard is active, count
all active frames and support at most 64 nested guards. Protect block execution
with `mrb_protect_error`; restore frames/hook before raising or propagating a
nonlocal exit. No longjmp crosses Rust.

Suspend an exhausted frame's checks so normal rescue/ensure can run. Exhaustion
wins at guard exit even over rescued returns/breaks or cleanup exceptions. The
Ruby wrapper checkpoints host state and restores it on exception. No claim of
rolling back arbitrary Ruby state, filesystem operations or process effects.
This is a trusted-code diagnostic, not sandboxing, a hard deadline or a guarantee
against a block that rescues and never returns. C functions/I/O are not preempted.
Fiber yield cannot cross mruby's C frame; resumed child Fiber opcodes are counted.

## Evidence and consequences

Focused regressions cover limits/results, ensure, nested exhaustion, rescued
errors/nonlocal exits, rollback, GC, independent VMs, Fiber boundaries, load-time
use, cooperative-item failure, and the terminal-summary example.
The same Windows test-profile Ruby loop measured 21.813 ms before the build
option, 22.860 ms with the hook inactive, and 34.239 ms with one active guard
(medians; 20 samples following warmup). This accepts a small inactive branch
cost and explicit guarded-loop cost; it does not establish release-profile or
Linux/macOS performance. See the [measurement contract](../performance-baseline.md#instruction-budget-hook).
The native terminal/rendering path and crate dependency graph are unchanged.

## Alternatives considered

- Thread termination or cross-thread exceptions violate VM ownership and cannot
  safely unwind C/Rust state.
- Automatic budgets on every callback would silently alter trusted extension
  behavior and impose active counter overhead on all Ruby work.
- A monotonic deadline check per opcode would add clock-reading cost and still
  could not interrupt synchronous native calls.
- Cooperative slices alone remain useful but cannot bound an item's Ruby work.
