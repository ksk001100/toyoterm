# Steel memory investigation (2026-10-31)

This is the pre-optimization baseline. See the
[subsequent optimization and comparison](2026-10-31-steel-optimization.md)
for the changed collector policy and snapshot reuse.

The current Steel integration has a reproducible memory and CPU problem:
forcing full collection after each script request repeatedly expands Steel's
free-list storage, even when the application retains little live data. Theme
count and wallpaper storage do not explain the approximately 1 GiB process
peaks. This investigation adds an opt-in probe; it does not change production
collection policy.

## Environment and evidence

Windows 11 build 26300, x86-64 MSVC, 32 logical processors, locked release
build, steel-core 0.8.3. The working tree includes the uncommitted Scheme
migration and personal configuration port. Application binary SHA-256:
`8f240a6b5810df574bbb4584cef6c22ff68ecd85df5f2adead3647a53f90188c`.

Raw evidence is retained locally in
`.performance-results/steel-memory-2026-10-31/`: `metadata.json` (including
binary/config hashes), `measure.ps1`, `analyze.ps1`, `headless.jsonl`,
`headless-memory.csv`, `gc-only.jsonl`, `gui-memory.csv`, `gui-*.log`, and
`summary.json`. This directory is ignored by Git. An initial 1,000-request
batch was stopped to investigate its high resource consumption; its partial
record is `aborted-large-batch.jsonl`, excluded from the results below.

Process working set and Private Bytes are separate Windows metrics. Private
Bytes describe private committed virtual memory, not resident RAM. Neither
includes dedicated GPU VRAM or child process memory. The probe's requested
Rust allocation bytes exclude allocator overhead, stacks and direct OS/driver
allocations; they are not an independent measurement of Scheme object liveness.

## Production GUI measurements

Each fresh GUI has an isolated IPC instance, a quiet `cmd.exe /d /q` pane and
30 seconds of idle sampling at 250 ms. The GUI uses the normal allocator;
there is no counting allocator in this executable. The personal fonts are
JetBrainsMono Nerd Font 20 with Hack Nerd Font fallback, opacity 0.95.
`TOYOTERM_LOG=warn,toyoterm::soak=trace` enables resource diagnostics equally
in every case. No compilation or other benchmark ran during GUI sampling.
External shell/Git/PowerShell child memory and CPU are excluded.

All values below cover the 30-second idle phase. The minimum/maximum are
sampled working set, while the median covers the last ten seconds. For the
oscillating cases, the median strongly depends on which GC phases land in that
window; the range is more useful than a single idle number.

| Configuration / run | Working set range (MiB) | Last-ten-second median working set / Private Bytes (MiB) | Sampled max Private Bytes (MiB) | CPU (% of one logical core) |
| --- | --- | --- | --- | --- |
| Minimal / 1 | 138.36–138.38 | 138.38 / 152.71 | 152.71 | 0.10 |
| Minimal / 2 | 137.62–137.65 | 137.65 / 150.77 | 150.77 | 0.05 |
| Settings + 463 themes / 1 | 154.41–154.43 | 154.43 / 170.65 | 170.65 | 0.16 |
| Personal, no wallpaper / 1 | 163.57–1,073.00 | 619.18 / 662.80 | 1,133.69 | 62.33 |
| Complete personal / 1 | 176.66–1,091.61 | 284.14 / 334.67 | 1,164.27 | 52.86 |
| Complete personal / 2 | 170.62–1,090.48 | 554.37 / 626.68 | 1,162.07 | 57.95 |

On this 32-logical-CPU machine, 53–58% of one core is approximately 1.7–1.8%
of total machine CPU. These figures are process user+kernel time divided by
wall time, not a single thread utilization measurement. Theme registration
alone is stable; adding active widget/callback traffic exposes the forced-GC
cycle. Removing wallpaper does not remove it.

The second complete-personal GUI then performed 30 reloads, each followed by
an IPC evaluation barrier verifying `font.size = 20`. The whole reload phase
took 62.75 seconds, including IPC client launch/barriers and normal concurrent
bar work; this is not the isolated configuration evaluation latency. Sampled
working set reached 1,170.88 MiB and Private Bytes 1,302.11 MiB. Windows'
process peak-working-set counter reached 1,186.11 MiB (it can capture peaks
missed between samples).

During the following 30 seconds with normal widgets still active, working set
continued oscillating between 218.08 and 1,124.54 MiB; Private Bytes reached
1,261.77 MiB. The last-ten-second medians were 775.98 / 921.57 MiB. CPU was
60.77% of one core. This does not demonstrate memory recovery to the minimal
baseline or prove a monotonically accumulating leak. All six GUI processes
exited normally, with zero `ERROR` log records.

## VM-only measurements

`crates/toyoterm-script/examples/memory_probe.rs` uses the production named
script thread and request/completion boundary, with one pane/context snapshot.
Its counting global allocator records outstanding requested bytes and peaks.
The executable waits before constructing a VM, providing a process baseline
of approximately 0.8 MiB Private Bytes. No renderer, PTY, font database or
external children run in this probe. Only the top bar is requested, so the
personal Git async widgets are not launched. Runs reuse the same `STEEL_HOME`;
filesystem caches are not flushed. Each run uses a fresh process.

Each case has two fresh processes, each with startup, five batches of 20 top-bar
requests, five batches of 20 `(+ 1 2)` evaluations, explicit GC, 30 reloads,
another explicit GC and VM teardown. Windows process memory is sampled every
100 ms while commands run. Explicit `gc` also encounters the integration's
ordinary post-request collection, so it is not a neutral measurement barrier.

