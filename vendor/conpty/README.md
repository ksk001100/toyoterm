# Microsoft Windows Console ConPTY

This directory contains the Windows x86_64 runtime files from
`Microsoft.Windows.Console.ConPTY 1.25.260710002-preview`, downloaded from the official
NuGet feed:

`https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/1.25.260710002-preview/microsoft.windows.console.conpty.1.25.260710002-preview.nupkg`

The package is published by the `Microsoft.Terminal` verified NuGet owner and
is signed by Microsoft Corporation and licensed under MIT. This preview is used
because it includes [Microsoft Terminal PR #19535][cursor-resize], which
resynchronizes the console cursor after a resize for callers such as PowerShell.
`conpty.dll` and `OpenConsole.exe` must remain a matched pair. `conpty-oxide`
validates that pair before loading it and falls back to the system ConPTY if
validation fails.

SHA-256:

- `win-x64/conpty.dll`: `E2FE87E2258C4E46FFC5157F727218CC25F34A174902F72EB8A5B49EDD9A6458`
- `win-x64/OpenConsole.exe`: `2525C351AA136D555E5DF9A3C9D6CE9BE43F785E37E3C993B8F23B3F0A53C7FA`

The upstream license is preserved in `LICENSE`.

[cursor-resize]: https://github.com/microsoft/terminal/pull/19535
