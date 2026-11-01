# Loading and composing extension libraries

Extension libraries are trusted Ruby files that use the same public APIs as the
main configuration. They can define terminal commands, keymaps, modes, hooks,
advice, prompts and tasks. Their code runs on the script owner; PTY parsing and
rendering remain native. The canonical behavior and limits are in the
[mruby reference](mruby-api.md).

## Install and load explicitly

Place files you trust next to your chosen configuration, for example:

```text
config.rb
libraries/
  directory_provider.rb
  launch_policy.rb
  launch_observer.rb
```

Load each explicitly from `config.rb`:

```ruby
require_relative 'libraries/directory_provider'
require_relative 'libraries/launch_policy'
require_relative 'libraries/launch_observer'
```

Relative imports resolve from the importing file. Files are loaded once per VM;
successful configuration reload starts a fresh VM and imports them again.
Nothing scans a plugin directory. Keep required dependencies explicit with
ordinary `require_relative`/`require`; if one library needs another's constants,
load that dependency before referencing them. No package registry or dependency
resolver is implied. This follows [ADR 0007](adr/0007-use-ruby-loading-and-direct-registration.md).

Configuration reload evaluates and validates the candidate before replacing
the active runtime. A failing library leaves the previous runtime active.
Runtime loads obey the enclosing callback/command transaction: do not rescue a
load error and assume all language-level or external effects were undone.
Ruby objects, constants and filesystem/process effects are outside host rollback.
Libraries should validate required capabilities before making registrations.

## Publish a hook contract

Give each exported hook a documented name, argument order, result convention
and firing point. Prefix library-specific names to avoid accidental collisions;
use a shared name intentionally when several libraries implement one contract.

| Pattern | API | Contract |
| --- | --- | --- |
| Observation | `run_hook` | All handlers run, producing a frozen result Array |
| Provider | `run_hook_until_success` | First truthy value wins; nil/false declines |
| Policy | `run_hook_until_failure` | Every reached handler must return truthy; nil/false declines |

An empty provider hook returns nil; an empty policy hook returns true. Require
mandatory policy libraries explicitly before exposing the consuming command.

Use `priority:` to express ordering independent of import order. Lower values
run first; ties retain registration order. The default is 0. A late fallback
provider can use 100, while a project provider uses 0 or -10. A descriptive
`label:` identifies a handler in `Toyoterm.hook_handlers`; labels need not be
unique. Inspection lists IDs, priority, labels and source locations in execution
order. Keep handles to remove a particular handler.

All strategies snapshot their ordered handlers before dispatch. Removing or
adding a handler affects the next call. Exceptions stop dispatch and roll back
host-managed effects of that entire call; a normal early stop retains effects
of handlers already run. Arguments/results are ordinary Ruby objects passed by
reference, so mutation of those objects remains visible even after an exception.
Policies express application behavior; every library is trusted code with the
same process permissions and can choose not to use that policy.

The [runnable composition example](../examples/hook_composition.rb) explicitly
loads three independent libraries. Alt+P chooses a directory from providers,
checks launch conditions, queues a terminal split, then notifies observers.
Its observation hook is named `terminal_launch_requested` because it fires
before native PTY creation completes.

The composition regression loads these libraries plus a project-directory
provider in all 24 import orders. It checks that priority selects the project
provider regardless of order, repeated `require` returns false without duplicate
handlers, and removing the project provider/observer falls back to the shell
directory without an observer badge. This establishes the bundled fixture's
contract; it does not infer ordering guarantees for libraries that depend on
each other's constants or external effects.

Each library retains its own handles:

```ruby
Toyoterm.hook_handlers(:terminal_directory)
ExampleLaunchObserver::REGISTRATION.remove
```

## Keep lifecycle and compatibility explicit

Use a Ruby module for a library's constants/state. Keep its registration handles
there and expose explicit enable/disable methods if it needs runtime toggling.
Removing a handle does not reload the file: `require` still returns false for
an already-loaded file. Re-enabling should create fresh registrations explicitly.
Duplicate command/keymap/mode definitions fail; only use `replace: true` for a
command when replacement is intentional. Hook/advice changes during a dispatch
take effect next time.

A mode owns registrations created in its lifecycle callbacks and cleans them
up on disable/close. Owned timers, cooperative-work items, async completions and transient key callbacks
propagate that scope. Ordinary hook callbacks do not acquire ownership merely
because their subscription was created in a mode. Use the documented lifecycle
APIs rather than assuming all resources belong to the importing file.

For a library that does not need a mode, use
[`Toyoterm.resource_scope`](mruby-api.md#resource-scopes). Install its commands,
hooks and keymap activations inside `scope.within`; use `scope.wrap` on later
command/event/hook callbacks to give their descendants the same owner. Timers,
async work, cooperative work and overlays propagate captured scope ownership.
`scope.close` removes the group transactionally, including child scopes and
newly-enabled mode instances. Existing definitions/settings and arbitrary Ruby
or native terminal effects are not undone. Re-enable by creating a fresh scope;
closed scopes cannot be reopened. The [scoped library example](../examples/resource_scopes.rb)
can be disabled/re-enabled without redefining its named keymap.

Names are descriptive rather than unique; retain exact handles or apply a
documented namespacing convention for application-level lookup. Closures wrapped
by a closed scope become no-ops; unwrapped callbacks retain their normal
snapshot/delivery rules. Mode and explicit-scope ownership follow lexical entry
order. A parent lifetime dominates a newly created child scope, including global
children. Library-owned module variables are still ordinary Ruby state and do
not automatically revert when a host transaction rolls back.

Check `Toyoterm.supports?(:hook_strategies)` and other required capabilities;
`Toyoterm.version` and `Toyoterm.api_version` supply version strings for reporting.
The API is pre-release and does not promise compatibility aliases. Pin/test the
toyoterm build and library revisions together; capability presence does not
prove that every library assumption is compatible. Do not depend on `__` helpers
or mutable internal registries. Test the actual combination, including duplicate
names, reversed imports, cleanup, callback failure and failed reload. Native GUI
startup smoke alone does not verify keyboard/IME or interactive extension behavior.
