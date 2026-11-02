# Terminal extensibility

The goal is Emacs-level extensibility for toyoterm as a terminal emulator.
Users must be able to add, replace, inspect, compose, and remove terminal
behavior through trusted Ruby libraries, without modifying the Rust application.
Emacs feature parity, editor workflows, file editing, editable text buffers,
and editor key bindings are outside this goal. The experimental editor layer
has been removed at the user's request.

## Scope and evidence

The [validation audit](extension-validation-audit.md) maps each requirement to
current sources and observed checks. Full-workspace/all-target compile checks
also passed from Windows for Linux x86_64 and macOS ARM64 using existing Zig/Rust
tools. These checks include the target mruby C build but do not link or run target
executables; runtime and desktop gates below remain unverified.
The subsequent full Linux `cargo test --workspace --all-targets --no-run` cross
build also passed after fixing Windows icon-resource emission for non-Windows
targets. It links Linux executables without running them; macOS final linking
and target runtime/desktop checks remain unverified.
The macOS script-test final link was attempted and failed because the Apple SDK
and target `iconv` library are unavailable; the existing cross compiler does not
remove that prerequisite. The user confirmed that no additional verification
environment is available in this session.

| Area | Current evidence | Remaining acceptance criteria |
| --- | --- | --- |
| Commands and discovery | Named commands, immutable metadata, nested invocation, rollback, key inspection; unified native/Ruby operation catalog and palette; typed Ruby arguments; searchable command help, source provenance and definition-navigation callbacks | Broader desktop help/navigation validation |
| Keymaps and modes | Inherited maps, global/Pane-local layers, multi-step native resolution, transactional mode replacement and owned cleanup; scoped transient stacks with one-shot/repeated dispatch and native cancellation | Broader desktop validation |
| Hooks and advice | Named hook inspection/priority/strategies and library composition; removable command/action advice; native key interception; PTY readiness and per-Pane grid-change observations with causal ordering and coalescing | Broader desktop lifecycle/composition validation |
| Extension UI | Status sections, selectors, input/history/completion, revision-checked updates, mode-owned overlay lifetimes; read-only output panels with update/close handles and native filtering/paging | Broader desktop/IME and extension-view validation |
| Tasks and lifecycle | Async processes, cancellation, monotonic timers and reload isolation; cooperative work/diagnostics; opt-in opcode budgets with VM-native unwind and rollback; nested resource scopes, wrapped callbacks, activation-specific modes and transactional cleanup | Broader desktop lifecycle validation; Linux/macOS budget performance |
| Loading and compatibility | Explicit trusted Ruby require, versions, capability checks and rollback; library installation/composition/lifecycle and pre-release compatibility guide; four independent libraries tested in all 24 import orders, including repeated require and fallback after removal | Desktop lifecycle validation and third-party libraries beyond bundled fixtures |
| Responsiveness and portability | Native PTY/rendering hot path, single script owner, typed request/completion boundary; measured Windows release-profile native key resolution and Ruby callback round trips | End-to-end extension keyboard/IME/UI coverage; Linux/macOS validation |

Native terminal operations, events and snapshots are the extension contract.
UI and task features supply reusable extension mechanisms. Examples demonstrate
terminal navigation, command selection, directory-based terminal creation and
clock/reminder behavior. They do not install default application key bindings.
Normal Ruby libraries and explicit loading remain the model in ADR 0007.

## Validation

The public surface is specified by [the mruby reference](mruby-api.md).
Regression coverage belongs to toyoterm-script and toyoterm-app; it checks
command/hook/keymap transactions, ownership, typed callbacks, timers and task
generations, prompt history, provider validation, provider-effect rollback,
query revision rejection, overlay cancellation and failed mode replacement.
A regression asserts that editor capabilities and APIs are absent while
terminal extension capabilities remain available.

