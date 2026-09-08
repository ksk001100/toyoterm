# Vendored mruby

These files are the mruby 4.0.0 amalgamation generated from commit
`831da26b9021de0369d17b71b5667e2941a1a32d` of the official mruby repository.

The build includes `mruby-error` and these core gemboxes:

- `stdlib`: collection, object, numeric, range, symbol, fiber, enumerator,
  object-space, kernel, class, and control-flow extensions
- `stdlib-ext`: pack/unpack, formatting, time, struct, data, and random APIs
- `math`: math, rational, complex, and multi-precision integer APIs
- `metaprog`: compiler, eval, binding, proc binding, reflection, and method APIs

Platform-dependent I/O, directory, socket, task, sleep, exit, binary, and test
gems are intentionally excluded so the amalgamation remains portable across
Linux, macOS, and Windows and does not change toyoterm's runtime ownership.

Regenerate from an mruby 4.0.0 checkout with:

```sh
MRUBY_CONFIG=build_config/toyoterm.rb rake amalgam
```

The source is distributed under the MIT license in `LICENSE`.
