# Terminal extension validation audit

Audit date: 2026-11-01. This describes the current uncommitted worktree, not a
published release or a successful remote CI run.

The user requested extensibility comparable to Emacs for toyoterm as a terminal
and explicitly excluded editor functionality. The extension contract is the
ability to add, replace, inspect, compose and remove terminal behavior through
trusted Ruby without changing Rust. Editable documents/file editing/undo are not
acceptance requirements. The detailed contract remains in [mruby-api.md](mruby-api.md)
and [the feature matrix](extensibility.md).

## Evidence by requirement

| Requirement | Current implementation and evidence | Verification limit |
| --- | --- | --- |
| Define, discover and compose terminal commands | Named command metadata, palettes, typed arguments and help; [argument regressions](../crates/toyoterm-script/src/command_argument_tests.rs), [help regressions](../crates/toyoterm-script/src/command_help_tests.rs) and real [controller/script-thread flows](../crates/toyoterm-app/src/command_dispatch/ui_controller/extension_flow_tests.rs) | Desktop help/definition navigation outside these fixtures remains unverified |
| Replace/augment built-in behavior without rewriting Rust | Removable before/around/after native action advice; [action regressions](../crates/toyoterm-script/src/action_tests.rs) assert substitution, suppression, rollback and captured targets; [native dispatch tests](../crates/toyoterm-app/src/command_dispatch.rs) use the actual script owner before mux mutation | No claim that extensions can replace arbitrary Rust/C internals |
| Compose keys and terminal modes | [Keymap/mode regressions](../crates/toyoterm-script/src/keymap_tests.rs), [transient regressions](../crates/toyoterm-script/src/transient_keymap_tests.rs), native sequence lookup and immediate retirement/cancellation | OS keyboard layouts/input sequences require desktop testing |
| Publish and inspect library hook contracts | Priorities, source metadata, provider/policy/observation strategies, snapshot order and rollback; [hook regressions](../crates/toyoterm-script/src/hook_tests.rs) include all 24 import permutations of four real libraries, load-once and fallback after removal | The bundled-library matrix cannot prove arbitrary third-party libraries compatible |
| Provide reusable terminal extension UI | Input/history/completion, native stale-query rejection, read-only panels and status sections; [prompt tests](../crates/toyoterm-script/src/prompt_tests.rs), [panel tests](../crates/toyoterm-script/src/panel_tests.rs), native controller flows and the extension GUI smoke | Presented frames do not assert pixel appearance or OS IME behavior |
| Own and remove extension resources predictably | [Resource-scope regressions](../crates/toyoterm-script/src/resource_scope_tests.rs) cover reverse cleanup, nested scopes, atomic failure, mode activation identity, wrapped callbacks, tasks and closed targets | External Ruby/filesystem/process effects deliberately remain outside host rollback |
| Schedule work without moving the VM onto the PTY/render hot path | Async tasks, native monotonic timers, cooperative work and opt-in opcode guards; [native timer integration](../crates/toyoterm-app/src/timers.rs), [cooperative tests](../crates/toyoterm-script/src/cooperative_work_tests.rs), [budget tests](../crates/toyoterm-script/src/instruction_budget_tests.rs) | Guards do not interrupt C/I/O or malicious/noncooperative trusted Ruby; OS suspend/resume remains untested |
| Preserve architecture and threading | Architecture checker reports 10 crates/22 production edges/no cycles; [VM owner](../crates/toyoterm-script/src/runtime.rs) remains non-Send/non-Sync; typed requests/completions, snapshots and native commands remain the boundary | Compile/test evidence does not establish every long-duration desktop workload |
| Exclude editor features | `extension_runtime_excludes_editor_apis_and_preserves_terminal_extension_capabilities` rejects editor capabilities/APIs/context while requiring terminal mechanisms; editor-specific symbols are absent from production extension code | Prompt text editing and terminal VT/screen/GPU buffers remain necessary terminal functionality |
| Remain portable across supported desktops | Windows execution plus offline cross-target checks cover the full workspace and all test targets for Linux x86_64/macOS ARM64; all Linux binaries/tests also linked successfully | macOS final linking and execution on either target remain unverified |
| Keep the public contract reviewable | English/Japanese README summaries, canonical Ruby reference, focused guides and runnable examples; 211 relative file links in changed/new Markdown were checked before adding this audit | Anchor targets and external web links were not included in that file-existence check |

