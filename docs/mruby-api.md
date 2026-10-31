# Scripting API migration

The embedded mruby runtime has been replaced by Steel. Configuration is now
Scheme in `config.scm`; use `toyoterm steel console` for interactive evaluation.
Ruby source compatibility is intentionally not retained.

The canonical reference is [Steel configuration DSL and API](steel-api.md).
It documents settings, key bindings, native handles, callback contexts, events,
commands, registration handles, selection, themes, process tasks, modules,
errors, and transaction/rollback semantics.
