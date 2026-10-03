# Long-running soak and memory stability

Repeat this procedure for v0.2 release candidates. Look for continued growth
with time or iteration after caches warm up. An absolute RSS value is not a
pass/fail rule. Python 3's standard library and existing CLI/IPC/Ruby operations
drive the tests; there is no special GUI execution mode or diagnostic public API.

## Automated short soak

From the repository root, using a new results directory:

```sh
python3 scripts/test-soak-test.py
python3 scripts/soak-test.py headless --output .soak-results/short
```

On Windows use `python` or its absolute executable path instead of `python3`.
The headless command runs locked Cargo tests serially with `soak_` and
`--ignored --nocapture`. Add `--release` for an optimized build. Compilation
time is additional; the workloads typically take seconds to minutes. Normal
workspace tests do not execute them. CI may explicitly run this short command;
never schedule the eight-hour procedure in normal test CI.

| Scenario | Work and checks |
| --- | --- |
| Pane/PTY | 100 splits, native child launches, bounded incremental reads, snapshots, child waits, reader joins and closes; mux handles return to baseline each cycle |
| Output/scrollback | 100,000 lines, scroll, search and selection; history stays at 1,000 lines and viewport snapshots at 24 rows |
| Images | 1,000 unique Kitty transmit/place/delete and Sixel/iTerm erase/scroll cycles return images, bytes and placements to zero; transmissions without deletion exercise the 128-image storage limit, and large placements the 64 MiB conservative byte budget |
| Config | 1,000 file reloads including fonts, wallpaper PNG, colors, bar closures and command/event registrations; post-GC objects and arena remain bounded |
| mruby/async | 2,000 event callbacks, completed tasks and cancellations after request drain; cancelled callbacks cannot run, registry returns to zero and post-GC objects/arena remain bounded |

Each test retains its subsystem instance across iterations. Headless async
drives the host completion boundary without external processes. These tests do
not measure GPU residency, GUI ProcessRuntime cleanup or GUI event-loop behavior.
Their structural bounds are not a process-memory verdict; use the GUI procedure
for those paths.

## Real GUI soak

Build once and measure the executable directly, excluding Cargo/build memory:

```sh
cargo build --release --locked -p toyoterm-cli --bin toyoterm
python3 scripts/soak-test.py gui --executable target/release/toyoterm \
  --output .soak-results/gui-rc
```

Windows: use `target/release/toyoterm.exe`, on one line or with PowerShell
backticks. A display and working native GPU stack are required. The runner
starts an isolated named instance with its own IPC directory and console
history, uses `scripts/soak-config.rb`, prints the actual GUI PID and closes
its GUI on completion or interruption. The fixture uses `/bin/sh` or `cmd.exe`
to avoid user shell profiles. It fails on IPC/Ruby errors, timeouts, abnormal
GUI exit, sampling errors, ERROR-level GUI log records or retained mux objects/stale IDs. `FAILED.txt`
distinguishes failure from `COMPLETED.txt`; completion means workloads ran,
not that memory stability was established.

Default work: 30 seconds idle, 30 seconds paced output, 100,000 lines with
search/selection, 100 pane cycles, 100 tab cycles, 100 workspace shell-exit
cycles, 100 mixed Kitty/Sixel/iTerm cycles, 300 real completed/cancelled async
processes, 1,000 reloads, then 60 seconds recovery idle. Workspace cleanup uses
shell exit because Ruby commands cannot close the last window/pane. Creation
and subsequent inspection use separate requests: immutable callback snapshots
cannot expose newly created objects within the creating callback. Saved panes
must become invalid after cleanup, and hierarchy counts must return to baseline.

This larger GUI workload can take substantially longer than the short headless
soak, especially on Windows. Validate the harness first by adding:

```text
--iterations 2 --reloads 3 --async-iterations 2 --idle-seconds 2
--output-seconds 2 --recovery-seconds 3 --interval 1 --warmup 1
```

That reduced run is a harness check, not RC churn coverage. Use defaults for RC;
increase `--iterations 1000` and `--async-iterations 1000` when investigating
iteration-dependent growth. Never reuse an existing results directory.

Inside a toyoterm pane, these producers provide bounded work instead of `yes`:

```sh
python3 scripts/soak-test.py fixture --lines 100000 --delay 0
python3 scripts/soak-test.py fixture --lines 1000000000 --seconds 300 --delay 0.01
python3 scripts/soak-test.py fixture --kind images --iterations 1000 --delay 0.1
```

