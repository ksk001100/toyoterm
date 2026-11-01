# Platform validation

The CI matrix validates pull requests and pushes to `master` on Linux, macOS,
and Windows. Pushes to working branches do not trigger CI, avoiding duplicate
runs when a pull request is open. It builds,
lints, runs all tests, exercises the native PTY and terminal parser, and creates
the platform release archive. Linux additionally starts the complete GUI under
both X11 (Xvfb) and Wayland (headless Weston); macOS and Windows also run the
native GUI smoke test. These are startup checks, not interactive validation.
The extension GUI smoke (`gui-smoke-test --extension-ui`) also requires an actual
presented frame for a Ruby-created Unicode prompt and read-only panel. The native,
X11 and Wayland CI jobs include it. This verifies the script/native/render path;
it does not drive OS keyboard or IME events, or assert screenshot appearance.
The GUI smoke command exits only
after creating the window, renderer, IME context, and initial shell session.
Core behavior has also been exercised on physical Linux, macOS, and Windows
machines. The checklist below is for release-candidate-specific validation.

Before publishing a release candidate, run the following interactive checks on
physical machines. Record the OS version, display scale, keyboard layout, and
result in the release issue.

## Cross-target checks from Windows

When Zig, `llvm-ar`, the Linux/macOS Rust targets and cached Cargo dependencies
are already available, a Windows host can check target-specific Rust code and
compile mruby's target C objects without another OS runtime. The extension audit
records successful full-workspace/all-target checks with rustc 1.99.1 and Zig
0.17.0. This does not link executables, run target tests or replace the physical
checks below. It may require ordinary compiler/filesystem permissions outside a
restricted Windows sandbox.

Run from PowerShell; preserve existing environment values:

```powershell
$crossNames = @(
  'CC_x86_64_unknown_linux_gnu', 'AR_x86_64_unknown_linux_gnu',
  'CC_aarch64_apple_darwin', 'AR_aarch64_apple_darwin',
  'CC_KNOWN_WRAPPER_CUSTOM', 'ZIG_GLOBAL_CACHE_DIR'
)
$savedCross = @{}
foreach ($name in $crossNames) {
  $savedCross[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
try {
  $env:CC_x86_64_unknown_linux_gnu = 'zig cc -target x86_64-linux-gnu'
  $env:AR_x86_64_unknown_linux_gnu = 'llvm-ar'
  $env:CC_aarch64_apple_darwin = 'zig cc -target aarch64-macos'
  $env:AR_aarch64_apple_darwin = 'llvm-ar'
  $env:CC_KNOWN_WRAPPER_CUSTOM = 'zig'
  $env:ZIG_GLOBAL_CACHE_DIR = Join-Path $env:TEMP 'toyoterm-zig-cache'
  cargo check --workspace --all-targets --target x86_64-unknown-linux-gnu --locked --offline
  if ($LASTEXITCODE -ne 0) { throw 'Linux cross-check failed' }
  cargo check --workspace --all-targets --target aarch64-apple-darwin --locked --offline
  if ($LASTEXITCODE -ne 0) { throw 'macOS cross-check failed' }
} finally {
  foreach ($name in $crossNames) {
    [Environment]::SetEnvironmentVariable($name, $savedCross[$name], 'Process')
  }
}
```

`CC_KNOWN_WRAPPER_CUSTOM` lets cc-rs recognize `zig cc` during compiler-family
detection. The target-specific CC/AR variables keep the host compiler unchanged.
No tools, Rust targets or packages are installed by these commands.

### Linux linking without execution

Full-workspace/all-target Linux linking also passed on the Windows host using
the same compiler variables. Run this within the `try` block above, before its
`finally` restores CC/AR/Zig settings:

```powershell
$savedLinuxLinker = $env:CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER
$linuxLinker = Join-Path $env:TEMP 'toyoterm-linux-zig-linker.cmd'
[System.IO.File]::WriteAllText(
  $linuxLinker, "@echo off`r`nzig cc -target x86_64-linux-gnu %*`r`n",
  [System.Text.Encoding]::ASCII
)
try {
  $env:CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER = $linuxLinker
  cargo test --workspace --all-targets --target x86_64-unknown-linux-gnu --no-run --locked --offline
  if ($LASTEXITCODE -ne 0) { throw 'Linux link check failed' }
} finally {
  $env:CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER = $savedLinuxLinker
}
```

This produces Linux executables, including test harnesses, but does not run
them. The generated `toyoterm` was checked as ELF64 x86-64 PIE with interpreter
`/lib64/ld-linux-x86-64.so.2`. It is a development build, not a verified release
archive or evidence of runtime/GUI behavior.

The link check exposed a host/target error in the CLI build script: Windows-hosted
Linux builds had emitted a Windows COFF icon resource, which the Linux linker
rejected. Resource embedding now checks `CARGO_CFG_TARGET_OS` before running;
checking the build script's host OS would not fix cross compilation. Native
Windows resource embedding and x86_64 ConPTY bundle copying remain enabled.

### macOS link prerequisite

The existing Zig compiler was sufficient for macOS ARM64 C compilation and
full-workspace Rust type checking. A script-test `--no-run` final-link attempt
still failed: rustc could not locate a macOS SDK and the target `iconv` library
was unavailable. The check needs a real compatible Apple SDK/sysroot (or a macOS
build host); compilation success alone does not establish successful macOS
linking. No target test execution or GUI result is implied.

