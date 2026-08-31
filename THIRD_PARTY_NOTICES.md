# Third-Party Notices

toyoterm includes third-party software. This file must be distributed with
source and binary releases.

## mruby 4.0.0

Project: [mruby](https://github.com/mruby/mruby)

Vendored form: amalgamated `mruby.c` and `mruby.h`

Source revision: `831da26b9021de0369d17b71b5667e2941a1a32d`

License: MIT

```text
Copyright (c) 2010- mruby developers

Permission is hereby granted, free of charge, to any person obtaining a
copy of this software and associated documentation files (the "Software"),
to deal in the Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, sublicense,
and/or sell copies of the Software, and to permit persons to whom the
Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

The original license is also preserved at `vendor/mruby/LICENSE`.

## Rust dependencies

Rust dependencies and their exact versions are recorded in `Cargo.lock`.
Their license expressions are checked by `cargo-deny` in CI against
`deny.toml`. Each dependency remains subject to its own license terms.
