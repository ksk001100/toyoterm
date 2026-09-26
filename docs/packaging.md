# Packaging and installation

toyoterm packages target-named artifacts with the embedded mruby runtime. The version comes
from the Cargo workspace and the target comes from `rustc`; filenames therefore
identify exactly which binary they contain. The project is not published to
crates.io.

## Linux

Extract `toyoterm-VERSION-TARGET.tar.gz` and run:

```sh
./install.sh
```

The default prefix is `~/.local`. The installer writes the executable to
`~/.local/bin`, a desktop entry to `~/.local/share/applications`, a 1024×1024 PNG icon
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

The application bundle is unsigned and not notarized. A DMG downloaded from the
internet can be opened and the app copied to Applications, but macOS may block
the first launch because it cannot verify the developer or check the app for
malicious software. If you trust the downloaded artifact and have verified its
checksum, try opening `toyoterm.app` once, then open **System Settings → Privacy
& Security**, scroll down, select **Open Anyway** for toyoterm, and confirm
**Open**. macOS then remembers the exception for that copy of the app. Managed
Macs may not offer this option. See [Apple's instructions for opening an app
from an unidentified developer](https://support.apple.com/en-us/102445).

This manual approval is part of the unsigned distribution; no paid Apple
Developer account is needed to download or use toyoterm. The SHA-256 checksum
checks the download's integrity but does not provide a developer signature.
Interactive validation is tracked separately in the [platform
checklist](platform-validation.md).

## Windows

Download `toyoterm-VERSION-TARGET.msi` and double-click it for a per-user
installation. It installs to `%LOCALAPPDATA%\Programs\toyoterm`, adds that
directory to the user `PATH`, and creates a Start Menu shortcut. A newer MSI
upgrades the previous MSI installation. Remove toyoterm through Windows
**Installed apps**. Close toyoterm before upgrading or uninstalling. User
configuration is preserved. The Start Menu shortcut opens in the user's home
directory.

The portable `toyoterm-VERSION-TARGET.zip` can be extracted and run in place.
It also retains the PowerShell installer for users who need a custom install
directory or want to opt out of PATH and Start Menu changes. From the extracted
directory, run:

```powershell
powershell -ExecutionPolicy Bypass -File .\Install-Toyoterm.ps1
```

The PowerShell installer uses the same default destination. Use
`-NoPath`, `-NoStartMenu`, or `-InstallDirectory PATH` to change this behavior.
Run the installed `Uninstall-Toyoterm.ps1` to remove the executable, user PATH
entry, shortcut, and installer files. Uninstall a PowerShell installation
before switching to the MSI, since the two installers do not share ownership
of installed files.

The MSI, archive, and installed directory contain `toyoterm.exe` for CLI use and
`toyoterm-gui.exe` as the no-console Start Menu launcher. Both start the same
terminal application; keeping the CLI executable in the console subsystem makes
interactive commands such as `toyoterm ruby console` own their input normally.

The installed directory keeps `conpty.dll` and `OpenConsole.exe` next to both
executables. This matched Microsoft ConPTY bundle preserves private
terminal control strings such as Kitty graphics APC on Windows; the operating
system ConPTY may filter them. Do not copy or update only one file from the
pair.

The MSI and executables are not currently Authenticode-signed. Signing requires a
project-owned code-signing certificate. Interactive validation is tracked
separately in the [platform checklist](platform-validation.md).

All platforms preserve user configuration when uninstalling. Configuration paths
are listed in the [API reference](mruby-api.md#loading-configuration).

## Included documentation

Packages include both READMEs, the `docs/` and `examples/` trees, the README icon,
`AGENTS.md`, the project license, third-party notices, and applicable mruby/ConPTY
licenses. README links to local guides, examples, and the mruby license work from
an extracted package. On macOS the common files are inside
`toyoterm.app/Contents/Resources`.

## Integrity and release automation

`sh scripts/package.sh` performs a locked release build, checks license notices,
assembles the native artifacts, and invokes `scripts/verify-package.sh`. Windows
packaging also needs the .NET SDK to restore the pinned WiX Toolset and build
the MSI. Package verification rejects unsafe archive paths, checks the required
documentation and license payload, runs the packaged binary's `version`
command, and exercises Linux installation and removal. Each artifact receives
a `.sha256` sidecar.

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
