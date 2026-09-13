# Threading and script execution

The GUI uses the main thread, PTY reader workers, a script thread, and an IPC
listener thread:

```text
PTY reader workers --AppEvent::Output/Eof/Error--> main thread
IPC listener        --typed IPC requests---------> main thread
Async workers       --AppEvent::AsyncCompleted---> main thread
main thread         --terminal input/state-------> PTY sessions
main thread         --ScriptRequest--------------> toyoterm-script
toyoterm-script     --ScriptCompletion-----------> main thread
```

The main thread owns the winit event loop, terminal backends, mux, renderer, and
PTY session handles. Each PTY reader owns only its blocking reader. The named
`toyoterm-script` thread constructs, calls, reloads, and drops the single mruby
VM. `MrubyRuntime` remains `!Send + !Sync`, so the C API cannot cross the owner
thread through Rust's safe type system. Dedicated background threads execute
asynchronous child processes requested via `Toyoterm.async` and report output
back to the main thread through the winit event loop.

Script requests carry an immutable mux/object-model snapshot and clipboard
snapshot. Script completions carry inspected values, context-bound
`NativeCommand`s, asynchronous spawn and cancellation requests, script log
records, and validated configuration snapshots or registries when they change,
including immutable image pixels. Before applying a non-global action, the main
thread validates its captured workspace/window/tab/pane IDs and activates that
hierarchy. Stale contexts fail before partially activating that hierarchy.
The main thread serializes requests, applies returned commands, reconciles PTY
runtimes, spawns background workers for asynchronous tasks, then submits the
next request. This preserves event and re-entrant command ordering without
allowing Ruby to mutate native state directly.

Ruby evaluation is asynchronous from the GUI's point of view. A slow or stuck
callback delays later script requests, but it does not prevent PTY output from
being parsed or frames from being scheduled and rendered. Asynchronous tasks
(`Toyoterm.async`) execute outside both the main and script threads, keeping
long-running external calls from blocking either subsystem. Cancelling an async
task suppresses its callback and discards its eventual result. A process that
has already started may continue in the background; cancellation is deliberately
not presented as an operating-system process-kill guarantee.

The main-thread request queue bounds only Ruby runtime events: at most 1,024
event requests may wait behind the active callback. State notifications for the
same object (`title_changed`, `cwd_changed`, `pane_focused`, and
`workspace_changed`) coalesce to the newest snapshot. Other events are dropped
with rate-limited warnings once that event budget is exhausted. Lossless inputs
such as key bindings, named commands, reloads, and IPC evaluations remain
ordered and are never dropped by this overload policy. Trace logging under
`toyoterm::script` reports the pending request count, native runtime-event
count, mruby GC arena index, and mruby live-object count.

## Execution-budget investigation

mruby 4.0 exposes `code_fetch_hook` and `debug_op_hook` only when built with
`MRB_USE_DEBUG_HOOK`; toyoterm's vendored build does not currently enable that
option. Enabling a fetch hook could implement a cooperative instruction counter
or deadline check, but it adds overhead to every executed opcode and needs a
well-tested, VM-native exception/unwind path.

A wall-clock timeout on another Rust thread cannot safely kill or unwind an
mruby C call. Dropping the worker or detaching it would leak the VM and leave
script ordering undefined. For that reason v0.1 deliberately provides
isolation, duration logging, and slow-callback warnings, but no unsafe forced
cancellation. A future budget should be implemented with the mruby debug hook,
benchmarked, and converted into a normal Ruby exception at a safe VM boundary.
Cancellation before a queued request starts is safe and can be added when the
public scripting API has a request/cancellation handle.
