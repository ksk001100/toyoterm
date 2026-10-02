# End-to-end performance baseline (v0.2.0)

The opt-in baseline uses the production GUI, native PTYs and reader workers,
terminal parser, application redraw scheduling, snapshots, glyphon/wgpu and
surface presentation. It complements the existing terminal benchmarks; those
remain unchanged. This measures toyoterm commit A versus commit B on the same
machine, not a competition with other terminals.

## Run and retain evidence

Use a working display/GPU, Python 3 and a release binary. Keep the window visible,
disable power saving, and use the same fonts, display scale, GPU/backend, window
manager, power settings and instrumentation for both commits. On Linux, Xvfb
and a software GPU are usable but constitute a separate baseline from physical
desktop hardware. Windows uses ConPTY; macOS uses the native PTY and Metal.

```sh
cargo build --release --locked
python3 scripts/performance-baseline.py run \
  --executable target/release/toyoterm \
  --output .performance-results/v0.2.0-machine-A \
  --samples 5 --label v0.2.0 --ruby \
  --notes 'CPU/GPU, OS, fonts/versions, display scale, backend, power mode'
```

On Windows use `python`, `target/release/toyoterm.exe`, and PowerShell line
continuations (or put the command on one line). No shell or Neovim installation
is required for the output fixtures. Paths and commands are passed as argument
arrays. Each GUI has an isolated IPC instance and config; user config is not
loaded. The generated fixture is preloaded in the child, then a file gate starts
its output after a successfully presented ready marker. Split children share
the gate. The final marker must be presented for **every** pane. Child startup,
fixture generation, and process/GPU startup are outside the measured interval.
Python's Windows console transport requires complete UTF-8 writes; fixture
chunks respect character boundaries. Existing terminal fragmentation tests
continue to cover arbitrary PTY splits.

There is one unrecorded warmup run for each case and multiple fresh-process
samples. This is a cold application-content baseline (font/image caches warmed
only by the ready screen), not a long-running application's steady state.
Image repeat and cached redraw are additional phases in the same process. `--quick --samples 1`
selects one small case per scenario for diagnostic validation; do not treat it
as the release baseline. `--scenario panes` and the other scenario names run
only a selected matrix. `--build-profile debug` records a diagnostic debug build
accurately; never compare it to release results.

The output directory must be new. It contains:

- `metadata.json`: schema, label, commit, dirty state, binary SHA-256, version,
  OS/CPU, profile, Python version, harness hash, notes and scenario/sample selection.
- `results.jsonl`: one JSON object per completed case/sample; failed runs are
  explicitly recorded and the driver exits unsuccessfully.
- `ruby.jsonl` and `ruby.log` when `--ruby` is selected.
- Per-case config, exact fixture bytes and SHA-256, producer timestamps, gates,
  and the complete GUI log (including adapter/backend and surface size).

Retain the **whole directory**, optionally archived as a release/benchmark
artifact. The ignored `.performance-results/` directory supports storing
v0.2.0 evidence locally without committing machine-specific numbers. A reviewed
record may be copied under `docs/performance-baselines/` with its environment
description. Never replace absent measurements with invented numbers or mark
an unsupported GUI run as passed.

## Scenario matrix

| Scenario | Reproducible workload | Result |
| --- | --- | --- |
| A: bulk | ASCII, ANSI colors, Unicode, mixed; exactly 1 and 10 MiB per pane | Bytes, MiB/s, producer-to-present duration, frames, parser time, main-thread work |
| B: redraw | 200 alternate-screen redraws at fixture sizes 80×24, 120×40, 200×60; CUP, SGR, ED, EL, rewrite, DEC 2026 synchronized update | Duration, snapshots, pane update/shaping, frames |
| C: scroll | 1,000 / 10,000 / 100,000 fixed lines, 100,000-line scrollback budget | Parser, snapshots, renderer update, event count, memory |
| D: panes | 1 / 2 / 4 / 8 panes in one tab/window, each producing 1 MiB of mixed output with 1 ms per batch pacing | Aggregate throughput, process CPU, frame p50/p95, memory, event count |
| E: search | 100,000 lines then `current_pane.search('needle')` | Active pane search, overlay update and first presented frame |
| F: images | Sixel, Kitty PNG and iTerm2 PNG; 60² / 300² / 1200² pixels (well below the image limits) | First, repeat and cached redraw duration, decode/placement, upload, retained/uploaded images |
| G: scripting | Static key resolver, Ruby key, bell event, `Toyoterm.async` callback completion | Separate machine-readable overhead distributions |
| H: reload | Small config, and config registering 500 commands | Evaluate, validate, VM swap, app/config apply, presented frame |

Full-screen fixtures calibrate the window before measurement: the driver
restarts at adjusted sizes until the actual terminal grid equals 80×24, 120×40
or 200×60. This is a bounded preparation step outside the producer gate. Display
constraints that prevent the requested grid fail the run explicitly. Each result
also records `actual_grids`; the other scenarios record the OS-selected geometry.
Split cases keep
the total window size constant, so each additional pane has a smaller viewport.

Sixel and iTerm2 repeats retransmit and decode the image. Kitty repeat uses an
existing image ID and a new placement (no retransmission); `uploaded_images`
shows whether renderer cache reuse occurred. Cache hits are observations, not
assumptions. A third `cached_redraw` phase moves the cursor and presents the
retained placements without transmitting or decoding an image, exposing actual
GPU texture reuse independently of protocol retransmission. Image phases use
OSC title markers only, since writing completion text over a large image would
remove its placement. A missing retained image or initial upload fails the benchmark rather than producing
a misleading zero-cost image result. PTY implementations that filter image
protocols need a separate supported backend run.