Output is written incrementally in batches of at most 1,000 lines. Continuous
output has both a deadline and line cap. `--hold SECONDS` keeps a producer alive
after `SOAK_FIXTURE_DONE` so its output can be inspected before explicit closure
of a live child. Image fixtures pause for uploads, use unique Kitty IDs, and
erase/scroll legacy placements away before re-display. Confirm image visibility
manually too: OS scheduling can skip an intermediate frame. Headless parsing
does not prove a GPU texture was rendered.

## Measurements and interpretation

Results include `metadata.json` (commit, dirty state, OS, Python, argv, UTC
start, fixture/harness/binary hashes and executable version), `journal.jsonl`
(phase, iteration milestones, elapsed time, UTC),
`memory.csv` (elapsed time, UTC, phase, separate byte-valued metrics), `gui.log`
and `summary.json`. The default sample interval is five seconds. The sampler
reads the GUI PID, excluding CLI, Cargo and child shell/async process memory;
account for children separately when investigating system-wide growth. Results
under `.soak-results/` are ignored by Git.

The summary reports regression in MiB/minute, early/late third means and min/max
for each available metric. The whole-run report excludes the first 30 seconds
(`--warmup`); phase reports retain their samples. Fewer than six samples yield
**insufficient evidence**, never stability. Mixed workloads can inflate the
whole-run slope as caches are exercised. Compare equal-work iteration windows
and recovery idle, and correlate CSV times with iteration milestones. Inspect
raw CSV for jumps and oscillation; regression alone can conceal later freeing.
Milestone samples also produce MiB per 100 iterations in the summary's
`iterations` reports, independently of machine speed. These require six samples
too; compare warmed iteration windows rather than interpreting initial growth
as a leak.

`TOYOTERM_LOG=warn,toyoterm::soak=trace` enables internal instrumentation:

| Trace | Meaning |
| --- | --- |
| Native resources | panes, tabs, workspaces, ProcessRuntime entries, PTY handles, queued script requests, cancellation IDs awaiting completion |
| Script resources | mruby live objects, GC arena and Ruby async task/callback registry sizes after a script request |
| Terminal graphics | stored Kitty image count/bytes, placement count/bytes and virtual placements after input; placement bytes conservatively count shared pixels again |
| Renderer | cached panes, image entries and uploaded textures after pane update; first-frame uploads occur later, so texture counts can lag one frame |
| PTY output | bytes/capacity on drain and the 1 MiB pending-output limit |

Traces are event driven; the soak instrumentation does not schedule Ruby work.
Idle records come from ordinary bar/script requests and PTY input. Image
records are per terminal input batch; never add them
across time. To capture a quiet post-churn script snapshot, evaluate `GC.start`
in the isolated instance's console. Do not force GC throughout a production-like
GUI run. Headless tests force GC to distinguish reachable objects from garbage.
PTY handle count is not reader-thread count: use OS threads and the headless
reader-join test. GPU counters describe application references, not driver
residency or swapchains; glyph/font/driver caches are not fully inventoried.
RSS/private bytes do not replace GPU or physical footprint measurements.
Record trace settings/overhead and repeat suspicious growth with normal logging.

Accept a plateau when warmed equal-work windows stop rising, resources return
to expected bounds, and recovery idle stabilizes near the warmed baseline.
Initial font/glyph, wallpaper, pipeline and allocator caches are allowed. If
successive windows keep rising, rerun longer on the same machine with at least
three comparable windows. Never use a universal 400 MB threshold. Record
numeric deltas/slopes, elapsed time/iterations, retained counts and the exact
reproducer for every finding.

## Manual idle: one to eight hours

Run a release GUI with one idle shell, fixed size/display/scale/opacity and no
automatic OS sleep:

```sh
python3 scripts/soak-test.py gui --executable target/release/toyoterm \
  --scenario idle --idle-seconds 28800 --interval 60 --warmup 1800 \
  --output .soak-results/idle-8h-rc
```

For one hour use `--idle-seconds 3600`. Record startup, 5 min, 30 min, 1 h, 4 h
and 8 h (0, 300, 1800, 3600, 14400, 28800 seconds), plus the OS/GPU snapshots
below. Use the nearest CSV sample and its actual UTC for each checkpoint. Keep
the shell idle; run a separate daily-driver session using shell/Neovim.
Use the journal's PID and UTC to align external measurements.

## OS memory and GPU collection

Keep raw output and UTC timestamps at each checkpoint. Some commands require
privileges; record denied/unavailable metrics as unavailable, not zero. Check
PID identity before collecting; PID reuse after GUI exit invalidates a sample.

**Linux**:

