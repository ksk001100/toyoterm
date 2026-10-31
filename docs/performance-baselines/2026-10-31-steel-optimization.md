# Steel GC and snapshot optimization (2026-10-31)

This change removes host-forced full collection and avoids transferring
unchanged config/theme/registration snapshots on every request. It addresses
the memory and CPU expansion reproduced in the
[pre-optimization investigation](2026-10-31-steel-memory.md).

## Implementation and behavior

- Steel's allocation-driven collector runs on the same script owner thread.
  Success, rollback and configuration loading no longer force full GC. Merely
  collecting less frequently would retain Steel 0.8.3's expansion cycle.
- A revision in the persistent Scheme application map identifies validated
  native config/registration snapshots. Settings, palettes and overrides,
  leader, bindings, events, commands and bar registrations invalidate it.
  Widget-cache, task, selector, output-queue and badge updates do not.
- Rollback restores the revision with its owning application state. Native
  cached values and revision change only after all candidate config, palettes
  and registrations validate successfully. A new VM replaces the revision on
  reload. Failed validation cannot poison a subsequent request's cache.
- Callback roots and rollback roots remain visible to the collector. No Scheme
  value crosses the request/completion boundary, no thread ownership changes,
  and native command ordering and fresh-VM reload semantics are preserved.
- Ordinary soak runs no longer insert artificial full collections. Explicit
  full-GC failure/root tests and the plain-Engine diagnostic remain available.

See [ADR 0010](../adr/0010-allocation-driven-steel-gc.md).

## Measurement conditions and evidence

Same Windows 11 build 26300, locked release dependencies and steel-core 0.8.3
as the initial investigation. The Scheme files and wallpaper hashes are
identical before/after; all 463 themes and personal plugins remain enabled.
The optimized application binary SHA-256 is
`cba9bda9b404a70ac3e844b422efb895e15746ab36e106bf24d5a61ad396e3cb`.
The initial binary hash is
`8f240a6b5810df574bbb4584cef6c22ff68ecd85df5f2adead3647a53f90188c`.

Complete local evidence is retained under
`.performance-results/steel-memory-2026-10-31-optimized/`: metadata and hashes,
measurement/analysis scripts, JSONL and CSV samples, GUI logs, request latency
records and validation logs. The initial evidence remains under
`.performance-results/steel-memory-2026-10-31/`. An intermediate headless run
with only forced GC removed is retained under
`.performance-results/steel-memory-2026-10-31-after-gc/`; it showed that snapshot
encoding still allocated heavily, motivating the second part of the change.

## Production GUI comparison

Both versions use the ordinary allocator, one quiet `cmd.exe /d /q` pane,
isolated IPC instances, identical fonts/settings and the same resource trace
flags. Each GUI idles for 30 seconds, sampled at 250 ms. Minimal and complete
personal configurations have two fresh-process runs each; the theme-only and
no-wallpaper cases have one each. No builds or other benchmarks run during GUI
sampling. CPU below is process CPU divided by wall time, where 100% means one
logical core; this machine has 32 logical processors.

| Configuration | Sampled idle working set before / after (MiB) | Idle CPU before / after (% of one core) |
| --- | --- | --- |
| Minimal | 137.62–138.38 / 135.69–135.82 | 0.05–0.10 / 0.00–0.05 |
| Settings + 463 themes | 154.41–154.43 / 152.49–152.51 | 0.16 / 0.31 |
| Personal, no wallpaper | 163.57–1,073.00 / 154.41–155.02 | 62.33 / 0.79 |
| Complete personal | 170.62–1,091.61 / 166.52–181.64 | 52.86–57.95 / 0.84–1.62 |

Complete-personal idle Private Bytes range from 203.34 to 219.15 MiB after,
versus 210.99 to 1,164.27 MiB before. Last-ten-second working-set medians in the
two optimized runs are 166.83 / 166.69 MiB, with Private Bytes medians of
204.27 / 203.50 MiB. Removing wallpaper is no longer needed to suppress the
large full-GC expansion.

The second personal GUI performs the same 30 reloads with IPC evaluation
barriers, then 30 seconds of recovery with widgets still active:

| Metric | Before | After |
| --- | --- | --- |
| Reload phase wall time, including IPC/barriers | 62.75 s | 15.70 s |
| OS peak working set | 1,186.11 MiB | 267.96 MiB |
| Sampled reload Private Bytes maximum | 1,302.11 MiB | 432.62 MiB |
| Working set over the following 30 seconds | 218.08–1,124.54 MiB | 199.40–223.85 MiB |
| Last-ten-second working set / Private Bytes medians after recovery | 775.98 / 921.57 MiB | 199.41 / 236.85 MiB |
| Recovery CPU, one-core equivalent | 60.77% | 0.89% |

