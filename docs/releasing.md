# Releasing toyoterm

toyoterm uses Semantic Versioning. Until 1.0, a minor version may contain API
changes and a patch version contains compatible fixes. The single source of
truth is `[package].version` in `Cargo.toml`; `Cargo.lock` and the macOS bundle
metadata are derived from it.

## Release checklist

1. Choose the version, update `Cargo.toml`, and run `cargo check` to refresh
   `Cargo.lock`.
2. Update both READMEs and user-visible examples when behavior changed.
3. Complete the manual checks in `docs/platform-validation.md` for the release
   candidate and link their results from the release issue.
4. Run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace --all-targets`, and `sh scripts/check-licenses.sh`.
5. Confirm the Linux, macOS, and Windows CI jobs created archives. Inspect each
   archive for its binary, README files, example config, and license notices.
6. Tag the reviewed commit as `vVERSION` and publish the three CI artifacts.
7. Install from each published archive, run `toyoterm version`, then exercise
   upgrade and uninstall once before announcing the release.

## Distribution formats

- Linux: `toyoterm-VERSION-TARGET.tar.gz`, containing a portable binary and docs.
- macOS: `toyoterm-VERSION-TARGET.tar.gz`, containing an unsigned `.app` bundle.
- Windows: `toyoterm-VERSION-TARGET.zip`, containing `toyoterm.exe` and docs.

The initial macOS bundle is unsigned. Users may need to approve it in Privacy &
Security. The Windows archive is portable and does not modify the registry.
Signing, notarization, and an installer can be added without changing the
archive layout.