## Linux Wayland and X11

See also the [current terminal-extension audit](extension-validation-audit.md)
for the distinction between target compilation, native presentation and actual
desktop interaction evidence.

- Start `toyoterm`, type ASCII and non-ASCII text, and run `printf '\e[31mred\e[0m\n'`.
- Compose one accented character and enter Japanese text through an IME.
- Copy and paste between toyoterm and a native desktop application.
- Set `config.window.opacity = 0.8`, reload, and confirm compositor transparency.
- Split a pane, resize the window, then close the application with shells alive.
- Install from the archive, launch it from the desktop menu and shell, install
  the same release again, then run the packaged uninstaller. Confirm no installed
  files remain and the user configuration is preserved.

Run the list once in a Wayland session and once in an X11 session. CI covers
startup for both display protocols on every push.

## macOS

- Launch `toyoterm.app` and exercise the default shell PTY.
- Verify Command shortcuts, Option-modified input, dead keys, and Japanese IME.
- Copy and paste to TextEdit; check rendering at 1x and Retina scale.
- Repeat pane split, resize, reload, and shutdown checks.
- Start with `config.window.opacity = 0.8` and confirm the desktop is visible
  through the default terminal background. Reload through `1.0`, `0.8`, `0.0`,
  `0.5`, and `1.0`; repeat using `Toyoterm.configure` in the Ruby Console.
  Confirm each change takes effect without stale terminal contents behind the
  transparent background, including after resizing and at Retina scale.
- With JetBrainsMono Nerd Font, open a synthetic TypeScript/React buffer in
  LazyVim with vtsls diagnostics, split it vertically, and scroll both windows.
  Check that diagnostic icons, split separators, and the right-hand text remain
  in their columns, including at Retina scale. Repeat with the Mono variant
  and with diagnostics disabled to distinguish font layout from redraw issues.
- Open the DMG, drag the app to Applications, launch it, replace it with the same
  release once, and remove it. Confirm the `.tar.gz` contains the same version.

## Windows

- Start both PowerShell and `cmd.exe`; verify output, input, resize, and exit.
- From a toyoterm PowerShell pane, run `toyoterm ruby console`. Submit an empty
  line and confirm the REPL remains active, then run `exit` and confirm the
  parent shell accepts input normally.
- Run `python examples/terminal_images.py --protocol kitty` and `bsky_tui`;
  confirm Kitty images render rather than being filtered by ConPTY.
- Verify Ctrl shortcuts, AltGr, dead keys, and a Windows IME.
- Copy and paste to Notepad; check 100%, 150%, and 200% DPI.
- Repeat pane split, reload, and shutdown checks.
- Bind `CTRL+[` / `CTRL+]` to subtract/add `0.1` from `config.window.opacity`.
  Start at `0.9`, press `CTRL+]` once, then `CTRL+[` once; the background must
  become opaque and then transparent again. Repeat starting at `1.0` and via
  Ruby Console updates and config reloads. Windows should retain its transparent
  window and surface alpha mode throughout, without recreating the renderer.
  Repeatedly press beyond both limits, then reverse direction. Confirm the
  background opacity changes on the first reverse press and rendering continues
  across `1.0`, including after a resize.
- Start on an HDR-enabled primary display, then drag the window onto an
  SDR-only secondary display and back. Test equal DPI first (no resize or DPI
  event is required), then mixed DPI, with opacity `0.0`, `0.5`, `0.9`, and
  `1.0`. Compare background transparency, ANSI colors, and text with a fresh
  launch on the destination display; allow for the displays' different physical
  brightness and Windows SDR brightness settings. Repeat with an idle shell,
  active output, and minimize/restore. On the SDR display, change opacity using
  keybindings and confirm the desktop becomes more/less visible, rather than
  only the terminal brightness changing. Verify `backend = Dx12` and
  `alpha_mode = PreMultiplied` in the startup log. Windows uses a DirectComposition
  visual paired with `WS_EX_NOREDIRECTIONBITMAP`; both are required for per-pixel
  transparency. This physical HDR/SDR check is not covered by headless tests.
- Exercise portable zip startup and its PowerShell installer, upgrade, and
  uninstaller. Confirm the user PATH entry and Start Menu shortcut are both
  added and removed.
- Double-click the MSI, launch from Start Menu, upgrade using a newer MSI, and
  uninstall through Installed apps. Confirm PATH and shortcut removal, and
  verify that user configuration remains. Uninstall any PowerShell installation
  before testing the MSI at the same destination.

For RC memory stability, also run the [soak procedure](soak-validation.md),
including short churn, eight-hour idle and physical macOS/Windows sleep/wake
cycles. Retain per-OS memory/GPU time series; headless CI cannot establish this.

Windows PTY code is confined to `crates/toyoterm-pty/src/windows.rs`; the Unix backend remains
behind `cfg(unix)` in `crates/toyoterm-pty/src/lib.rs`. The rest of the application uses `Pty`,
`PtySession`, `PtyCommand`, and `PtySize`, preventing ConPTY details from leaking
into mux, terminal, renderer, or scripting code.