An additional 180 seconds of ordinary widget updates after recovery holds
working set at 199.41–199.60 MiB and Private Bytes at 236.85–237.02 MiB. CPU is
0.96% of one core. This extension has no matching three-minute pre-optimization
run and is reported as an additional check, not a controlled longer-run ratio.
All six optimized GUIs exit normally with zero `ERROR` records. There is no
claim of constant memory for arbitrary workloads or longer sessions.

## Script-thread request measurements

The counting-allocator probe uses the same matrix as before: two fresh
processes per case, 100 top-bar requests, 100 tiny evaluations and 30 reloads
per process. Diagnostic explicit-GC commands are unchanged, and are excluded
from the ordinary-request table below. These timings include the counting
allocator's overhead; they are not uninstrumented production latency.

| Configuration | Mean top-bar request before / after (ms) | Mean tiny eval before / after (ms) | Requested allocation peak in ordinary bars/eval before / after (MiB) |
| --- | --- | --- | --- |
| Minimal | 101.48 / 0.055 | 102.37 / 0.140 | 721.98 / 23.15 |
| Settings + 463 themes | 123.73 / 0.056 | 124.31 / 0.223 | 738.98 / 37.62 |
| Personal, no wallpaper | 122.82 / 0.097 | 121.76 / 0.232 | 742.27 / 38.31 |
| Complete personal | 121.31 / 0.101 | 122.11 / 0.237 | 742.71 / 41.13 |

Each mean covers 200 requests. After the personal bar batches, outstanding
requested Rust allocations remain around 40–41 MiB, rather than recreating the
large full-GC free list. Reloads still evaluate/validate fresh VMs: personal
reload mean is 315.8 ms versus 329.5 ms before (60 reloads in each case). The
large improvement is in ordinary requests and peak memory, not config parsing.

The normal allocator release `performance_script_roundtrip` test measures the
actual script-thread boundary with a minimal config, 100 warmups followed by
1,000 samples per scenario. No external process launch is included in the
async-completion case.

| Scenario | p50 (ms) | p95 (ms) |
| --- | --- | --- |
| Dynamic Scheme key | 0.0394 | 0.0486 |
| Bell event | 0.0456 | 0.0595 |
| Async callback completion | 0.0485 | 0.0863 |

An additional fresh personal VM receives 100,000 cached top-bar requests in
100 batches of 1,000, without diagnostic forced collection. Batch medians are
around 0.06–0.07 ms; the maximum outstanding requested Rust allocation count
is 158.36 MiB. The retained pool/caches grow under this burst (about 38 MiB at
startup, 67 MiB after 20,000 requests and 158 MiB after 100,000). This is not a
claim that Steel's heap is fixed-size or that arbitrarily long churn cannot
grow further. It does avoid the original 700+ MiB expansion after only a few
ordinary requests. The separate `stress.jsonl` contains the first 20,000-request
run; `stress-100k.jsonl` contains the fresh 100,000-request run.
The probe's `dropped` record means owner-thread shutdown was requested by
dropping its handle; production shutdown deliberately does not join a possibly
stuck callback. That record is not used as a post-VM-destruction memory metric.

## Regression coverage and validation

New regressions cover:

- Allocation-driven collection during large bursts of unreachable cyclic
  closures, retained key closures and failed-callback rollback.
- Native snapshot invalidation for each public settings/registration category,
  theme replacement, registration removal, failed native validation and VM
  reload, including a repeated failed generation followed by a successful one.
- A dedicated counting-allocator test with 463 themes and 1,000 alternating
  successful and failing requests; additional requested allocations must stay
  below 64 MiB above the loaded baseline.

Passed checks:

```text
cargo fmt --check
python scripts/check-crate-architecture.py                 # 10 crates, 22 edges
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked              # 438 passed, 16 ignored
cargo test -p toyoterm-script --all-targets --locked
cargo test -p toyoterm-script --release --locked performance_script_roundtrip -- --ignored --nocapture
TOYOTERM_SOAK_ITERATIONS=30 cargo test -p toyoterm-script --release --locked soak_config_reload_and_async_registry -- --ignored --nocapture
python scripts/test-soak-test.py                           # 10 passed
sh scripts/check-licenses.sh
cargo build --release --locked
cargo build --locked
```

Python harness tests use a workspace-local temporary directory because the
managed Windows sandbox's default temporary-directory permissions rejected
file writes. The workspace native tests and GUI measurements use normal
Windows permissions. This does not constitute Linux/macOS or a multi-hour soak
validation. OS working set, private committed bytes, allocator request bytes
and GPU VRAM are different metrics and must not be added together.
