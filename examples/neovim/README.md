# LazyVim / Snacks images

[日本語](README.ja.md)

Copy [snacks.lua](snacks.lua) into your Neovim configuration as
`lua/plugins/toyoterm-image.lua`, then restart Neovim inside toyoterm.
This is a Lazy plugin specification, not a toyoterm Ruby configuration.
It enables Snacks images while preserving any other image options you set.

Snacks currently recognizes Kitty, Ghostty and WezTerm by name; it does not
probe Kitty graphics support. This example registers toyoterm through
`snacks.image.terminal.envs()` with Unicode placeholders and `remote = true`.
Here `remote` selects direct, chunked PNG transmission (`t=d`), even locally.
Snacks otherwise sends a local file path (`t=f`), which toyoterm does not accept.
No `TERM` changes, `force = true`, `SNACKS_KITTY`, or `SNACKS_SSH` are needed.
Remove earlier `SNACKS_KITTY` / `SNACKS_SSH` overrides from your Neovim config.

Snacks' default direct sender calls `uv.sleep(1)` after every 4 KiB chunk,
blocking Neovim's input loop. Windows timer granularity can turn this into
seconds of frozen input. This recipe replaces that sender for toyoterm: it
reads and encodes small blocks, sends at most 64 KiB per timer callback, and
returns control to Neovim between batches. Images are queued, and placement/
deletion requests are deferred until the current transfer finishes, so commands
from another buffer cannot interrupt a chunk sequence. Placements become ready
only after transmission. Failed transfers notify, release the file, and allow
the next queued image to proceed. The encoded transfer limit remains 32 MiB.

The registration applies only when `TERM_PROGRAM=toyoterm`, and skips tmux and
Zellij. tmux passthrough is not supported by toyoterm. SSH sessions do not
normally inherit `TERM_PROGRAM`; this example does not configure SSH forwarding.
The example registers an environment and wraps Snacks' image `send`, `ready`,
`failed`, and terminal `request` methods. It was checked with Snacks 2.31.0;
review these integration points when upgrading Snacks. ImageMagick conversion,
terminal decoding, and GPU upload can still contribute to display latency.

Open a small PNG with `:edit /absolute/path/image.png`, then run
`:checkhealth snacks`. Check that the image section reports `toyoterm` and
Unicode placeholders as supported. Install ImageMagick (`magick`) for conversion
of formats other than PNG. Snacks may still report a missing Kitty/WezTerm/
Ghostty executable; installing one is not required by this integration.

For inline Markdown images, enable LazyVim's `lang.markdown` extra and the
Treesitter parsers reported by health checks. Try scrolling, resizing, opening
the same PNG in another buffer, and closing it. Native protocol regression tests
cover direct transmission, multiple virtual placements, and independent deletion;
they do not replace this GUI check. toyoterm's
[image limits](../../docs/image-protocols.md) still apply.

Run the integration check from the repository root with an installed Snacks:

```sh
nvim --headless -u NONE -l examples/neovim/check.lua /path/to/snacks.nvim
```

This checks event-loop progress during transfer, byte transport, readiness,
queue order, deferred commands, failure recovery, and fallback to the original
sender for other terminals. The 3 MiB fixture measures sender overhead with
terminal writes captured, not end-to-end rendering. On the development Windows
machine, one original transfer took 13.4 s; two cooperative transfers took
about 1.6–1.7 s total.
