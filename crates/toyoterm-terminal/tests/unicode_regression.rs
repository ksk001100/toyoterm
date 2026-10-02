//! Public-boundary regressions: VT bytes -> grid -> selection/copy and IME bytes.
use toyoterm_terminal::{
    AlacrittyTerminalBackend, SelectionKind, SelectionSpan, TerminalBackend, encode_ime_commit,
};

#[test]
fn unicode_grid_and_copy_preserve_current_vt_widths() {
    // Ordinary VT input is scalar-based, unlike OSC 66's grapheme-based input.
    for (text, widths) in [
        ("ASCII", vec![1; 5]),
        ("日本語", vec![2; 3]),
        ("界", vec![2]),
        ("漢字ABC", vec![2, 2, 1, 1, 1]),
        ("ABC日本語XYZ", vec![1, 1, 1, 2, 2, 2, 1, 1, 1]),
        ("e\u{301}", vec![1]),
        ("a\u{308}", vec![1]),
        ("😀", vec![2]),
        ("👍🏻", vec![2, 2]),
        ("🇯🇵", vec![1, 1]),
        ("👨‍👩‍👧‍👦", vec![2, 2, 2, 2]),
        ("A界e\u{301}😀Z", vec![1, 2, 1, 2, 1]),
        ("\u{e0b0}\u{e0b2}\u{f120}\u{f005}", vec![1; 4]),
    ] {
        let mut terminal = AlacrittyTerminalBackend::new(64, 4);
        // Split every UTF-8 codepoint, not just between characters.
        for byte in text.as_bytes() {
            terminal.advance(&[*byte]);
        }
        let snapshot = terminal.snapshot();
        assert_eq!(snapshot.lines[0], text, "{text:?}");
        assert_eq!(
            snapshot.cells[0]
                .iter()
                .map(|c| c.width)
                .collect::<Vec<_>>(),
            widths,
            "{text:?}"
        );
        let mut column = 0;
        for cell in &snapshot.cells[0] {
            assert_eq!(cell.column, column, "{text:?}");
            column += u16::from(cell.width);
        }
        assert_eq!(terminal.cursor().column, column, "{text:?}");
        assert_eq!(terminal.cursor().row, 0);
        for (start, end) in [(0, column - 1), (column - 1, 0)] {
            terminal.start_selection(start, 0, SelectionKind::Simple);
            terminal.update_selection(end, 0);
            assert_eq!(terminal.selected_text().as_deref(), Some(text), "{text:?}");
        }
    }
}

#[test]
fn mixed_selection_expands_wide_trailing_cells_and_preserves_newlines() {
    let mut terminal = AlacrittyTerminalBackend::new(20, 4);
    terminal.advance("A界e\u{301}😀Z\r\n日本語".as_bytes());
    for (start, end) in [(2, 5), (5, 2)] {
        terminal.start_selection(start, 0, SelectionKind::Simple);
        terminal.update_selection(end, 0);
        assert_eq!(terminal.selected_text().as_deref(), Some("界e\u{301}😀"));
        assert_eq!(
            terminal.snapshot().selection,
            [SelectionSpan {
                row: 0,
                start_column: 1,
                end_column: 5
            }]
        );
    }
    terminal.start_selection(0, 0, SelectionKind::Simple);
    terminal.update_selection(5, 1);
    assert_eq!(
        terminal.selected_text().as_deref(),
        Some("A界e\u{301}😀Z\n日本語")
    );
}

#[test]
fn osc66_keeps_graphemes_atomic_and_copies_each_block_once() {
    for (text, width) in [
        ("界", 4),
        ("e\u{301}", 2),
        ("a\u{308}", 2),
        ("😀", 4),
        ("👍🏻", 4),
        ("🇯🇵", 4),
        ("👨‍👩‍👧‍👦", 4),
    ] {
        let mut terminal = AlacrittyTerminalBackend::new(40, 4);
        terminal.advance(format!("A\x1b]66;s=2;{text}\x07Z").as_bytes());
        let snapshot = terminal.snapshot();
        let cell = &snapshot.cells[0][1];
        assert_eq!(
            (cell.column, cell.text.as_str(), cell.width),
            (1, text, width)
        );
        assert_eq!(cell.text_size.unwrap().rows, 2);
        assert_eq!(terminal.cursor().column, u16::from(width) + 2);
        // Start inside the lower-right occupied cell, then drag back to A.
        terminal.start_selection(u16::from(width), 1, SelectionKind::Simple);
        terminal.update_selection(0, 0);
        assert_eq!(
            terminal.selected_text().as_deref(),
            Some(format!("A{text}Z\n").as_str())
        );
        terminal.start_selection(0, 0, SelectionKind::Simple);
        terminal.update_selection(u16::from(width) + 1, 0);
        assert_eq!(
            terminal.selected_text().as_deref(),
            Some(format!("A{text}Z").as_str())
        );
    }
}

#[test]
fn ime_commit_uses_utf8_or_kitty_associated_codepoints() {
    for (text, codepoints) in [
        ("日本語", "26085:26412:35486"),
        ("e\u{301}a\u{308}😀", "101:769:97:776:128512"),
    ] {
        // Flag 16 alone is insufficient: associated text requires flag 8 too.
        for (flags, expected) in [
            (0, text.as_bytes().to_vec()),
            (8, text.as_bytes().to_vec()),
            (16, text.as_bytes().to_vec()),
            (24, format!("\x1b[0;;{codepoints}u").into_bytes()),
        ] {
            let mut terminal = AlacrittyTerminalBackend::new(40, 4);
            terminal.advance(format!("\x1b[>{flags}u").as_bytes());
            assert_eq!(
                encode_ime_commit(text, terminal.mode()),
                expected,
                "flags={flags}"
            );
        }
    }
}
