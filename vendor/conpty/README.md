# Microsoft Windows Console ConPTY

This directory contains the Windows x86_64 runtime files from
`Microsoft.Windows.Console.ConPTY 1.24.260710001`, downloaded from the official
NuGet feed:

`https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/1.24.260710001/microsoft.windows.console.conpty.1.24.260710001.nupkg`

The package is published by the `Microsoft.Terminal` verified NuGet owner and
is licensed under MIT. `conpty.dll` and `OpenConsole.exe` must remain a matched
pair. `conpty-oxide` validates that pair before loading it and falls back to the
system ConPTY if validation fails.

SHA-256:

- `win-x64/conpty.dll`: `39FBA2713E2495117B1591AE8C32A3B904BEA7AA66069CF7815E2844C76D75D8`
- `win-x64/OpenConsole.exe`: `B7FD936C2668B87B9ECF7B3366DC6568AFC1C6F981874CBA3E955A1C35CF8160`

The upstream license is preserved in `LICENSE`.
