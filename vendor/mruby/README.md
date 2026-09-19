# Vendored mruby

These files are mruby 4.0.0 amalgamations generated from official commit
`831da26b9021de0369d17b71b5667e2941a1a32d`.

The build uses the host GCC toolchain for the POSIX artifact and Visual C++ for
the Windows artifact. Cargo compiles the generated source directly; it never
runs Ruby, Rake, or the mruby build during a normal toyoterm build.

## Included APIs

The build includes `mruby-error`, `mruby-errno`, and these core gemboxes:

- `stdlib`: collection, object, numeric, range, symbol, fiber, enumerator,
  object-space, kernel, class, and control-flow extensions
- `stdlib-ext`: pack/unpack, formatting, time, struct, data, and random APIs
- `math`: math, rational, complex, and multi-precision integer APIs
- `metaprog`: compiler, eval, binding, proc binding, reflection, and method APIs

Filesystem support uses official `mruby-io` and `mruby-dir` with
`hal-posix-io` / `hal-posix-dir` in `mruby.c`, and `hal-win-io` /
`hal-win-dir` in `mruby-windows.c`. Configuration and plugins can therefore use
ordinary `File` and `Dir` APIs. The Ruby-only `filesystem-ext` gem supplies the
`File.write` and `Dir.glob` class APIs absent from mruby 4.0 by composing those
official primitives; it adds no host filesystem bridge.

Socket, task, sleep, exit, binary, and test gems are intentionally excluded.
`Time.now` remains supplied by `stdlib-ext`; toyoterm's existing host
environment synchronization remains unchanged.

Filesystem access is not sandboxed. Ruby configuration and plugins are trusted
local code and use the same filesystem permissions as the toyoterm process.
File and directory calls are synchronous and can block the script thread on a
network filesystem or a large directory tree.

## Regeneration

Use an mruby checkout at the commit above. Point `MRUBY_CONFIG` at this
repository's `build_config.rb`; it records the toolchain, gemboxes, individual
gems, platform HAL selection, and intentional exclusions.

On Linux, macOS, or another POSIX host:

```sh
export TOYOTERM_ROOT=/path/to/toyoterm
export MRUBY_CONFIG="$TOYOTERM_ROOT/vendor/mruby/build_config.rb"
export TOYOTERM_MRUBY_PLATFORM=posix
rake amalgam
ruby "$TOYOTERM_ROOT/vendor/mruby/postprocess.rb" posix \
  build/host/amalgam/mruby.c build/host/amalgam/mruby.h \
  "$TOYOTERM_ROOT/vendor/mruby/mruby.c" \
  "$TOYOTERM_ROOT/vendor/mruby/mruby.h"
```

From a Visual Studio developer PowerShell on Windows:

```powershell
$env:TOYOTERM_ROOT = "C:\path\to\toyoterm"
$env:MRUBY_CONFIG = "$env:TOYOTERM_ROOT\vendor\mruby\build_config.rb"
$env:TOYOTERM_MRUBY_PLATFORM = "windows"
rake amalgam
ruby "$env:TOYOTERM_ROOT\vendor\mruby\postprocess.rb" windows `
  build\host\amalgam\mruby.c build\host\amalgam\mruby.h `
  "$env:TOYOTERM_ROOT\vendor\mruby\mruby-windows.c" `
  "$env:TOYOTERM_ROOT\vendor\mruby\mruby-windows.h"
```

The postprocessor normalizes generated line endings. For the POSIX artifact, it
restores portable `struct stat` timestamp access after an earlier amalgamated
source undefines the usual compatibility macros. For the Windows artifact, it
also applies the small amalgamation-only include ordering fix required by mruby
4.0's Windows IO and Time sources. The individual upstream source files compile
normally; these fixes are needed only after they are concatenated.

The source is distributed under the MIT license in `LICENSE`.
