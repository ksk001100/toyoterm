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

The main thread also owns searchable selection overlays requested by Ruby.
Choosing or cancelling an item enqueues a typed `SelectCallback` request; the
saved block is resumed only on the script thread and never from winit input or
rendering code.
Read-only panels share that slot, sending typed `OpenPanel`/`UpdatePanel`
commands with copied line data. Their native filter, paging and selection index
stay on the main thread; Enter closes rather than returning a line. Dismissal
submits the same typed callback with nil selection to release script-owned
lifetime state. Updates are applied only to the matching panel ID/kind, so a
late task cannot recreate a dismissed view or overwrite another overlay.
Single-line input prompts use the same native overlay and typed completion
boundary. Their initial value, candidate list, match policy, and history snapshot
cross in an `InputPrompt`; query editing, grapheme movement, IME, filtering,
completion, and history navigation stay on the main thread. Rendering
and the OS IME candidate anchor use the same clipped input cursor metadata;
overlay anchoring takes precedence over the underlying terminal cursor and
requires no Ruby request. Final acceptance
or cancellation submits `SelectCallback`. Optional dynamic candidate providers
receive debounced `CompletePrompt` requests on the script thread; one request
is outstanding per prompt. Returned candidate updates carry a prompt ID and
query revision, so old results cannot overwrite new input. Providers discard
host-managed effects; mode cleanup closes owned prompts without firing their
result blocks. Ruby validates the result and owns
bounded named histories; updates participate in the callback transaction.
Typed command argument collection reuses those requests. Each field returns to
the script owner, which validates it and either queues the next prompt or calls
the command with the captured target. No partially collected command runs on
the main thread; no additional thread or synchronous VM call is introduced.
Timer registrations also cross in the immutable script registry. The main
thread owns monotonic deadlines, submits typed `Timer` requests only when Ruby
is available, and rearms repeating timers after completion. No timer worker or
Ruby execution runs in `about_to_wait`; due calls coalesce while Ruby is busy.
One-shots and async deliveries are consumed before the callback transaction so
failure cannot resurrect delivery. Resource creation/cancellation still rolls
back. Mode ownership propagates through timer/async callbacks on the VM thread.
Async workers stamp results with the main thread's VM generation; successful
reload drops old callbacks/schedules, while late worker output cannot target a
new registration. Tracking sets retain only workers still awaiting completion.

The main thread also owns multi-step key-sequence prefix/timeout and held-key
state. Prefix tables are rebuilt from the immutable script registry on snapshot
changes. Prefix presses never invoke Ruby; only a completed Ruby binding
submits the existing typed `KeyBinding` request. Registry changes and
input-context changes cancel pending prefixes, while held keys stay suppressed
until release.

Action advice uses the same serialized request/completion boundary. Only native
key/leader actions whose names are mirrored in the advice registry submit
`Action` requests. Those requests preserve the original action and target IDs,
while their script context contains a fresh immutable model. Ruby advice can
suppress or replace queued work transactionally; returned native actions apply
without another script round trip. Callback-side before/after ordering concerns
queuing, and native events observe application. No advice runs while parsing
PTY output, rendering, or applying a script-produced native command.

Named keymap inheritance and global/Pane-local mode activation are resolved on
the script thread into immutable binding tables for each active scope. These
tables include callback-owner IDs and layer ranks. The main thread selects a
table when the active Pane changes and uses native prefix/priority resolution.
Only a completed Ruby keymap binding submits a typed `KeymapBinding` request;
its callback is invoked in the owning keymap on the script thread. Mode lifecycle
callbacks and registration cleanup use the normal callback transaction, never
the rendering/input thread. Closed-Pane scopes are pruned before registries are
mirrored, so their owned commands/events are removed in the same snapshot.
Exclusive transient maps are another immutable per-scope descriptor in that
registry. The main thread chooses the top map, resolves prefixes, consumes
cancellation/unassigned keys and immediately retires one-shots. A lossless
`TransientKeymap` request carries the activation ID, completed key (or nil for
cancellation), and originating Pane. Preparation consumes it on the VM thread
before the callback checkpoint, preventing callback failure from resurrecting
a native one-shot. Native retirement survives intervening older snapshots and
Pane switches; successful reload clears retired IDs and queued transient
deliveries together with other old-VM callbacks. No Ruby runs for transient
prefixes or modifier-only presses.
The main thread owns the winit event loop, terminal backends, mux, renderer, and
PTY session handles. Each PTY reader owns only its blocking reader. The named
`toyoterm-script` thread constructs, calls, reloads, and drops the single mruby
VM. `MrubyRuntime` remains `!Send + !Sync`, so the C API cannot cross the owner
thread through Rust's safe type system. Dedicated background threads execute
asynchronous child processes requested via `Toyoterm.async` and report output
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
OSC wallpaper worker); expansion does not run Ruby or retain a second decoded
CPU copy.

