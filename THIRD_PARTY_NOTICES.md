# Third-Party Notices

toyoterm includes third-party software. This file must be distributed with
source and binary releases.

## Steel 0.8.3

Project: [Steel](https://github.com/mattwparas/steel)

Cargo dependency: `steel-core`, locked in `Cargo.lock`

License: MIT OR Apache-2.0, as declared by the Rust dependency. Rust dependency
license compliance is checked with cargo-deny; Steel is compiled into toyoterm.

## Microsoft Windows Console ConPTY 1.25.260710002-preview

Project: [Microsoft Terminal](https://github.com/microsoft/terminal)

Vendored form: Windows x86_64 `conpty.dll` and `OpenConsole.exe` from the
`Microsoft.Windows.Console.ConPTY` NuGet package

License: MIT

The original license is preserved at `vendor/conpty/LICENSE`.

## Rust dependencies

Rust dependencies and their exact versions are recorded in `Cargo.lock`.
Their license expressions are checked by `cargo-deny` in CI against
`deny.toml`. Each dependency remains subject to its own license terms.