## Metric meanings and instrumentation

The driver enables `TOYOTERM_LOG=warn,toyoterm::perf=trace`. Detailed clock reads
are gated on this trace target and disabled in ordinary builds/runs. No new
Ruby-visible API or cross-thread VM access is introduced. Stage events include
`stage`, `duration_us`, and integer epoch `ts_ns`; presented completion markers
include pane ID and grid. Epoch timestamps align the child and app on one host;
do not change the system clock during a run. Log polling does not determine the
reported end time, and the driver reads logs incrementally to limit observer
overhead. Trace logging still has overhead; compare identically instrumented runs.

- `duration_ms`: producer start to successful present containing its completion
  marker; includes PTY transport/backpressure, parsing and redraw scheduling.
  Presentation means the CPU submitted/presented the frame, not photons on the
  screen or GPU completion. `mib_s` uses fixture bytes, excluding marker bytes
  and any PTY translation. Multi-pane duration spans earliest start/latest end.
- `frame`: CPU wall duration of RedrawRequested, including snapshot/update,
  acquisition, submission and present. Attempts skipped by the surface may
  appear here; `frame_count` counts only successful `present` calls. This is
  neither refresh interval nor GPU execution time.
- `main_thread_work_max_ms`: largest measured PTY event or redraw CPU wall
  interval. It is a stall **proxy**, not a sampled input-latency or scheduler
  stall probe; sleeps, OS scheduling and trace overhead can contribute.
- `process_cpu_ms`: app user+kernel CPU across the phase on Linux/Windows,
  excluding child producers and the Python driver. Its sampling envelope includes
  gate/poll bookkeeping. It is `null` on unsupported platforms (currently macOS).
  Memory is OS RSS/working set and, where available, private bytes/PSS; it does
  not claim to measure GPU VRAM. Before/after values capture growth, not peak RSS.
- Stages: `parser` (includes scrollback maintenance), `pty_event`, `snapshot`,
  `layout` (on pane resize), `renderer_update`, `pane_update_shaping`,
  `glyph_shaping` (main pane text shaping), `image_decode_place`, `image_upload`,
  `gpu_frame`, `gpu_submit`, `present`, and `frame`. Nested stage totals overlap;
  do not add them together. Sized/cursor/UI glyph work remains included in the
  enclosing update stages. Upload time includes CPU texture allocation/enqueue,
  not asynchronous GPU transfer completion.
- Reload stages: `config_evaluate` parses/evaluates user source in a fresh VM;
  `config_validate` reads/validates config and registrations; `config_vm_swap`
  replaces active VM/config/registry and drops the old VM; `config_reload` also
  includes file reading, VM/DSL setup and snapshot construction. `config_apply`
  applies the result through the existing app/renderer path. Search/reload
  end-to-end phases include the driver's IPC request and wait for a successful
  present after `search_apply` / `config_apply`.

## Ruby and native keys

`--ruby` runs the two ignored diagnostic tests with locked dependencies and
saves four JSON records. Each discards 100 warmup iterations and measures 1,000
samples. Ruby measurements submit typed requests to the actual named script
thread and wait for completion, including context installation, dispatch,
callback execution and transaction/result collection. Async registration is
prepared outside timing, and a typed successful completion is delivered through
the real callback path; external process launch/runtime is excluded to isolate
mruby completion overhead. Callback failures are errors, not performance samples.

The static-key record measures the native key **resolver** in batches of 1,000,
reports per-key ms, and never invokes Ruby. It is narrower than a script-thread
roundtrip: do not subtract it from Ruby results as if the two were identical
end-to-end actions. Full native key dispatch is separately traceable as
`key_dispatch`. The same stage on a Ruby key ends at enqueue; `ruby_key` records
script-thread service time, while the ignored test includes the channel roundtrip.

For manual key-to-frame investigation use the companion config:

```sh
TOYOTERM_LOG=warn,toyoterm::perf=trace target/release/toyoterm --config examples/performance.rb
```

Create a split, then repeatedly press Ctrl+Shift+F11 (native toggle zoom) and
Ctrl+Shift+F12 (Ruby callback returning the same action). Retain the trace and
environment; it exposes native dispatch, Ruby service, update and frame work.
Manual OS key injection and display latency are not included in `ruby.jsonl`.

The pre-existing isolated terminal measurements remain available:

```sh
cargo test -p toyoterm-terminal --release --locked --test performance -- --ignored --nocapture --test-threads=1
```

## Compare commits and interpret variation

```sh
python3 scripts/performance-baseline.py compare \
  .performance-results/commit-A/results.jsonl \
  .performance-results/commit-B/results.jsonl
```

Comparison prints JSON records with median duration per identical
case/fixture-hash/actual-grid/phase and percentage change. Review metadata, actual grids,
stage distributions, CPU and memory together. Nonmatching fixtures are not
compared; unmatched cases are reported. Missing/failed cases are not evidence of equivalent performance.
Compare `ruby.jsonl` distributions separately. Retain raw samples and look for
repeatable large changes before profiling. A 5–10% difference can be noise;
rerun under controlled conditions. Correctness and readability take precedence
over improving a benchmark number.

Normal CI runs only fixture/result-contract tests and ordinary Rust regression
tests. GUI baseline and ignored measurements are opt-in. There are **no ms or
percentage pass/fail thresholds**. Operational timeouts detect hung/failed runs,
not performance regressions. WezTerm/Ghostty/Alacritty comparisons, if performed
manually, are supplementary evidence and never CI gates.