`toyoterm-app`'s `extension_flow_tests` also runs the actual script-owner thread
against native key dispatch, overlay input and action resolution. It loads the
typed-command example, selects it through the operation palette, collects a
Japanese/emoji label, verifies Enter is suppressed during preedit, changes tabs
between argument prompts and checks that the resulting badge/zoom target the
original Pane. Cancellation produces no partial native command and permits a
retry. A dynamic-provider flow deliberately holds an old query's completion
until after new input, verifies native revision rejection, then accepts the new
candidate and checks history plus provider-effect rollback. Run it with:

```sh
cargo test -p toyoterm-app --locked extension_flow_tests
```

These are controller/script-thread integration tests without a display server;
they do not synthesize operating-system key/IME events, validate rendered text,
or establish physical-device/desktop coverage.
Native prompt clipping also has display-column regressions for ASCII, Japanese,
combining marks, emoji sequences and preedit at every cursor position and widths
0..23. Render-view tests check 1x/1.5x/2x scaling and tiny/normal windows: input
fits the available columns and candidate highlights never escape the overlay.
The same view provides a bounded native IME candidate anchor at the visible
input cursor. Tests check cursor movement, preedit, literal caret characters
inside user input, scrolling, scaled metrics and tiny windows. OS/IME candidate
placement itself remains unverified by these geometry tests.

[The completion example](../examples/completion.rb) chooses a directory and
opens a terminal Pane there. Its test traverses nested directories, excludes
files, and verifies the typed Pane launch command. Other examples are
[commands](../examples/command_palette.rb), [modes](../examples/modes.rb), and
[timers](../examples/timers.rb).
The [action-policy example](../examples/actions.rb) demonstrates modifying a
native split request, suppressing a close action based on terminal metadata,
removing the policy with its mode, and selecting native/Ruby operations together.
Action regressions cover native-name round trips, metadata immutability,
argument substitution, suppression, dispatch snapshots, rollback, continuation
lifetimes, recursion bounds, mode cleanup, reload and palette argument selection.
A native key action/real script-thread/Mux integration test proves that the
native split happens after advice and uses the replaced direction. Pure ingress
tests verify that unadvised actions and script-produced commands stay native.
The [typed-command example](../examples/command_arguments.rb) labels a terminal
Pane and optionally toggles its zoom. Argument regressions cover schema/value
validation, immutable metadata, nested advice, legacy callback arity, retry,
cancellation, palettes, completion, rollback and mode-owned input cleanup.
The [transient-map example](../examples/transient_keymaps.rb) provides one-shot
split selection and repeated Pane focus/zoom commands. Regression coverage
checks scope/stack ordering, prefix timeouts, modifiers, physical priority,
repeat suppression, cancellation, immutable inspection, edits, ownership,
closed targets, failed callbacks/reload and old-snapshot retirement. A native
resolver/real script-thread/Mux integration test verifies the selected split
occurs only after script delivery and the one-shot is no longer active.
The [help example](../examples/command_help.rb) browses metadata and copies a
selected definition location through a trusted callback. Regressions verify
root/required-file provenance, typed help content, effective Pane keys, metadata
bounds, immutable full definitions, cancellation, registration replacement,
callback rollback and mode-owned help cleanup. Definition navigation is an
explicit callback for independent libraries; core help does not read/edit files.
The [library guide](extension-libraries.md) and
[composition example](../examples/hook_composition.rb) use independent provider,
policy and observer files. Tests cover priority/tie ordering, immutable labels
and source provenance, truthy/falsy short-circuiting, dispatch snapshots,
exception rollback, recursion bounds, option validation, mode cleanup, directory
launch commands, policy refusal and observer removal across explicit imports.
The [live panel example](../examples/panels.rb) renders a terminal snapshot and
updates it from a mode-owned timer. Tests exercise typed create/update/close,
copied/empty/duplicate lines, payload limits, native read-only filtering/paging,
ID/kind rejection of late updates, dismissal, rollback, reload, mode cleanup
and automatic timer removal after closure. Rendering semantics verify bounded
visible rows and panel-specific dismissal hints without changing image snapshots.
The [cooperative word-count example](../examples/cooperative_work.rb) processes
terminal snapshot lines and publishes progress in a read-only panel. Regressions
cover monotonic clock binding, item/time boundaries, overrun statistics,
intervening requests, original-target continuation, item/configuration failure,
non-retry after rollback, cancellation restoration, resource ownership, closed
targets, reload preservation, payload bounds and panel-driven cleanup. Time
targets are cooperative between items, not arbitrary Ruby instruction limits.
The [resource-scope library example](../examples/resource_scopes.rb) toggles its
commands, keymap activation and runtime-created panel/timer as a group without
defining a mode. Regressions verify iterative reverse/nested cleanup, ownership
precedence, wrapped positional/keyword callbacks, rescued failures, atomic close
failure, late async suppression, typed prompt closure, cooperative continuation
restoration, closed targets, reload, bounded scope registration and reinstallation.
Mode handles are activation-specific and do not remove a later unowned instance.
Automatic stale-owner pruning also skips owned mode teardown on other live
Panes; an invalid owner's failing teardown cannot block subsequent requests.
The [Pane-event example](../examples/pane_events.rb) displays actual grid sizes
after PTY preparation and layout changes. Tests cover successful initial/change
delivery, unchanged/missing PTYs, failed synchronization and retry, per-Pane
coalescing, typed Ruby payloads, stale handles and context hierarchy after focus
changes. Native readiness and sizing never wait for Ruby handlers.

