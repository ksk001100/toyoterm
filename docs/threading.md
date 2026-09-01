# Threading and script execution

The GUI uses four ownership domains:

```text
PTY reader workers --AppEvent::Output/Eof/Error--> main thread
main thread         --terminal input/state-------> PTY sessions
main thread         --ScriptRequest--------------> toyoterm-script
toyoterm-script     --ScriptCompletion-----------> main thread
```

The main thread owns the winit event loop, terminal backends, mux, renderer, and
PTY session handles. Each PTY reader owns only its blocking reader. The named
`toyoterm-script` thread constructs, calls, reloads, and drops the single mruby
VM. `MrubyRuntime` remains `!Send + !Sync`, so the C API cannot cross the owner
thread through Rust's safe type system.

Script requests carry an immutable mux/object-model snapshot and clipboard
snapshot. Script completions carry only inspected values and `NativeCommand`s.
The main thread serializes requests, applies returned commands, reconciles PTY
runtimes, then submits the next request. This preserves event and re-entrant
command ordering without allowing Ruby to mutate native state directly.

Ruby evaluation is asynchronous from the GUI's point of view. A slow or stuck
callback delays later script requests, but it does not prevent PTY output from
being parsed or frames from being scheduled and rendered.

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
