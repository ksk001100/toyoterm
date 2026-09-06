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