```sh
cat /proc/GUI_PID/status
cat /proc/GUI_PID/smaps_rollup
ls /proc/GUI_PID/task | wc -l
```

The harness records VmRSS, PSS, private clean+dirty bytes and Threads. Inspect
DRM memory counters in `/proc/GUI_PID/fdinfo/*` where exposed, recording the
driver/counter. Vendor GPU tools may show device totals; label their scope and
avoid attributing other apps to toyoterm.

**macOS**:

```sh
ps -o pid,rss,vsz -p GUI_PID
footprint -p GUI_PID
vmmap -summary GUI_PID
```

The harness converts `ps` RSS from KiB to bytes. Save physical footprint and
graphics/IOKit summaries separately. Use Activity Monitor and Instruments/Metal
tools when summaries are insufficient. Record macOS, hardware and Metal backend.
Physical footprint, RSS and virtual size are different metrics.

**Windows (PowerShell)**:

```powershell
$soakPid = 12345 # actual GUI PID; do not overwrite PowerShell's automatic $PID
$soakProcess = Get-Process -Id $soakPid
$soakProcess | Select-Object Id, WorkingSet64, PrivateMemorySize64, HandleCount
$soakProcess.Threads.Count
Get-Counter -ListSet '*GPU*'
```

The harness uses GetProcessMemoryInfo for working set and private bytes. Enable
dedicated/shared GPU memory in Task Manager's Details view for that PID. Where
available, collect GPU Process Memory's dedicated/shared usage and total
committed counters for instances containing `pid_<GUI_PID>_`. Names are
localized: use paths returned by `Get-Counter -ListSet` on that machine. Save
all adapters/instances for the PID rather than only the first match. Record
DX12 backend, driver, GPU, scale/HDR/display state and handle/thread counts.

## Manual sleep/wake (macOS and Windows required)

Use the isolated idle command with a sufficiently long duration. Exercise real
OS sleep, not window minimization; do not automate sleep in CI. Repeat at least
three cycles:

1. Start release GUI; collect startup and warmed memory/GPU baseline after at
   least five minutes.
2. Use shell/Neovim and terminal images; record workload and UTC.
3. Sleep the OS for **at least ten minutes**.
4. Resume; confirm render/resize, keyboard input, new PTY output, Neovim redraw
   and images work.
5. Capture memory/GPU immediately, two minutes and five minutes after resume;
   compare pre-sleep baseline and previous cycles.
6. Inspect repeated device/surface recreation warnings, duplicated textures or
   swapchains, stopped PTYs or persistent footprint jumps. Record backend/driver
   messages and numerical deltas for each cycle.

Keep a UTC sleep/resume journal. Monotonic clocks treat suspend differently
across OSes; use `utc_s` for actual sleep duration. Missing samples during sleep
are expected. One recovery allocation that plateaus differs from a step added
on every resume. Zero cached textures cannot rule out driver/swapchain duplication.

## RC evidence and findings

### Streaming wallpaper uploads (2026-10-03 JST)

After lossless CPU storage was merged, wallpaper uploads were changed to expand
row chunks directly into one mapped `MAP_WRITE | COPY_SRC` transfer buffer,
followed by one queued buffer-to-texture copy. The GPU texture keeps its original
resolution, color, and alpha. The renderer no longer allocates a full decoded
CPU `Vec`; scratch space is at most 256 KiB or one row, whichever is larger.
For the personal 3344x1882 image, that decoded allocation changes from 24.01 MiB
to 254,144 bytes (about 0.24 MiB). The mapped transfer buffer is still image-sized
and lives until the GPU finishes its copy. No synchronous GPU wait was added.

Two fresh 30-second processes per version used the same Windows 11 / NVIDIA
RTX 4070 Ti SUPER / DX12 personal configuration and isolated `cmd.exe /d` as
below. Idle values are medians of the final ten one-second samples. The first
control overlapped a build before the final ten samples; the other runs did
not overlap builds or tests.

| Metric (MiB) | Full CPU expansion | Direct mapped upload |
| --- | ---: | ---: |
| Idle CPU private working set | 96.78 / 90.67 | 92.09 / 99.48 |
| Idle CPU total working set | 145.12 / 136.84 | 137.50 / 144.90 |
| Idle CPU private bytes (commitment) | 187.46 / 178.52 | 180.89 / 190.08 |
| Sampled peak CPU private working set | 184.49 / 153.52 | 146.65 / 148.83 |
| GPU dedicated / shared usage | 56.79 / 32.57 | 56.79 / 33.57 |