The native timer/script-thread integration additionally advances scheduler
deadlines by one hour and delays completion by another 30 seconds. A repeat
callback cancels itself, creates an immediate one-shot and writes a badge before
raising. The test verifies cancellation rollback, removal of the failed child
task and badge, a full interval after completion, no missed-tick replay, and
successful self-cancellation on its next callback. This uses explicit `Instant`
values without sleeping or changing the OS clock; it does not validate how a
platform's monotonic clock advances during physical suspend/resume.

The resource-scope suite also warms up 10 install/close cycles, then performs
250 cycles in five batches. Each cycle captures a distinct payload in commands,
hooks, action advice, timers, async/cooperative callbacks and combines them with
a panel, explicit/transient keymaps and a mode. It verifies that registries are
empty afterward, no cancelled async spawn escapes, the GC arena is restored and
live mruby objects stay within 64 objects of the warmed-up baseline after full
collections. This detects per-cycle callback/payload retention in the fixture;
it is not an OS working-set/GPU-memory measurement or a long desktop soak.

The following baseline passed on Windows on 2026-11-01, with GUI startup using
the terminal-directory completion, action-policy, typed-command, transient-map
and command-help examples, plus independent hook-library composition and live
panel, cooperative-work, resource-scope and Pane-event examples:

```text
cargo fmt --check
py -3 scripts/check-crate-architecture.py
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
sh scripts/check-licenses.sh
cargo run --locked -- gui-smoke-test
```

`cargo run --locked -- gui-smoke-test --extension-ui` also passed locally on
Windows. It waits for presented frames of a Ruby-created Unicode prompt and a
read-only panel, with scope closure between them. Early window closure and a
30-second event-loop timeout fail the check; the timeout does not preempt blocked
Ruby/C calls or renderer initialization. The default smoke exits at initialization;
it alone does not establish extension frame presentation. Native/X11/Wayland CI
now includes the additional check; those remote runs have not been executed for
this uncommitted worktree.

The Windows sandbox does not permit the existing hard-link filesystem test;
that test requires normal permissions. Git Bash's license check also requires
execution outside that sandbox.
The five opt-in `toyoterm-render` GPU tests also passed locally with
`cargo test -p toyoterm-render --locked -- --ignored --nocapture`: transparent
cell replacement, terminal-image clipping/blending, inactive-window tint,
Unicode bitmap composition and background alpha. They do not rasterize extension
prompt fonts or drive operating-system input. Interactive extension/IME behavior,
timer timing and suspend/resume, Linux/macOS execution, and opt-in performance
tests beyond the [recorded Windows extension-runtime measurements](performance-baselines/2026-11-01-extension-runtime.md)
require separate validation. Those measurements also passed optimized-build
instruction-budget regressions and the complete app suite. The broader
extensibility goal remains incomplete.
