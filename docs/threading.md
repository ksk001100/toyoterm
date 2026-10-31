# Threading and script execution

The GUI uses the main thread, PTY reader workers, a script thread, and an IPC
listener thread:

```text
PTY reader workers --bounded Output/Eof/Error----> main thread
IPC listener        --typed IPC requests---------> main thread
Async workers       --AppEvent::AsyncCompleted---> main thread
main thread         --terminal input/state-------> PTY sessions
main thread         --ScriptRequest--------------> toyoterm-script
toyoterm-script     --ScriptCompletion-----------> main thread
```

The main thread also owns searchable selection overlays requested by Scheme.
Choosing or cancelling an item enqueues a typed `SelectCallback` request; the
saved block is resumed only on the script thread and never from winit input or
rendering code.

The main thread owns the winit event loop, terminal backends, mux, renderer, and
PTY session handles. Each PTY reader owns only its blocking reader. The named
`toyoterm-script` thread constructs, calls, reloads, and drops the single Steel
VM. `SteelRuntime` remains `!Send + !Sync`, so the VM cannot cross the owner
thread through Rust's safe type system. Dedicated background threads execute
asynchronous child processes requested via `async!` and report output
back to the main thread through the winit event loop.

Each pane's PTY reader coalesces consecutive reads behind one winit wakeup. Its
pending userspace output is capped at 1 MiB; when the main thread falls behind,
the reader waits for the buffer to drain and lets the operating-system PTY
provide backpressure. This prevents high-volume output from creating an
unbounded queue of separately allocated 8 KiB application events while
preserving byte order and EOF ordering.

Script requests carry an immutable mux/object-model snapshot and clipboard
snapshot. The object model, handle list, and clipboard text use `Arc` ownership
so cloning a request context cannot deep-clone the snapshot; no lock or mutable
native state crosses the boundary. Equal object-model and handle snapshots
reuse their previous `Arc`; trace logging records construction time, reuse, and
pane count for lightweight scaling measurements. Script completions carry
inspected values, context-bound
`NativeCommand`s, asynchronous spawn and cancellation requests, script log
records, and validated configuration snapshots or registries when they change,
including immutable image pixels. The main-thread renderer expands losslessly
compressed wallpaper RGBA bytes in row chunks into a mapped GPU upload buffer,
then submits one buffer-to-texture copy without waiting for GPU completion. The
decoded scratch buffer is at most 256 KiB (or one row if larger); no full decoded
CPU allocation is made. Image-file I/O, decoding,
validation, and compression remain on the script thread (or the existing native
OSC wallpaper worker); expansion does not run Scheme or retain a second decoded
CPU copy.

Before applying a non-global action, the main thread validates its captured
workspace/window/tab/pane IDs and activates that hierarchy. Stale contexts fail
before partially activating that hierarchy.
The main thread serializes requests, applies returned commands, reconciles PTY
runtimes, spawns background workers for asynchronous tasks, then submits the
next request. This preserves event and re-entrant command ordering without
allowing Scheme to mutate native state directly.

Scheme evaluation is asynchronous from the GUI's point of view. A slow or stuck
callback delays later script requests, but it does not prevent PTY output from
being parsed or frames from being scheduled and rendered. Asynchronous tasks
(`async!`) execute outside both the main and script threads, keeping
long-running external calls from blocking either subsystem. Cancelling an async
task suppresses its callback and discards its eventual result. A process that
has already started may continue in the background; cancellation is deliberately
not presented as an operating-system process-kill guarantee.

The main-thread request queue bounds only Scheme runtime events: at most 1,024
event requests may wait behind the active callback. State notifications for the
same object (`title_changed`, `cwd_changed`, `pane_focused`,
`workspace_changed`, and `window_resized`) coalesce to the newest snapshot.
Other events are dropped with rate-limited warnings once that event budget is
exhausted. Lossless inputs such as key bindings, named commands, reloads, and
IPC evaluations remain ordered and are never dropped by this overload policy. Trace logging under
`toyoterm::script` reports the pending request count, native runtime-event
count. VM collection stays on the script thread.

Native configuration, registry, queue and theme reads use typed Steel calls;
they do not compile source expressions for each field on every request. Strings,
object models, events and process outputs cross the host boundary as Steel values,
not interpolated source. Steel's allocation-driven collector runs only on the
owner thread while Scheme executes. The host does not force a full collection
after requests or configuration loads: in Steel 0.8.3, forced collections grow
free-list storage even when it is mostly unused. Persistent closures, task
handles and rollback checkpoints remain rooted during automatic or explicitly
user-requested collection. See [ADR 0010](adr/0010-allocation-driven-steel-gc.md).

Native config/registry snapshots are decoded only when their revision changes.
The revision belongs to the same persistent Scheme state as settings, themes,
bindings, events, commands, leader and bar registrations, so rollback restores
it together with those values. Widget caches, process queues, badges and object
snapshots do not invalidate it. A candidate snapshot is fully validated before
replacing any native cached values or its revision. Fresh-VM reload replaces
the revision with the candidate generation's revision.

## Execution budgets

There is no forced callback timeout. Slow-callback logging starts at 100 ms.
A future budget must use a supported Steel VM interruption boundary and preserve
transaction and request ordering. Detaching or terminating a running thread is
not an acceptable mechanism for cancelling an evaluation.