These runs demonstrate no consistent idle working-set reduction. Removing the
decoded allocation bounds upload scratch memory; it does not promise that
the allocator or driver returns an equivalent number of resident pages. The
one-second sampled peaks are not exact upload peaks. Initial `image_upload`
trace stages were 37.68 / 38.39 ms before and 32.63 / 32.15 ms after; these are
individual observations, not latency percentiles. A trial using many separate
`queue.write_texture` calls increased DX12 shared GPU memory and was discarded.

The personal-config check switched horizontal/vertical images, changed window
opacity, cleared the wallpaper, and reloaded the config without ERROR records.
CPU tests cover raw/compressed chunks, offsets, short final chunks, corrupt and
wrong-length data. GPU composition reads every row of a 257x257 image, exercising
row padding and a short final decoded chunk, as well as the tiny raw-image case.
The normal GUI harness completed eight pane/tab/workspace/image cycles, four
async cycles, and ten reloads with no sampling errors or ERROR records. Workspace
all-target tests, all four opt-in GPU tests, workspace Clippy with warnings denied,
formatting, architecture, and license checks passed. The short GUI run does not
establish a memory plateau.
Physical device loss, sleep/wake, long idle, and macOS/Linux GUI validation remain
outstanding.

### Lossless wallpaper CPU storage comparison (2026-10-03 JST)

With the descriptor budget change already applied, a second release-build
comparison used the same Windows 11 / RTX 4070 Ti SUPER / DX12 environment,
960x600 window, personal settings, and isolated `cmd.exe /d` sessions described
below. Two fresh processes per version each idled for 30 seconds; values are
the median of the final ten one-second samples, with no overlapping builds or
tests. Raw pixel storage was replaced by losslessly compressed `ImagePixels`
only when an image is at least 64 KiB and compression saves at least 12.5%.
Small/incompressible images keep raw storage. No Ruby settings changed.

| Metric (MiB) | Raw wallpaper pixels, two runs | Lossless storage, two runs |
| --- | ---: | ---: |
| CPU wallpaper storage | 24.01 | 2.38 |
| CPU private working set | 127.93 / 127.10 | 104.02 / 107.09 |
| CPU total working set | 173.14 / 172.07 | 151.95 / 152.33 |
| CPU private bytes (commitment) | 223.06 / 221.06 | 195.87 / 197.63 |
| GPU dedicated / shared usage | 56.79 / 32.57 | 56.79 / 32.57 |

The uploaded 3344x1882 image still contains exactly 25,173,632 RGBA bytes;
CPU storage contains 2,498,129 zlib bytes. The renderer expands it once for a
new GPU upload, drops that temporary allocation immediately after the queue
copies the pixels, and reuses the texture on ordinary redraws. A trace-enabled
comparison recorded an initial `image_upload` stage of 8.92 ms before and
36.52 ms after; subsequent stages were below 0.001 ms. These are single startup
observations, not frame-time percentiles. Compression adds work on the config
loader and expansion adds work on uploads/recovery; it does not reduce GPU
texture resolution or memory. Exact RGBA/alpha round trips, incompressible
fallback, bounded corrupt-data errors, and raw/compressed GPU composition are
covered by tests. The personal configuration still exceeded 100 MiB of private
working set in these samples; no long-idle, sleep/wake, or other-platform
footprint claim follows from this comparison.

A separate personal-config GUI check switched to the 1704x3692 wallpaper,
changed opacity from 0.95 to 0.5 and 1.0, cleared the image, and reloaded the
original config. All three expected GPU uploads were logged, with no ERROR
records and clean shutdown. Its pre-action private working set was 114.21 MiB,
showing process-to-process variation beyond the two comparison runs; wallpaper
storage savings alone do not predict the exact total resident footprint.
The normal GUI harness also completed eight pane/tab/workspace/image cycles,
four async cycles and ten reloads without sampling errors or ERROR records.
Workspace tests, the four opt-in GPU tests, Clippy, formatting, architecture,
and license checks passed; physical device-loss/sleep-wake and macOS/Linux GUI
validation remain outstanding.

### Windows startup allocation comparison (2026-10-03 JST)

The DX12 renderer requests 65,536 live non-sampler bindings instead of wgpu's
default 1,000,000. DX12 allocates the shader-visible descriptor heap at device
creation, so reducing this budget reduces fixed CPU/GPU commitments. Other
device limits, the memory-usage allocation policy, and other platforms are
unchanged. The startup renderer log includes the adapter name to identify the
driver used for comparisons.

