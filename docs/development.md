# Development

Read [AGENTS.md](../AGENTS.md) for the complete repository working contract,
[architecture](architecture.md) for dependencies, and [threading](threading.md)
for runtime ownership. Read the [mruby API reference](mruby-api.md) before
changing Ruby-visible behavior.

## Validation

Use locked dependency resolution for verification. Run focused tests while
iterating, then the checks relevant to the change. The full CI-equivalent baseline is:

```sh
cargo fmt --check
python3 scripts/check-crate-architecture.py
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
sh scripts/check-licenses.sh
```

On Windows, use `python` if `python3` is unavailable and a POSIX shell such as
Git Bash for shell scripts. The license script checks notices; the separate
compliance workflow checks Rust dependency licenses with `cargo-deny`.

For example, terminal changes can be checked with:

```sh
cargo test -p toyoterm-terminal --locked
```

## Terminal performance measurements

The terminal crate has opt-in measurements for VT input (plain and styled
Unicode), full-screen snapshots (plain text and detected URLs), and cold
searches over 1,000 lines of terminal output. Run them with an optimized build:

```sh
cargo test -p toyoterm-terminal --test performance --release --locked -- --ignored --nocapture --test-threads=1
```

Each result is the median of five samples after warm-up, reported as time per
operation; input cases also report MiB/s. The fixtures and iteration counts are
fixed so changes can be compared on the same machine. These measurements have
no timing pass/fail threshold and are ignored by ordinary tests. Compare runs
under similar system load; they do not include PTY I/O, GPU rendering, or Ruby.

Native smoke tests:

```sh
cargo run --locked -- pty-demo
cargo run --locked -- screen-demo
cargo run --locked -- gui-smoke-test
```

The GUI check requires a display and GPU or software rendering stack. Linux CI
uses Xvfb for X11 and `scripts/wayland-smoke-test.sh` for Wayland. Report
unavailable checks as untested. See [platform validation](platform-validation.md)
for physical-machine checks.

## Documentation and releases

Keep both READMEs synchronized. Document Ruby changes in [mruby-api.md](mruby-api.md)
and update relevant runnable examples. Review rendering snapshots intentionally;
do not refresh them just to pass tests.

Packaging has requirements beyond the checks above. Follow [packaging](packaging.md),
[platform validation](platform-validation.md), and [releasing](releasing.md).
Generated `target/` and `dist/` files are not source files to edit.
