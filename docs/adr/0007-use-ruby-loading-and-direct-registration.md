# 0007: Use Ruby loading and direct Toyoterm registration APIs

- Status: Accepted
- Date: 2026-09-20

## Context

The original scripting design exposed `Toyoterm::Plugin.define`, plugin
metadata, and a plugin registry in addition to Ruby's own `require`, lexical
scope, modules, and closures. The wrapper duplicated language mechanisms and
made command, event, key, and theme registration appear to depend on plugin
ownership even though atomic reload is implemented by replacing the complete
configuration VM.

Themes remain a useful terminal-specific concept: they need named lookup,
validation, color override semantics, and transactional registration. Command
and event callbacks likewise still need rollback within a live VM request.

## Decision

Load trusted local Ruby libraries only through `require` and
`require_relative`. Do not scan a plugin directory or provide an implicit
loader. Required sources call `Toyoterm.command`, `Toyoterm.on`,
`Toyoterm.theme`, and `Toyoterm.configure` directly and use ordinary Ruby
scope and modules for helpers and reusable APIs.

Remove the public plugin namespace, definition DSL, metadata, registry, and
plugin-specific command, event, key, and theme registration methods. Keep the
registration lifecycle independent of that removed concept: configuration
reload builds and validates a fresh VM before replacing the active generation,
and live callback/configuration transactions checkpoint and roll back commands,
events, keys, themes, and other mutable runtime state together.

Keep themes as a first-class Toyoterm API. `Toyoterm.theme` registers them,
`Toyoterm.themes` lists them, and `config.theme=` selects them after all
required sources have been evaluated. Explicit `config.colors` values continue
to override selected theme values.

This record refines the scripting use cases described by
[ADR 0002](0002-use-mruby.md) and the runtime-state terminology in
[ADR 0006](0006-single-script-runtime.md); it does not change their mruby or
single-script-thread decisions.

## Consequences

- Ruby files behave as ordinary libraries and have no toyoterm-specific
  metadata or ownership object.
- Registration ownership is the active VM/configuration generation rather than
  a plugin.
- Successful reloads replace all callbacks and themes without accumulation;
  failed reloads preserve the previous VM and its complete registry state.
- Callback and live-configuration failures discard partial registrations,
  including themes.
- Packaging or dependency management, if added later, requires a separate
  design rather than preserving the removed plugin registry.