| Configuration | Outstanding Rust bytes after startup, runs 1 / 2 (MiB) | Private Bytes after startup, runs 1 / 2 (MiB) | Startup, runs 1 / 2 (ms) |
| --- | --- | --- | --- |
| Minimal, personal fonts/opacity | 24.32 / 24.32 | 40.05 / 41.72 | 289 / 320 |
| Personal machine settings + 463 themes | 36.49 / 35.89 | 56.90 / 56.37 | 390 / 410 |
| Personal plugins, no wallpaper | 36.65 / 39.56 | 57.87 / 61.55 | 497 / 412 |
| Complete personal configuration | 40.00 / 39.94 | 61.11 / 60.27 | 510 / 480 |

Registering all themes adds approximately 12 MiB of outstanding requested Rust
allocations at startup. The entire personal VM/config setup is approximately
40 MiB at this checkpoint. These startup figures must not be mistaken for
steady memory under callbacks.

| Configuration | Mean top-bar round trip (ms) | Mean tiny eval round trip (ms) | Mean reload (ms) | Largest requested Rust allocation peak during bars/eval (MiB) |
| --- | --- | --- | --- | --- |
| Minimal | 101.5 | 102.4 | 134.3 | 722.0 |
| Settings + themes | 123.7 | 124.3 | 248.2 | 739.0 |
| Personal, no wallpaper | 122.8 | 121.8 | 269.7 | 742.3 |
| Complete personal | 121.3 | 122.1 | 329.5 | 742.7 |

Each bars/eval mean covers 200 requests; each reload mean covers 60 reloads.
Counting allocator atomics affect these timings: they are diagnostic timings,
not an uninstrumented latency baseline. Nevertheless, instrumenting only the
probe cannot explain the peaks reproduced in the ordinary production GUI.
Minimal bar batches end near 24.4 MiB outstanding allocations despite their
722 MiB within-batch peaks. In the complete personal case, 30 reloads end near
46 MiB; this small post-reload increase alone is not evidence of a leak.

## Reproduction without toyoterm callbacks

The probe's `--gc-only` mode constructs a plain Steel `Engine`, defines one
no-op procedure and one procedure invoking `#%gc-collect`, and calls each 22
times. It does not construct `ConfigManager`, load personal configuration, run
callbacks or retain results. The allocator and timings have the same caveats.

The no-op leaves outstanding allocation bytes at 22.46 MiB, with calls around
0.01 ms. Consecutive forced GC calls produce:

| Forced collection | Outstanding requested Rust allocations (MiB) | Time (ms) |
| --- | --- | --- |
| 1 | 23.84 | 1.20 |
| 3 | 32.04 | 6.03 |
| 5 | 64.85 | 18.91 |
| 7 | 196.10 | 88.47 |
| 9 | 721.10 | 404.25 |
| 10 | 22.47 | 356.21 |
| 19 | 721.32 | 307.79 |
| 20 | 22.47 | 351.77 |

This repeating expansion/contraction is sufficient to explain a major part of
the request peaks; it is not monotonically retained application data.

Source inspection confirms the mechanism in locked steel-core 0.8.3:

- `src/steel_vm/vm.rs::gc_collect` requests `force_full = true`.
- `src/values/closed.rs::value_collection` enters the full-collection branch
  when forced and calls `memory_free_list.grow()` until its growth counter
  exceeds `RESET_LIMIT = 9`, then compacts it.
- `FreeList::grow_by` appends at least the current slot count, doubling storage
  as it grows. This happens even when a forced collection found plenty of free
  slots.
- toyoterm's `run_script_request` invokes
  `manager.runtime.collect_garbage()` after every completed request, including
  cached/empty bar requests. Failure and configuration-load paths also collect.

The first correction to investigate is the forced full-GC policy, using
Steel's ordinary allocation-driven collector where appropriate. Merely forcing
GC less frequently would delay the same expansion cycle. Any policy change
must retain owner-thread execution, rollback roots, failed-callback isolation,
task/selector cleanup and cyclic closure handling, and then repeat this matrix.
After that, theme/registry snapshot transfer is a separate optimization target.

## Commands and limits

```powershell
cargo build --release --locked
cargo build -p toyoterm-script --example memory_probe --release --locked
target/release/examples/memory_probe.exe --gc-only
# The interactive probe accepts: start ABSOLUTE_CONFIG_PATH, bars 20,
# eval 20, reload 30, gc, quit. Responses are JSON lines.
cargo clippy -p toyoterm-script --example memory_probe --locked -- -D warnings
cargo fmt --check
# Local Windows GUI and process-memory evidence driver:
# & ./.performance-results/steel-memory-2026-10-31/measure.ps1 -Mode Headless
# & ./.performance-results/steel-memory-2026-10-31/measure.ps1 -Mode Gui
# & ./.performance-results/steel-memory-2026-10-31/analyze.ps1
```

This is a short Windows investigation, not a long-run leak verdict or a
Linux/macOS validation. Scrollback growth, sustained PTY throughput, input
latency and GPU residency were not measured. The October 13 mruby personal
baseline (approximately 131 MiB working set / 165 MiB Private Bytes) is useful
historical context, but is not a controlled same-day comparison: installed
fonts, OS environment and shell differ. No claim of an exact migration
regression percentage is made.
