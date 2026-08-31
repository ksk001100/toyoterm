# Vendored mruby

These files are the mruby 4.0.0 amalgamation generated from commit
`831da26b9021de0369d17b71b5667e2941a1a32d` of the official mruby repository.

The build includes these core gems:

- mruby-compiler
- mruby-error
- mruby-eval
- mruby-enum-ext
- mruby-array-ext
- mruby-hash-ext
- mruby-string-ext
- mruby-proc-ext

Regenerate from an mruby 4.0.0 checkout with:

```sh
MRUBY_CONFIG=build_config/toyoterm.rb rake amalgam
```

The source is distributed under the MIT license in `LICENSE`.
