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

The registration applies only when `TERM_PROGRAM=toyoterm`, and skips tmux and
Zellij. tmux passthrough is not supported by toyoterm. SSH sessions do not
normally inherit `TERM_PROGRAM`; this example does not configure SSH forwarding.
The example uses Snacks' environment registration interface, so review it if
you upgrade Snacks and that interface changes.

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
