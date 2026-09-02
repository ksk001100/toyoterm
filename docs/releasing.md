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
5. Run `sh scripts/package.sh` locally. It must verify the archive and create its
   SHA-256 sidecar.
6. Tag the reviewed commit as `vVERSION` and push the tag. The Release workflow
   rejects a tag that does not exactly match the Cargo version.
7. Confirm all five native package jobs succeed. The workflow publishes Linux
   x86_64/aarch64, macOS x86_64/aarch64, and Windows x86_64 artifacts only after
   their format, lint, test, package, and executable checks pass.
8. Download one published artifact for each OS, verify it against `SHA256SUMS`,
   install it, run `toyoterm version`, and exercise upgrade and uninstall before
   announcing the release. Use the manual Release workflow with the same tag to
   repair an interrupted asset upload.

## Distribution formats

- Linux: `toyoterm-VERSION-TARGET.tar.gz`, containing a portable binary, a
  per-user installer/uninstaller, desktop entry, icon, and docs.
- macOS: `toyoterm-VERSION-TARGET.dmg` and `.tar.gz`, each containing the same
  unsigned `.app` bundle with application metadata and icon.
- Windows: `toyoterm-VERSION-TARGET.zip`, containing portable `toyoterm.exe`,
  optional per-user install/uninstall scripts, and docs.
- Integrity: a `.sha256` sidecar for every artifact and one combined
  `SHA256SUMS` file on the GitHub Release.

The macOS bundle is unsigned and not notarized; users may need to approve it in
Privacy & Security. The Windows executable is not Authenticode-signed. Those
steps require project-owned signing identities and are not replaced by SHA-256
checksums. The Windows zip remains portable and modifies user state only when
the included installer is explicitly run. See `docs/packaging.md` for layouts,
commands, and verification details.
