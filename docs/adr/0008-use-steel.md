# 0008: Replace mruby with Steel Scheme

- Status: Accepted
- Date: 2026-10-31
- Supersedes the language and loader choices in ADR 0002 and ADR 0007.
- Adapts the runtime identity in ADR 0006 while preserving its ownership contract.

## Context

The configuration language is moving to a Lisp family language while retaining
toyoterm's native settings, actions, object snapshots, callbacks, events, commands,
themes, selectors, processes, status widgets and atomic reloads. Ruby source
compatibility is explicitly outside the migration; capability parity is required.

## Decision

Embed `steel-core` 0.8.3 through Cargo, using its normal trusted-code engine and
Scheme module system. Replace the mruby C build, FFI shim and Ruby DSL with a
Rust owner and a Scheme prelude. Configuration is `config.scm`; the persistent
IPC console is `toyoterm steel console`. Register capabilities as procedures
rather than emulating Ruby's object/method syntax.

`SteelRuntime` remains explicitly `!Send + !Sync`. The one named script thread
constructs, invokes, reloads, collects and drops the VM. No Scheme values cross
the request/completion boundary. Native bindings keep bypassing the VM.

Use immutable Scheme maps for application settings, registration generations,
queued commands, themes and widget caches. Retain prior maps as transaction
checkpoints. Reject an invalid candidate VM without replacing the active VM.
Each event handler has its own rollback checkpoint. Scheme variables and external
filesystem/process side effects are not transactional. Task result cells belong
to task handles; completed tasks leave the pending registry.

Use typed Steel calls and data conversion for host snapshots and queries. Do not
compile interpolated source for clipboard values, object metadata, event payloads,
process output, or each native snapshot field. Required modules use ordinary
`require` and `provide`, with declaring-source-relative resolution and explicit
config/user search directories.

## Consequences

Rust builds no longer compile or link mruby; the historical vendored sources and
notices remain untouched. Steel and its transitive licenses follow the existing
Cargo license policy. Settings, examples, live console input and libraries must
be migrated to Scheme. The native crate graph and command/application ordering
remain unchanged. Slow Scheme still cannot block PTY parsing or rendering; there
is no forced callback timeout. Configuration remains trusted local code, without
a sandbox or a security-boundary claim.
