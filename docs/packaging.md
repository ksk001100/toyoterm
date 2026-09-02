# Packaging and installation

toyoterm releases self-contained, target-named artifacts. The version comes
from the Cargo workspace and the target comes from `rustc`; filenames therefore
identify exactly which binary they contain. The project is not published to
crates.io.

## Linux

Extract `toyoterm-VERSION-TARGET.tar.gz` and run:

```sh
./install.sh
```

The default prefix is `~/.local`. The installer writes the executable to
`~/.local/bin`, a desktop entry to `~/.local/share/applications`, a scalable icon
to the hicolor icon tree, and an uninstaller to
`~/.local/lib/toyoterm/uninstall.sh`. It does not edit shell startup files.
Add `~/.local/bin` to `PATH` when the directory is not already present.

Use an absolute custom prefix when needed:

```sh
./install.sh --prefix /absolute/prefix
/absolute/prefix/lib/toyoterm/uninstall.sh --prefix /absolute/prefix
```

Installing a newer archive to the same prefix atomically replaces the installed
Linux executable.
The executable in the extracted directory remains portable and may instead be
run without installation. Linux binaries are dynamically linked against the
baseline system libraries of the release runner; they are not fully static.

## macOS

Open the target DMG and drag `toyoterm.app` to Applications. A `.tar.gz`
containing the same application bundle is provided for scripted or portable
use. Quit toyoterm before replacing an existing bundle. Uninstall by removing
`toyoterm.app`.

The application bundle currently has no Apple Developer signature or
notarization ticket. macOS may require explicit approval in Privacy & Security.
Signing and notarization require project-owned Apple credentials and are the
remaining external release prerequisite.

## Windows

Extract `toyoterm-VERSION-TARGET.zip`. It can be run in place without modifying
the registry. For a per-user installation, run PowerShell from the extracted
directory:

```powershell
powershell -ExecutionPolicy Bypass -File .\Install-Toyoterm.ps1
```

The default destination is `%LOCALAPPDATA%\Programs\toyoterm`. The installer
adds that directory to the user `PATH` and creates a Start Menu shortcut. Use
`-NoPath`, `-NoStartMenu`, or `-InstallDirectory PATH` to change this behavior.
Run the installed `Uninstall-Toyoterm.ps1` to remove the executable, user PATH
entry, shortcut, and installer files. The zip remains usable as a portable
fallback.

The executable is not currently Authenticode-signed. Signing requires a
project-owned code-signing certificate and is the remaining external Windows
release prerequisite.

## Integrity and release automation

`sh scripts/package.sh` performs a locked release build, checks license notices,
assembles the native artifact, and invokes `scripts/verify-package.sh`. The
verification rejects unsafe archive paths, checks the required documentation
and license payload, runs the packaged binary's `version` command, and exercises
Linux installation and removal. Each artifact receives a `.sha256` sidecar.

Pushing a `vVERSION` tag starts `.github/workflows/release.yml`. It refuses a tag
that differs from the Cargo version, then formats, lints, tests, packages, and
verifies the following native targets:

- Linux x86_64 and aarch64
- macOS x86_64 and aarch64
- Windows x86_64

After every target succeeds, the workflow verifies all sidecars, writes a
combined `SHA256SUMS`, and creates or repairs the GitHub Release. The manual
workflow dispatch accepts an existing tag so an interrupted publication can be
retried without creating a new tag.

Verify a downloaded artifact from the directory containing it with one of:

```sh
sha256sum -c toyoterm-VERSION-TARGET.tar.gz.sha256
shasum -a 256 -c toyoterm-VERSION-TARGET.tar.gz.sha256
```

Windows users can compare `Get-FileHash -Algorithm SHA256` with `SHA256SUMS`.
Checksums detect damaged or replaced downloads; they do not replace platform
code signing.
