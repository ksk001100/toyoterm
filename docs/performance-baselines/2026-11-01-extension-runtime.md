# Windows release-profile extension runtime measurements

Measured on 2026-11-01 against the uncommitted extension worktree based on
`df27fbd`. These are isolated local measurements, not a comparison with a clean
checkout or another terminal application. Raw summaries are retained in
[the JSON record](2026-11-01-extension-runtime.json).

Environment: Windows NT 10.0.26300.0, x86_64-pc-windows-msvc, Intel64 Family 6
Model 183 Stepping 1, rustc 1.99.1 (58cf8f9c3), LLVM 23.1.1. Cargo's release
profile uses optimization and panic unwinding. Power mode, background load and
thermal state were not controlled; GPU/display settings do not participate in
these isolated workloads.

## Commands and measurements

```sh
cargo test --release -p toyoterm-script --locked performance_instruction_budget -- --ignored --nocapture
cargo test --release -p toyoterm-script --locked performance_script_roundtrip -- --ignored --nocapture
cargo test --release -p toyoterm-app --locked performance_native_key_resolution -- --ignored --nocapture
```

| Workload | Samples after warmup | Median ms | p95 ms |
| --- | ---: | ---: | ---: |
| Ruby integer-sum loop, inactive instruction hook | 20 | 14.247 | 14.900 |
| Same loop, one active instruction guard | 20 | 18.160 | 19.034 |
| Ruby key callback round trip | 1000 | 0.5026 | 0.5839 |
| Ruby event callback round trip | 1000 | 0.5093 | 0.5859 |
| Ruby async-completion callback round trip | 1000 | 0.5264 | 0.6066 |
| Native key binding resolution, per lookup | 1000 batches | 0.0000778 | 0.0000812 |

The [instruction-budget fixture](../../crates/toyoterm-script/src/instruction_budget_tests.rs)
evaluates a 500,000-iteration Ruby integer sum and checks its result. It discards
three warmups. Both variants use the debug-hook-enabled build, with the hook
pointer null outside guarded blocks. One active guard added about 27.5% to this
loop, including its checkpoint. This run has no release build without
`MRB_USE_DEBUG_HOOK`, so it cannot establish the release cost of enabling that
build option. Nested-guard costs were not measured.

The [script-thread fixture](../../crates/toyoterm-script/src/tests.rs) sends real
requests to the owning thread and waits for completions. It excludes 100 warmups
per scenario and measures empty callbacks, model/registry/transaction handling,
GC and channel/scheduler overhead together. The async scenario registers a real
callback before timing, then supplies a typed result; it does not launch or time
an external process. These are serialized single-request latencies, not GUI
keyboard-to-pixel latency or concurrent throughput.

The [native-key fixture](../../crates/toyoterm-app/src/command_dispatch.rs) resolves
a fixed native action from a script snapshot, without entering Ruby. Each sample
averages 1000 resolutions; 100 warmup batches are excluded. The stored unit is
milliseconds per lookup (about 0.078 microseconds at the median), not milliseconds
for a whole batch. This excludes OS input, sequence-prefix processing, action
execution, PTY I/O and rendering; it must not be presented as user-visible key
latency or compared directly with a complete Ruby callback round trip.

## Optimized-build correctness and remaining scope

```sh
cargo test --release -p toyoterm-script --locked instruction_budget -- --nocapture
cargo test --release -p toyoterm-app --all-targets --locked
```

The instruction-budget suite passed 10 tests, including normal VM unwind,
nonlocal exits, nested exhaustion, GC, Fiber boundaries and host rollback.
The app suite passed 139 tests; its opt-in performance test remained ignored in
that correctness run. The Windows hard-link test required ordinary permissions.

These results do not validate OS IME placement, complete extension UI rendering,
timer timing across sleep/wake, Linux/macOS performance or end-to-end extension
latency. See [the acceptance matrix](../extensibility.md) for remaining work.