Before applying a non-global action, the main thread validates its captured
workspace/window/tab/pane IDs and activates that hierarchy. Stale contexts fail
before partially activating that hierarchy.
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
same object (`title_changed`, `cwd_changed`, `pane_focused`, `pane_resized`,
`workspace_changed`, and `window_resized`) coalesce to the newest snapshot.
Other events are dropped with rate-limited warnings once that event budget is
exhausted. Lossless inputs such as key bindings, named commands, reloads, and
IPC evaluations remain ordered and are never dropped by this overload policy. Trace logging under
`toyoterm::script` reports the pending request count, native runtime-event
count, mruby GC arena index, and mruby live-object count.

Theme validation transfers names and color values as one length-prefixed snapshot,
instead of compiling a Ruby expression for every field of every registered theme.
The script thread collects temporary compilation objects after a successful config
load and completed script request, after native result values have been copied.
Persistent Ruby roots and transaction ordering are preserved; `GC.disable` remains
effective. The opt-in `script_gc` performance stage records collection time. No GC
or Ruby work is added to the main-thread PTY/rendering path.

Explicit resource scopes also live entirely on the script owner. Scope entry
and mode lifecycle entry use the same lexical ownership stack; existing task
callbacks carry their captured owner. Wrapped command/event/hook callbacks can
re-enter a scope, and closed wrappers return without executing their body.
Scope closure removes resources/children in reverse order under one host
checkpoint, producing the usual typed cancellation/UI commands. A scope-owned
mode handle is tied to one activation ID, so later reactivation is independent.
Closed-Pane scopes are pruned before registry mirroring. No main-thread Ruby
callbacks or shared mutable VM objects are added.
Pane readiness/resize notifications are generated on the main thread only after
successful PTY size synchronization. Each runtime retains its last reported cell
grid; unchanged/pixel-only changes do not enqueue grid events. Pending mux events
are collected before runtime readiness so creation remains causally ordered.
Size/PID model changes invalidate cached native snapshots before a script request
is built. Per-Pane resize events coalesce under the existing bounded runtime
event queue. Ruby handlers never participate in sizing or block PTY parsing.

## Cooperative work and execution budgets

The opt-in extension GUI smoke uses the same serialized request/completion
boundary: native frame presentation schedules the next Ruby UI phase, and only
the script owner creates its scope/prompt/panel. The main event loop observes
overlay kind/title and presentation, enforces a bounded wait, and never invokes
Ruby directly from rendering. Ordinary GUI startup behavior is unchanged.

`Toyoterm.work_each` runs Enumerator items in bounded batches on the script owner.
One-shot timers submit each slice through the existing typed request/completion
boundary; continuations retain their original context and mode owner. Items/time
targets are checked only between items, allowing other requests between slices
without moving the VM or its Enumerator/Fiber state to another thread.
The Rust host exposes only process-relative `Instant` elapsed time through a
small mruby C shim; the hot PTY/rendering path does not consult Ruby or this clock.

Slice host effects participate in ordinary rollback, while iterator progress and
diagnostics cannot be rewound. Request completion records a delivery failure
after rollback, drops its continuation and retains error/statistics on its handle.
This also covers configuration-validation failures after the item block returns.
Cancellation remains transactional for pending work; mode cleanup and old-VM
timer generation handling reuse the existing scheduler/lifetime mechanisms.
Items are not forcibly interrupted; an expensive item can still delay Ruby work.

### Opt-in instruction budgets

The mruby runtime and shim are built with `MRB_USE_DEBUG_HOOK`, without changes
to the vendored amalgamation. `Toyoterm.with_instruction_budget` installs a fetch
hook only for its explicit block. A C-owned stack counts opcodes against all
active guards; outside guards the hook pointer is null. No counter or VM state
crosses the script-thread boundary. `mrb_protect_error` restores the active frame
and hook before rethrowing exceptions or nonlocal exits. Exhaustion is a normal
Ruby exception, and the Ruby wrapper rolls back host effects even when the caller
rescues. See [ADR 0008](adr/0008-opt-in-instruction-budgets.md), the
[API contract](mruby-api.md#instruction-budgets), and
[measured overhead](performance-baseline.md#instruction-budget-hook).

A wall-clock timeout on another Rust thread cannot safely kill or unwind an
mruby C call. Dropping the worker or detaching it would leak the VM and leave
script ordering undefined. Instruction guards do not terminate the thread,
interrupt C/native calls or provide a wall-clock limit. Exhausted counters suspend
checks for rescue/ensure cleanup; trusted code that rescues and loops forever can
still delay later Ruby requests. Ordinary callbacks remain unlimited. Isolation,
duration logging and slow-callback warnings remain available independently.
Generic request cancellation before execution still requires a request handle;
cooperative work already supports cancelling its own pending continuation.
