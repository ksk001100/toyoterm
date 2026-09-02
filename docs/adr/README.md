# Architecture decision records

Architecture decision records (ADRs) capture choices that constrain multiple
crates or are expensive to reverse. They describe the decision and its
consequences; operational details remain in the focused documents linked from
each record.

Accepted records:

- [0001: Use Rust for the native application](0001-use-rust.md)
- [0002: Use mruby for embedded scripting](0002-use-mruby.md)
- [0003: Use wgpu for GPU rendering](0003-use-wgpu.md)
- [0004: Isolate the terminal backend behind an abstraction](0004-terminal-backend-abstraction.md)
- [0005: Normalize control-plane mutations into native commands](0005-command-model.md)
- [0006: Own one mruby runtime on a dedicated script thread](0006-single-script-runtime.md)

New records use the next four-digit number. An accepted record is not edited to
hide a later reversal: add a superseding ADR and link the two records instead.

