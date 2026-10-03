# Unicode / IME / rendering regressions

This suite records current behavior for v0.2.0 P0-1. It does not add a font
engine, ligature option, bidi implementation, or Unicode protocol.

## Automated coverage

| Owning test | Cases and assertions |
| --- | --- |
| `toyoterm-terminal/tests/unicode_regression.rs` | ASCII; 日本語, 界, 漢字ABC, ABC日本語XYZ; e + U+0301 and a + U+0308; 😀, 👍🏻, 🇯🇵, family ZWJ; mixed A界é😀Z; Powerline U+E0B0/U+E0B2 and PUA U+F120/U+F005. Byte-fragmented UTF-8, exact snapshot text, cell widths/columns, cursor position and forward/reverse UTF-8 copy. |
| `mixed_selection_expands_wide_trailing_cells_and_preserves_newlines` | Drag from either wide trailing cell across CJK, combining and emoji; exact selected spans and multiline copy. |
| `osc66_keeps_graphemes_atomic_and_copies_each_block_once` | CJK, combining, emoji modifier, flag and ZWJ graphemes at scale 2; one cell block per grapheme, rectangle/cursor advance, mixed ordinary/sized copy from top and lower rows. |
| `ime_commit_uses_utf8_or_kitty_associated_codepoints` | Japanese and composed Unicode through the same `encode_ime_commit` API used by GUI commits. Parsed Kitty flags 0/8/16/24; exact UTF-8 or associated decimal codepoint sequences. |
| renderer `unicode_and_ligature_shaping_preserves_grid_selection_and_cursor` | Advanced shaping of fi/fl/ffi/->/=>/==/===/!=/!==, mixed Unicode, emoji and PUA; separate fallback runs, finite glyph positions, explicit run origins, fixed cursor columns, selection rectangles and copy. No particular ligature or installed font is required. |
| renderer `shared_glyph_keys_follow_effective_style_and_preserve_cell_geometry` | Ordinary isolated glyphs share shaped buffers across screen positions only with equal text, width and effective glyph attributes, including selection foreground, hyperlink decoration and combining marks. Sized blocks and multi-cell ASCII runs retain separate buffers. Each pane retains at most 512 shared entries; layout, colors, selection and font changes invalidate the cache. Shared buffers are immutable; changed runs receive uniquely owned buffers before shaping. |
| renderer `unicode_cursor_shapes_span_the_occupied_rectangle` | Block/Beam/Underline over ASCII, CJK, both combining examples and emoji, ordinary and OSC 66. Beam stays one thin glyph per row; block/underline use occupied columns. Sized interior lookup covers the lower row. |
| renderer `cursor_text_preserves_glyph_layout_and_overrides_cell_colors` | Extended to combining diaeresis, emoji modifier, flag and ZWJ; normal and fractional sized text keep glyph layout during block text redraw, explicit color overrides and hidden text. Existing automatic black/white contrast tests remain active. |
| renderer offscreen `unicode_render_colors_and_wide_cursor_have_stable_pixels` | Exact pixel assertions for two-cell block coverage, redraw color with explicit/automatic contrast, selected Unicode foreground and default foreground fallback. Uses existing CPU `TestImage` bitmap masks; no new or refreshed image snapshots. |
| renderer `gpu_unicode_bitmap_composition_matches_offscreen_pixels` | Opt-in GPU readback through the production UI pipeline using the same recorded bitmap masks and rectangles. Compares every RGB pixel against the CPU frames with a one-byte tolerance; covers wide cursor, cursor text contrast, selection foreground and default color without system fonts. |
| app `grid_hit_testing_selects_unicode_and_ligature_candidates_at_any_scale` | Production mouse-to-cell calculation (extracted without changing its arithmetic), pane offsets/padding, scales 1/1.25/2, ligature candidate columns and wide trailing cells; hit cells select the expected logical text. |

Ordinary VT input currently follows scalar widths: 👍🏻 occupies 4 cells,
🇯🇵 2 cells (two one-cell regional indicators), and 👨‍👩‍👧‍👦 8 cells.
Combining marks and ZWJ attach to preceding cells. These tests deliberately
preserve that behavior rather than promise grapheme clustering for normal VT
input. OSC 66 uses grapheme segmentation: each of those emoji sequences has
Unicode width 2 and occupies 4 columns at scale 2. Selecting into a lower
OSC 66 row preserves the selected newline while emitting the block text once.

Run the suite and repository checks with locked dependencies:

```sh
cargo test -p toyoterm-terminal --test unicode_regression --locked
cargo test -p toyoterm-render --lib --locked
cargo test -p toyoterm-app --lib grid_hit_testing --locked
cargo fmt --check
python3 scripts/check-crate-architecture.py
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
sh scripts/check-licenses.sh
# Requires a GPU/software adapter; ignored by the ordinary workspace suite:
cargo test -p toyoterm-render --lib gpu_unicode_bitmap --locked -- --ignored
```

## Manual validation

Inside toyoterm, run `python3 examples/unicode_rendering_test.py` (use `python`
on Windows). `--plain` prints the text without controls or waiting. Interactive
mode clears the viewport; use a pane at least 80 columns by 20 rows.
Use `--cursor block|beam|underline` and `--target ascii|cjk|combining|sized`
to repeat each cursor/target combination. Press Enter to restore the default
cursor and finish.

- Inspect actual glyph fallback for CJK, combining, emoji, Powerline and Nerd
  Font PUA using your configured fonts. Missing-glyph boxes are permitted when
  no suitable font is installed; cells and neighboring text must remain aligned.
- Inspect wide block/underline width, thin beam, retained combining marks during
  block redraw, sized text and selection foreground. Compare default colors,
  explicit cursor-text colors and light/dark cursor backgrounds in existing
  configuration or OSC color controls.
- Drag forward/backward across wide trailing cells, combining marks, emoji,
  multiline text and sized blocks. Paste into a UTF-8 editor and compare text;
  do not expect copy to normalize combining sequences.
- Commit Japanese and composed Unicode with the OS IME; inspect preedit,
  candidate placement, cancellation and focus changes, then confirm committed
  text reaches the shell. Repeat with an application requesting Kitty flags
  8+16 and with those flags disabled.
- Repeat on Linux X11/Wayland, macOS and Windows, including fractional DPI,
  font changes and pane resizing. Run `cargo run --locked -- gui-smoke-test`
  where a display/GPU is available.

Actual glyph rasterization, color emoji, GPU text composition, OS clipboard,
IME preedit/candidate UI and platform/font differences remain manual. The
pixel regressions use synthetic bitmap masks and do not claim to validate GPU
glyph outlines. Adapter-dependent GPU tests remain opt-in/ignored in
the ordinary workspace run. Shaping tests compare layout invariants rather
than system-dependent glyph IDs, counts or pixel images across machines.

## Recorded validation

On Windows on 2026-10-02, the workspace/all-targets locked run passed
499 tests with 7 opt-in GPU/performance tests ignored. The new GPU bitmap
composition test was also run explicitly and passed. Formatting, Clippy with
warnings denied, crate architecture, license notices, fixture syntax and plain
UTF-8 output checks passed. `cargo run --locked -- gui-smoke-test` exited
successfully; its optional live Ruby console reported access denied while
writing IPC instance state in this restricted environment. Interactive IME,
font inspection and Linux/macOS validation were not performed.