## Executed verification

The full Windows baseline passed after the extension GUI smoke addition:

```text
cargo fmt --check
py -3 scripts/check-crate-architecture.py
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
sh scripts/check-licenses.sh
cargo run --locked -- gui-smoke-test
cargo run --locked -- gui-smoke-test --extension-ui
```

The library-permutation and resource-churn additions passed the complete script suite:
255 unit tests and two public-boundary integration tests; three opt-in tests were
ignored. Their focused Clippy/fmt checks also passed. The latest full app baseline
has 141 passing tests and one ignored performance test. Normal Windows filesystem
permissions were required for the existing hard-link test.

All five opt-in renderer GPU tests passed separately. Extension UI smoke observed
real presented frames for a long Japanese/emoji prompt and read-only panel,
including scope closure between phases. It has a bounded event-loop wait and
rejects early closure; it does not synthesize operating-system input.
[Release-profile measurements](performance-baselines/2026-11-01-extension-runtime.md)
retain separate native-key/Ruby-round-trip/opcode-guard costs and their limits.

## Cross-target checks from the Windows host

Existing tools were used: rustc 1.99.1, Zig 0.17.0 and LLVM `llvm-ar`.
No new execution environment or tool installation was performed. Both targets
were already installed and dependencies were available offline.

| Target | Command | Result |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu` | `cargo check --workspace --all-targets --target x86_64-unknown-linux-gnu --locked --offline` | Passed |
| `aarch64-apple-darwin` | `cargo check --workspace --all-targets --target aarch64-apple-darwin --locked --offline` | Passed |

`toyoterm-script`'s build script also compiled the selected POSIX mruby
amalgamation and C shim, with `MRB_USE_DEBUG_HOOK`, for both targets. Rust crates,
binaries and tests were type-checked under each target's `cfg` branches. This is
not a final executable link, target test execution, packaging validation or GUI
run. See [the reproduction instructions](platform-validation.md#cross-target-checks-from-windows).

A subsequent Linux link check passed with the same existing toolchain:

```text
cargo test --workspace --all-targets --target x86_64-unknown-linux-gnu --no-run --locked --offline
```

It produced all test executables and the native Linux CLI/GUI binaries; the
main binary's ELF header and interpreter were inspected without executing it.
The first full link attempt exposed a CLI build-script host/target bug: a Windows
icon resource was passed to the Linux linker. Guarding resource embedding with
`CARGO_CFG_TARGET_OS == windows` fixed that failure; the same full link command
then passed. This is stronger than type checking, but is still not target test
execution. macOS final linking has not been verified.
The Windows workspace tests and extension GUI smoke passed again after that
build-script fix, and the Windows CLI retained a nonempty PE resource directory.

The equivalent macOS ARM64 script-test link was attempted with the existing
Zig linker wrapper:

```text
cargo test -p toyoterm-script --all-targets --target aarch64-apple-darwin --no-run --locked --offline
```

It failed at final linking: rustc reported no macOS SDK and the linker could not
find the target system library `iconv` (`libiconv.tbd`/dylib/archive). This is an
environment prerequisite missing after successful target compilation, not a
passing link or test. No SDK was installed, no system-library stubs were fabricated,
and no library dependency was removed to make the check appear successful.

## Unproven acceptance and next evidence needed

The full objective remains unproven. No additional Linux/macOS execution
environment is available in this session (confirmed by the user); no saved cloud
environment was found. CI has been updated, but a workflow definition is not a
passing run of this uncommitted worktree.
The macOS link attempt confirms that the existing cross compiler alone does not
supply all target SDK libraries. Linux execution and macOS linking/execution
therefore need target prerequisites outside the currently available toolchain.

Remaining evidence includes macOS executable linking, test execution and extension
GUI smoke on Linux/macOS, physical keyboard/IME candidate placement and focus changes,
pixel-level extension UI review, actual suspend/resume and longer desktop
composition/latency checks. Windows controller geometry, virtual-time timer tests,
GPU primitive tests and presented frames each cover part of that behavior; none
substitutes for these missing observations.

Windows UI automation through the available computer-use skill explicitly
excludes terminal applications. Native application self-tests and manual user
checks remain separate from that unavailable automation route. The audit does
not introduce an editor, a forced VM timeout or a security boundary as a substitute
for missing validation.