A release-build comparison on Windows 11 with an NVIDIA GeForce RTX 4070 Ti
SUPER used one 960x600 window, `cmd.exe /d`, and isolated IPC instances. The
personal configuration retained 10,000 history lines, JetBrainsMono/Hack Nerd
Fonts, 0.95 window opacity, status widgets, and a 3344x1882 RGBA wallpaper
(24.01 MiB of CPU pixels plus its GPU texture). Each fresh process idled for
30 seconds; CPU values below are the median of the final ten one-second
samples. GPU values are a separate checkpoint after sampling, summed across
all PID-matching adapter instances. Build/test jobs did not overlap these
final comparisons; child-process memory is excluded.

| Metric (MiB) | 1,000,000 descriptors | 65,536 descriptors |
| --- | ---: | ---: |
| CPU working set, including shared pages | 161.32 | 163.98 |
| CPU private bytes (commitment) | 232.91 | 208.72 |
| GPU dedicated usage | 85.41 | 56.79 |
| GPU shared usage | 32.57 | 32.57 |

This establishes lower commitments, not lower resident CPU memory: the working
set did not decrease in this comparison. Windows `QueryWorkingSet` counted
117.31 MiB of non-shared resident pages after the change; do not confuse that
private working set with either total working set or private bytes. Under the
same sampling procedure, removing only the wallpaper yielded 151.25 MiB total
working set / 106.16 MiB private working set / 171.39 MiB private bytes. A
minimal configuration with the same history budget yielded 139.67 / 76.69 /
145.52 MiB, respectively. These measurements do not establish a sub-100 MiB
footprint with the personal configuration, nor an eight-hour plateau, other
GPU/OS results, HDR behavior, or sleep/wake stability. GPU and CPU counters can
overlap; do not add them into a single memory total.

The changed release renderer also completed a 42-second GUI harness check:
eight pane/tab/workspace/image cycles, four async cycles, and ten reloads, with
no sampling errors or ERROR-level log records. All four opt-in GPU composition
tests passed. This is regression coverage for the reduced allocation budget,
not the default RC workload or long-term stability evidence.

Copy this into the RC issue and attach raw result directories:

```text
RC/version/commit and dirty state:
Date/operator; OS/hardware/GPU/driver/backend:
Profile, command, fixture revision, fonts/display/scale:
Headless result and durations:
GUI iterations/reloads/async counts and result:
Idle startup/5m/30m/1h/4h/8h (RSS/private/PSS/footprint/GPU with units):
Warmup, early/late means, slopes, recovery baseline:
Sleep cycles/durations and immediate/2m/5m resume samples:
Resource counts after churn/idle:
Unavailable metrics/platforms and remaining checks:
Findings (numbers, exact reproducer, plateau or continued growth):
Decision and links to logs/issues:
```

Initial Windows development evidence (unoptimized build): 100 headless PTY
cycles returned to four native handles with all readers joined; 1,000 reloads
and 2,000 completed/cancelled task cycles stayed at 1,808 live mruby objects and
arena index 1 after GC, registry zero. Image deletion returned stored images,
placements and bytes to zero; 100,000 lines retained 1,000 history lines. These
structural measurements do not rule out long-run retention. Previously reported
macOS 400–500 MB physical footprint and post-sleep growth remain historical
observations without a reproduced trend here. Rerun this procedure and attach
numeric time-series evidence before classifying them.

On 2026-10-03 (JST), a Windows 11 development GUI run completed the default
100 pane/tab/workspace/image cycles, 300 completed/cancelled async cycles and
1,000 reloads in approximately 12 minutes, including recovery idle. Using the
debug executable and two-second samples, working set during reload was
342.68–342.93 MiB (slope +0.015 MiB/min); private bytes were 340.23–340.32 MiB
(+0.009 MiB/min). Recovery-idle working set was 342.91–342.93 MiB
(-0.011 MiB/min). Final pane/tab/workspace/ProcessRuntime/PTY/renderer-pane
counts were one, pending requests/async tasks/cancellation IDs zero, and renderer
images/textures zero. This checks workload execution and short trends, not
release-build GPU residency or an eight-hour plateau. Build/test jobs overlapped
the first part of the run; repeat in controlled RC conditions.

The initial full Windows GUI development run also exposed one non-memory
anomaly during workspace churn: `resize pane PTY failed` / `failed to resize
pseudoconsole` after a shell exit. Reproducer: the GUI command above using the
debug executable, default 100 workspace cycles and `--interval 2`. One ERROR
was observed; hierarchy/resource counts subsequently returned to baseline.
This is not evidence of a memory leak and requires separate ConPTY lifecycle
investigation. The harness now reports ERROR log records and fails such runs
instead of treating workload completion as clean validation.
