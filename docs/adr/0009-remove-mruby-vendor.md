# 0009: Remove unused mruby vendor artifacts

- Status: Accepted
- Date: 2026-10-31
- Supersedes the vendor-retention consequence of [ADR 0008](0008-use-steel.md).

## Context

Steel has replaced mruby, and production builds no longer use the vendored
runtime. Keeping unused sources and requiring their license files in packages
leaves obsolete runtime artifacts in the source tree and distribution contract.
Git history preserves the previous runtime and its notices.

## Decision

Remove `vendor/mruby/`, its packaging copies and MSI file entry, and the obsolete
license-check and package-verification requirements. Remove its current
distribution notice and update both READMEs. Preserve historical ADRs and
validation records; they describe the runtime used at that time.

## Consequences

Steel remains a locked Cargo dependency subject to cargo-deny license checks.
ConPTY vendor files and their license requirements remain unchanged. This cleanup
does not alter runtime behavior or the internal crate graph. Package creation
and release validation still follow the normal packaging and release procedures.
