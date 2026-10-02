# Terminal protocol fuzzing

Six cargo-fuzz/libFuzzer targets live in the independent `fuzz/` workspace.
They call the production `toyoterm-terminal` public boundary. Protocol changes
should extend these targets and corpus rather than introduce another framework.

| Target | Coverage |
| --- | --- |
| `raw_terminal` | Arbitrary PTY bytes, VT, UTF-8, control strings and reset |
| `osc` | OSC 0/2, 4, 7, 8, 9, 21, 50, 52, 66, 99, 133, 1337, 21337 |
| `kitty_graphics` | APC G, chunks, zlib, IDs, sizes, base64, deletion, Unicode placeholders |
| `sixel` | DCS q, repeats, palettes, raster attributes, malformed numbers, huge dimensions |
| `iterm_image` | OSC 1337 images/files, multipart, size mismatch, missing FileEnd, metadata |
| `kitty_file_transfer` | OSC 5113, traversal, parent IDs (`pr`), symlinks, chunks, zlib, oversized files |

Every input is fed unchanged both whole and with input-selected PTY chunk sizes.
Protocol targets also frame newline-separated payload records, retaining state
across records. OSC chooses a selector from each record's first byte without
removing it; complete wire seeds always reach their intended selector via the
unchanged path. Snapshots, cells, cursor, image RGBA lengths, colors, modes and
drained events are inspected before and after cancellation/reset and subsequent
ordinary text. Unterminated strings are observed before recovery. Panics are
never caught.

## Filesystem safety and resource limits

OSC 5113 download/upload parsing is enabled, but events are inspected and dropped.
The terminal crate performs no local filesystem operations. Paths, parent IDs
and symlink metadata remain untrusted event data. This tests the parser/state
machine; on-disk containment policy belongs to app tests. Never connect the
harness to app transfer handlers. Future filesystem fuzzing must use a separately
reviewed fixed temporary root and traversal/symlink containment tests.
Kitty graphics' existing direct-transmission requirement rejects local file and
shared-memory transports. No app, GUI, PTY, mruby, clipboard or event consumer is
linked. The architecture checker enforces the fuzz dependency allowlist.

The harness does not truncate or sanitize input, alter production limits or use
`cfg(fuzzing)` bypasses. It uses an 80x24 grid with 64 scrollback lines and existing
limits: 32 MiB graphics payload/decoded data, 4096 image sides, 64 MiB stored
images, and bounded file-transfer sessions/chunks/decoded data. libFuzzer's
10-second per-input timeout and 1024 MiB RSS ceiling detect hangs/allocation
failures; they do not fix parser bugs. The default 1 MiB mutation `max_len` is a
search budget. Use `--max-len 40000000` for campaigns targeting control-string
limits. Committed oversized seeds are replayed in full by ordinary tests.
Review integer overflow, allocation arithmetic, image dimension multiplication,
base64 decoded size, zlib expansion, traversal, unterminated strings, escape
handling and state reset when protocols change.

## Execution

Linux is the reference platform for instrumented CI. From the repository root:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz --version 0.13.2 --locked
python3 scripts/run-fuzz.py build raw_terminal
python3 scripts/run-fuzz.py smoke raw_terminal
python3 scripts/run-fuzz.py run raw_terminal --seconds 3600
cargo test -p toyoterm-terminal --test fuzz_corpus --locked
```

Repeat build/smoke/run with each target name above. `run-fuzz.py` first fetches
dependencies with `cargo fetch --locked`, then invokes cargo-fuzz offline and
checks that `fuzz/Cargo.lock` is unchanged after every invocation. This is needed
because cargo-fuzz 0.13.2 does not accept `--locked`. Its lockfile is separate
from production. The runner uses the dictionary and timeout/RSS limits for
campaigns; smoke mode replays each seed file without mutation.

AddressSanitizer is the default. Consult the
[Rust Fuzz Book](https://rust-fuzz.github.io/book/cargo-fuzz.html) for platform
setup, particularly Windows LLVM requirements. `--sanitizer none` is available
for local platforms lacking ASan; record that limitation. Stable corpus replay
does not replace an instrumented build/campaign.

Local Windows validation for this change built all six ASan targets and passed
stable corpus replay. Instrumented smoke could not start because the installed
ASan runtime DLL did not match the compiler (`STATUS_ENTRYPOINT_NOT_FOUND`);
Linux CI smoke and the one-hour RC campaigns remain to be executed. Use a
matching sanitizer runtime on Windows rather than treating build success as a
completed fuzz run.

CI builds all six targets and replays each committed seed on every push/PR.
Daily scheduled and manual workflows additionally run each target for one hour,
with independent jobs and findings/corpus uploads even on failure. Download
evolved corpora for subsequent campaigns; scheduled jobs start from committed
seeds. Before RC, run at least one hour each for raw, OSC and graphics (Kitty,
Sixel and iTerm), record commit, toolchain, commands, elapsed time and results in
the release issue, and triage crashes, timeouts and OOMs. A workflow definition
alone is not execution evidence.

## Corpus and regression policy

`fuzz/corpus/TARGET/` holds wire streams and payload records derived from terminal
tests and `examples/terminal_images.py`. `python3 scripts/generate-fuzz-seeds.py`
recreates named baseline seeds without deleting regression inputs. The dictionary
provides introducers, terminators and representative parameters. Normal terminal
tests reuse the exact harness and replay every committed seed on stable Rust.

For every crash, timeout or excessive allocation:

1. Preserve the artifact, target, command/limits, toolchain and commit.
2. Reproduce and minimize with `cargo +nightly fuzz tmin TARGET
   fuzz/artifacts/TARGET/crash-HASH`. Prepare with `cargo fetch --locked
   --manifest-path fuzz/Cargo.toml`, use `CARGO_NET_OFFLINE=true`, and verify
   `git diff --exit-code -- fuzz/Cargo.lock` afterwards. Retain the original if
   minimization cannot reproduce a resource failure; use the same resource limits.
3. Fix the owning production parser with checked arithmetic and existing limits.
   Do not add unsafe code, exit on malformed input, remove limits or rewrite the
   parser merely for the fuzzer.
4. Copy minimized bytes to `fuzz/corpus/TARGET/regression-ISSUE` and add a focused
   owning-crate test with a behavioral assertion where feasible. Never discard
   a finding merely because its cause was fixed.
5. Replay the artifact and all seeds, run applicable locked tests/Clippy, then
   repeat the instrumented campaign. Review snapshot changes deliberately.

Keep target names and regression seeds stable across protocol changes. Extend
fixtures and dictionary as needed, preserving production budgets and the
filesystem boundary.
