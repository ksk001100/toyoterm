# Documentation

[English README](../README.md) · [日本語README](../README.ja.md)

## Using toyoterm

| Guide | Contents |
| --- | --- |
| [Installation](packaging.md) | Artifacts, install, upgrade, uninstall, and checksums |
| [Usage](usage.md) | Controls, CLI, logs, configuration recovery, and rendering |
| [mruby API](mruby-api.md) | Canonical configuration and plugin reference, defaults, validation, and execution semantics |
| [Shell integration](shell-integration.md) | Shell setup, cwd reporting, and command lifecycle |
| [OSC support](osc-support.md) | Supported OSC sequences, intentional restrictions, and prioritized backlog |
| [Terminal images](image-protocols.md) | Sixel, Kitty, OSC 1337, limits, and a runnable chart |
| [URL opening](url-security.md) | Modifier-click behavior and allowed schemes |
| [Local IPC](ipc.md) | Instance selection, transport, protocol, and authentication |

Runnable configurations live in [examples](../examples). The
[default configuration](../examples/default_config.rb) provides optional bindings;
toyoterm itself has no built-in GUI key bindings.

## Developing toyoterm

| Guide | Contents |
| --- | --- |
| [Development](development.md) | Repository workflow and validation commands |
| [Crate architecture](architecture.md) | Responsibilities and dependency rules |
| [Threading](threading.md) | Ownership, request ordering, and callback budgets |
| [Architecture decisions](adr/README.md) | Accepted decisions and rationale |
| [Platform validation](platform-validation.md) | CI coverage and physical-machine checks |
| [Releasing](releasing.md) | Versioning and release checklist |

Keep the READMEs as synchronized English/Japanese introductions, detailed Ruby
behavior in the API reference, operational instructions in the relevant guide,
and historical design decisions in ADRs. Update the owning guide and affected
README summaries and examples together when behavior changes.
