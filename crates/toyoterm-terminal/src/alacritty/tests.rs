use super::*;
use base64::Engine;

#[test]
fn parses_text_and_ansi_cursor_movement() {
    let mut backend = AlacrittyTerminalBackend::new(10, 3);
    backend.advance(b"hello\rX");
    assert_eq!(backend.snapshot().lines[0], "Xello");
    assert_eq!(
        backend.cursor(),
        CursorState {
            column: 1,
            row: 0,
            visible: true,
            shape: CursorShape::Block,
        }
    );
}

#[test]
fn preserves_utf8_across_input_chunks() {
    let mut backend = AlacrittyTerminalBackend::new(10, 2);
    let bytes = "日本語".as_bytes();
    backend.advance(&bytes[..2]);
    backend.advance(&bytes[2..5]);
    backend.advance(&bytes[5..]);
    assert_eq!(backend.snapshot().lines[0], "日本語");
}

#[test]
fn tracks_wide_combining_cjk_and_emoji_cell_widths() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance("A界e\u{301}😀".as_bytes());

    let snapshot = backend.snapshot();
    assert_eq!(snapshot.lines[0], "A界e\u{301}😀");
    assert_eq!(
        snapshot.cells[0]
            .iter()
            .map(|cell| (cell.column, cell.text.as_str(), cell.width))
            .collect::<Vec<_>>(),
        [(0, "A", 1), (1, "界", 2), (3, "e\u{301}", 1), (4, "😀", 2),]
    );
    assert_eq!(backend.cursor().column, 6);
}

#[test]
fn renders_kitty_osc66_scaled_and_fractional_text_blocks() {
    let mut backend = AlacrittyTerminalBackend::new(12, 4);
    backend.advance(b"\x1b]66;s=2;Hi\x07");
    backend.advance(b"\x1b]66;n=1:d=2:w=1:v=1:h=2;xy\x1b\\");

    let snapshot = backend.snapshot();
    assert_eq!(snapshot.lines[0], "Hixy");
    assert_eq!(backend.cursor().column, 5);
    assert_eq!(
        snapshot.cells[0]
            .iter()
            .map(|cell| (cell.column, cell.text.as_str(), cell.width, cell.text_size))
            .collect::<Vec<_>>(),
        [
            (
                0,
                "H",
                2,
                Some(TextSize {
                    scale: 2,
                    numerator: 0,
                    denominator: 0,
                    vertical_alignment: TextAlignment::Start,
                    horizontal_alignment: TextAlignment::Start,
                    rows: 2,
                }),
            ),
            (
                2,
                "i",
                2,
                Some(TextSize {
                    scale: 2,
                    numerator: 0,
                    denominator: 0,
                    vertical_alignment: TextAlignment::Start,
                    horizontal_alignment: TextAlignment::Start,
                    rows: 2,
                }),
            ),
            (
                4,
                "xy",
                1,
                Some(TextSize {
                    scale: 1,
                    numerator: 1,
                    denominator: 2,
                    vertical_alignment: TextAlignment::End,
                    horizontal_alignment: TextAlignment::Center,
                    rows: 1,
                }),
            ),
        ]
    );
    backend.start_selection(4, 0, SelectionKind::Simple);
    assert_eq!(backend.selected_text().as_deref(), Some("xy"));
}

#[test]
fn osc66_wraps_and_normal_text_erases_intersecting_blocks() {
    let mut backend = AlacrittyTerminalBackend::new(6, 3);
    backend.advance(b"12345\x1b]66;s=2;A\x07");
    assert_eq!(backend.snapshot().cells[1][0].width, 2);
    assert_eq!(
        backend.cursor(),
        CursorState {
            column: 2,
            row: 1,
            visible: true,
            shape: CursorShape::Block,
        }
    );

    backend.advance(b"\rX");
    let snapshot = backend.snapshot();
    assert_eq!(snapshot.lines[1], "X");
    assert!(
        snapshot.cells[1]
            .iter()
            .all(|cell| cell.text_size.is_none())
    );
}

#[test]
fn normal_text_skips_the_lower_rows_of_an_osc66_block() {
    let mut backend = AlacrittyTerminalBackend::new(8, 3);
    backend.advance(b"\x1b]66;s=2;A\x07\x1b[2;1HX");

    let snapshot = backend.snapshot();
    assert_eq!(snapshot.cells[0][0].text, "A");
    assert_eq!(snapshot.cells[0][0].width, 2);
    assert_eq!(snapshot.lines[1], "X");
    assert_eq!(snapshot.cells[1][0].column, 2);
    assert_eq!(backend.cursor().column, 3);
    backend.start_selection(1, 1, SelectionKind::Simple);
    assert_eq!(backend.selected_text().as_deref(), Some("A"));
}

#[test]
fn osc66_skips_all_adjacent_lower_row_blocks_even_without_autowrap() {
    let mut normal = AlacrittyTerminalBackend::new(10, 3);
    normal.advance(b"\x1b]66;s=2;AB\x07\x1b[2;1H\x1b[?7lX");
    let snapshot = normal.snapshot();
    assert_eq!(snapshot.cells[1][0].column, 4);
    assert_eq!(snapshot.cells[1][0].text, "X");
    assert_eq!(normal.cursor().column, 5);

    let mut sized = AlacrittyTerminalBackend::new(10, 3);
    sized.advance(b"\x1b]66;s=2;AB\x07\x1b[2;1H\x1b[?7l\x1b]66;w=1;Y\x07");
    let snapshot = sized.snapshot();
    let block = snapshot.cells[1]
        .iter()
        .find(|cell| cell.text_size.is_some())
        .unwrap();
    assert_eq!((block.column, block.text.as_str()), (4, "Y"));
    assert_eq!(sized.cursor().column, 5);
}

#[test]
fn combining_text_extends_the_preceding_osc66_block() {
    let mut backend = AlacrittyTerminalBackend::new(8, 3);
    backend.advance("\x1b]66;s=2;A\x07\u{301}".as_bytes());

    let snapshot = backend.snapshot();
    assert_eq!(snapshot.cells[0][0].text, "A\u{301}");
    assert_eq!(backend.cursor().column, 2);
}

#[test]
fn selection_extracts_mixed_normal_and_osc66_text_once() {
    let mut backend = AlacrittyTerminalBackend::new(12, 3);
    backend.advance(b"A\x1b]66;w=2;BC\x07D");
    backend.start_selection(0, 0, SelectionKind::Simple);
    backend.update_selection(3, 0);
    assert_eq!(backend.selected_text().as_deref(), Some("ABCD"));

    let mut lower_row = AlacrittyTerminalBackend::new(8, 3);
    lower_row.advance(b"\x1b]66;s=2;A\x07\x1b[2;1HX");
    lower_row.start_selection(0, 1, SelectionKind::Simple);
    lower_row.update_selection(2, 1);
    assert_eq!(lower_row.selected_text().as_deref(), Some("AX"));
}

#[test]
fn rejects_malformed_or_oversized_osc66_text() {
    let mut backend = AlacrittyTerminalBackend::new(12, 3);
    backend.advance(b"\x1b]66;s=0;bad\x07\x1b]66;n=2:d=1;bad\x07");
    let oversized = format!("\x1b]66;w=1;{}\x07", "x".repeat(4_097));
    backend.advance(oversized.as_bytes());
    assert!(backend.snapshot().lines.iter().all(String::is_empty));
    assert_eq!(backend.cursor().column, 0);
}

#[test]
fn osc66_blocks_follow_character_edits_or_clear_when_split() {
    let mut backend = AlacrittyTerminalBackend::new(10, 4);
    backend.advance(b"\x1b]66;w=1;A\x07\r\x1b[@");
    assert_eq!(
        backend.snapshot().cells[0]
            .iter()
            .find(|cell| cell.text_size.is_some())
            .unwrap()
            .column,
        1
    );
    backend.advance(b"\r\x1b[P");
    assert_eq!(
        backend.snapshot().cells[0]
            .iter()
            .find(|cell| cell.text_size.is_some())
            .unwrap()
            .column,
        0
    );

    let mut split = AlacrittyTerminalBackend::new(10, 4);
    split.advance(b"\x1b]66;w=2;AB\x07\x1b[2G\x1b[@");
    assert!(
        split.snapshot().cells[0]
            .iter()
            .all(|cell| cell.text_size.is_none())
    );
}

#[test]
fn osc66_multiline_blocks_clear_when_line_edits_split_them() {
    let mut inserted = AlacrittyTerminalBackend::new(10, 4);
    inserted.advance(b"\x1b]66;s=2;A\x07\x1b[2;1H\x1b[L");
    assert!(
        inserted
            .snapshot()
            .cells
            .iter()
            .flatten()
            .all(|cell| cell.text_size.is_none())
    );

    let mut deleted = AlacrittyTerminalBackend::new(10, 4);
    deleted.advance(b"\x1b]66;s=2;A\x07\x1b[1;1H\x1b[M");
    assert!(
        deleted
            .snapshot()
            .cells
            .iter()
            .flatten()
            .all(|cell| cell.text_size.is_none())
    );
}

#[test]
fn osc66_line_edits_preserve_shift_or_clip_whole_blocks() {
    let mut insert = AlacrittyTerminalBackend::new(10, 6);
    insert.advance(b"\x1b[4;1H\x1b]66;w=1;A\x07\x1b[4;1H\x1b[L");
    let snapshot = insert.snapshot();
    let block = snapshot.cells[4]
        .iter()
        .find(|cell| cell.text_size.is_some())
        .unwrap();
    assert_eq!(block.text, "A");

    let mut insert_clip = AlacrittyTerminalBackend::new(10, 6);
    insert_clip.advance(b"\x1b[5;1H\x1b]66;s=2;A\x07\x1b[5;1H\x1b[L");
    assert!(
        insert_clip
            .snapshot()
            .cells
            .iter()
            .flatten()
            .all(|cell| cell.text_size.is_none())
    );

    let mut delete = AlacrittyTerminalBackend::new(10, 6);
    delete.advance(b"\x1b[4;1H\x1b]66;w=1;A\x07\x1b[2;1H\x1b[M");
    let snapshot = delete.snapshot();
    let block = snapshot.cells[2]
        .iter()
        .find(|cell| cell.text_size.is_some())
        .unwrap();
    assert_eq!(block.text, "A");

    let mut region = AlacrittyTerminalBackend::new(10, 6);
    region.advance(b"\x1b[2;5r\x1b[2;1H\x1b]66;s=2;A\x07\x1b[4;1H\x1b[L");
    let snapshot = region.snapshot();
    let block = snapshot.cells[1]
        .iter()
        .find(|cell| cell.text_size.is_some())
        .unwrap();
    assert_eq!(block.text, "A");
}

#[test]
fn osc66_character_edits_handle_multiline_and_right_edge_cases() {
    let mut multiline_insert = AlacrittyTerminalBackend::new(10, 4);
    multiline_insert.advance(b"\x1b[4G\x1b]66;s=2;A\x07\x1b[1;2H\x1b[@");
    assert!(
        multiline_insert
            .snapshot()
            .cells
            .iter()
            .flatten()
            .all(|cell| cell.text_size.is_none())
    );

    let mut multiline_delete = AlacrittyTerminalBackend::new(10, 4);
    multiline_delete.advance(b"\x1b[4G\x1b]66;s=2;A\x07\x1b[1;2H\x1b[P");
    assert!(
        multiline_delete
            .snapshot()
            .cells
            .iter()
            .flatten()
            .all(|cell| cell.text_size.is_none())
    );

    let mut clipped = AlacrittyTerminalBackend::new(10, 4);
    clipped.advance(b"\x1b[9G\x1b]66;w=2;A\x07\x1b[1;8H\x1b[2@");
    assert!(
        clipped.snapshot().cells[0]
            .iter()
            .all(|cell| cell.text_size.is_none())
    );
}

#[test]
fn osc66_blocks_survive_safe_expansion_and_clear_before_reflow() {
    let mut backend = AlacrittyTerminalBackend::new(10, 4);
    backend.advance(b"\x1b]66;s=2;A\x07");
    backend.resize(12, 4);
    assert!(backend.snapshot().cells[0][0].text_size.is_some());

    backend.resize(8, 4);
    assert!(
        backend
            .snapshot()
            .cells
            .iter()
            .flatten()
            .all(|cell| cell.text_size.is_none())
    );
}

#[test]
fn osc66_support_is_detectable_with_cursor_position_reports() {
    let mut backend = AlacrittyTerminalBackend::new(12, 4);
    backend.advance(b"\r\x1b[6n\x1b]66;w=2; \x07\x1b[6n\x1b]66;s=2; \x07\x1b[6n");
    let replies = backend
        .drain_events()
        .into_iter()
        .filter_map(|event| match event {
            TerminalEvent::PtyWrite(reply) => Some(reply),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(replies, ["\x1b[1;1R", "\x1b[1;3R", "\x1b[1;5R"]);
}

#[test]
fn exposes_sgr_colors_and_text_attributes_in_snapshot_cells() {
    let mut backend = AlacrittyTerminalBackend::new(10, 2);
    backend.advance(b"\x1b[1;2;3;4;5;7;9;38;5;196;48;2;1;2;3mX");

    let cell = &backend.snapshot().cells[0][0];
    assert_eq!(cell.text, "X");
    assert_eq!(cell.attributes.foreground, CellColor::Indexed(196));
    assert_eq!(cell.attributes.background, CellColor::Rgb(1, 2, 3));
    assert!(cell.attributes.bold);
    assert!(cell.attributes.dim);
    assert!(cell.attributes.italic);
    assert!(cell.attributes.underline);
    assert!(cell.attributes.blink);
    assert!(cell.attributes.inverse);
    assert!(cell.attributes.strikethrough);
}

#[test]
fn tracks_slow_and_fast_blink_until_cancel_or_reset() {
    let mut backend = AlacrittyTerminalBackend::new(10, 2);
    backend.advance(b"\x1b[5mA\x1b[25mB\x1b[6mC\x1b[0mD");

    let snapshot = backend.snapshot();
    let blink = snapshot.cells[0]
        .iter()
        .map(|cell| (cell.text.as_str(), cell.attributes.blink))
        .collect::<Vec<_>>();
    assert_eq!(
        blink,
        [("A", true), ("B", false), ("C", true), ("D", false)]
    );
}

#[test]
fn exposes_osc8_links_and_detects_plain_urls() {
    let mut backend = AlacrittyTerminalBackend::new(60, 2);
    backend.advance(
        b"\x1b]8;;https://example.com/docs\x1b\\manual\x1b]8;;\x1b\\ http://localhost:3000/test).",
    );

    let snapshot = backend.snapshot();
    assert_eq!(
        snapshot.cells[0][0].hyperlink.as_deref(),
        Some("https://example.com/docs")
    );
    let plain = snapshot.cells[0]
        .iter()
        .find(|cell| cell.text == "l" && cell.column > 10)
        .expect("plain URL cell");
    assert_eq!(
        plain.hyperlink.as_deref(),
        Some("http://localhost:3000/test")
    );
}

#[test]
fn detects_multiple_plain_urls_after_unicode_and_shares_each_url() {
    let mut backend = AlacrittyTerminalBackend::new(100, 2);
    backend.advance("é https://a.test/x, mailto:b@example.test!".as_bytes());

    let snapshot = backend.snapshot();
    let cells = &snapshot.cells[0];
    let first = cells
        .iter()
        .find(|cell| cell.text == "h")
        .unwrap()
        .hyperlink
        .as_ref()
        .unwrap();
    let second = cells
        .iter()
        .find(|cell| cell.text == "m")
        .unwrap()
        .hyperlink
        .as_ref()
        .unwrap();
    assert_eq!(first.as_ref(), "https://a.test/x");
    assert_eq!(second.as_ref(), "mailto:b@example.test");
    assert!(std::sync::Arc::ptr_eq(
        first,
        cells
            .iter()
            .find(|cell| cell.text == "x")
            .unwrap()
            .hyperlink
            .as_ref()
            .unwrap()
    ));
    assert!(
        cells
            .iter()
            .any(|cell| cell.text == "é" && cell.hyperlink.is_none())
    );
    assert!(
        cells
            .iter()
            .any(|cell| cell.text == "," && cell.hyperlink.is_none())
    );
}

#[test]
fn answers_osc_palette_and_dynamic_color_queries() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    let mut ansi = [[0, 0, 0]; 16];
    ansi[3] = [12, 34, 56];
    backend.set_default_colors(
        [0x11, 0x22, 0x33],
        [0x44, 0x55, 0x66],
        [0x77, 0x88, 0x99],
        [0xaa, 0xbb, 0xcc],
        ansi,
    );

    backend.advance(b"\x1b]4;3;?\x07\x1b]10;?\x1b\\\x1b]11;?\x07\x1b]12;?\x1b\\");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::PtyWrite("\x1b]4;3;rgb:0c0c/2222/3838\x07".into()),
            TerminalEvent::PtyWrite("\x1b]10;rgb:1111/2222/3333\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]11;rgb:4444/5555/6666\x07".into()),
            TerminalEvent::PtyWrite("\x1b]12;rgb:7777/8888/9999\x1b\\".into()),
        ]
    );
}

#[test]
fn answers_iterm_default_foreground_and_background_alias_queries() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.set_default_colors(
        [0x11, 0x22, 0x33],
        [0x44, 0x55, 0x66],
        [0x77, 0x88, 0x99],
        [0xaa, 0xbb, 0xcc],
        [[0, 0, 0]; 16],
    );

    backend.advance(b"\x1b]4;-1;?\x07\x1b]4;-2;?\x1b\\");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::PtyWrite("\x1b]4;-1;rgb:1111/2222/3333\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]4;-2;rgb:4444/5555/6666\x1b\\".into()),
        ]
    );

    backend.advance(b"\x1b]10;#abcdef\x07\x1b]11;#123456\x07");
    backend.advance(b"\x1b]4;-1;?;-2;?\x1b\\");
    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::PtyWrite("\x1b]4;-1;rgb:abab/cdcd/efef\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]4;-2;rgb:1212/3434/5656\x1b\\".into()),
        ]
    );
}

#[test]
fn supports_xterm_selection_color_controls() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    let ansi = [[0, 0, 0]; 16];
    backend.set_default_colors(
        [0x11, 0x22, 0x33],
        [0x44, 0x55, 0x66],
        [0x77, 0x88, 0x99],
        [0xaa, 0xbb, 0xcc],
        ansi,
    );

    backend.advance(b"\x1b]17;#123\x07\x1b]19;rgb:40/50/60\x1b\\");
    assert_eq!(
        backend.render_colors().selection_background,
        Some([0x10, 0x20, 0x30])
    );
    assert_eq!(
        backend.render_colors().selection_foreground,
        Some([0x40, 0x50, 0x60])
    );
    backend.advance(b"\x1b]17;?\x07\x1b]19;?\x1b\\\x1b]117\x07\x1b]119\x1b\\");
    backend.advance(b"\x1b]17;?\x1b\\\x1b]19;?\x07");

    assert_eq!(backend.render_colors().selection_background, None);
    assert_eq!(backend.render_colors().selection_foreground, None);
    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::PtyWrite("\x1b]17;rgb:1010/2020/3030\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]19;rgb:4040/5050/6060\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]17;rgb:aaaa/bbbb/cccc\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]19;rgb:1111/2222/3333\x1b\\".into()),
        ]
    );
}

#[test]
fn supports_pointer_and_tektronix_dynamic_color_sequences() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.set_default_colors(
        [0x11, 0x22, 0x33],
        [0x44, 0x55, 0x66],
        [0x77, 0x88, 0x99],
        [0xaa, 0xbb, 0xcc],
        [[0, 0, 0]; 16],
    );

    backend.advance(b"\x1b]13;#010203;#040506;#070809;#0a0b0c;#0d0e0f;#101112;#131415\x1b\\");
    backend.advance(b"\x1b]13;?;?;?;?;?;?;?\x07");
    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::PtyWrite("\x1b]13;rgb:0101/0202/0303\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]14;rgb:0404/0505/0606\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]15;rgb:0707/0808/0909\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]16;rgb:0a0a/0b0b/0c0c\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]17;rgb:0d0d/0e0e/0f0f\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]18;rgb:1010/1111/1212\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]19;rgb:1313/1414/1515\x1b\\".into()),
        ]
    );

    backend
        .advance(b"\x1b]30001\x07\x1b]113\x07\x1b]114\x1b\\\x1b]115\x07\x1b]116\x1b\\\x1b]118\x07");
    backend.advance(b"\x1b]13;?;?;?;?\x07\x1b]18;?\x1b\\");
    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::PtyWrite("\x1b]13;rgb:1111/2222/3333\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]14;rgb:4444/5555/6666\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]15;rgb:1111/2222/3333\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]16;rgb:4444/5555/6666\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]18;rgb:7777/8888/9999\x1b\\".into()),
        ]
    );

    backend.advance(b"\x1b]30101\x1b\\\x1b]13;?\x07\x1b]18;?\x07");
    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::PtyWrite("\x1b]13;rgb:0101/0202/0303\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]18;rgb:1010/1111/1212\x1b\\".into()),
        ]
    );
}

#[test]
fn supports_osc21_color_sets_queries_and_resets() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    let mut ansi = [[0, 0, 0]; 16];
    ansi[1] = [1, 2, 3];
    backend.set_default_colors([4, 5, 6], [7, 8, 9], [10, 11, 12], [13, 14, 15], ansi);

    backend.advance(
            b"\x1b]21;1=#abc;foreground=rgb:f/0/8;cursor=rgbi:0.5/1/-1;unknown=?;1=?;foreground=?;cursor=?\x1b\\",
        );
    backend.advance(b"\x1b]21;1;foreground;1=?;foreground=?\x07");

    assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::PtyWrite(
                    "\x1b]21;unknown=?;1=rgb:a0/b0/c0;foreground=rgb:ff/00/88;cursor=rgb:80/ff/00\x1b\\"
                        .into(),
                ),
                TerminalEvent::PtyWrite(
                    "\x1b]21;1=rgb:01/02/03;foreground=rgb:04/05/06\x1b\\".into(),
                ),
            ]
        );
}

#[test]
fn supports_xterm_special_color_sets_queries_modes_resets_and_stack() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]5;0;#102030;1;#203040;2;#304050;3;#405060;4;#506070\x1b\\");
    backend.advance(b"\x1b]5;0;?;1;?;2;?;3;?;4;?\x07");
    backend.advance(b"\x1b]106;1;0;5;1\x1b\\\x1b]6;4;0\x07");

    let colors = backend.render_colors();
    assert_eq!(colors.special.bold, Some([0x10, 0x20, 0x30]));
    assert_eq!(colors.special.underline, Some([0x20, 0x30, 0x40]));
    assert_eq!(colors.special.blink, Some([0x30, 0x40, 0x50]));
    assert_eq!(colors.special.reverse, Some([0x40, 0x50, 0x60]));
    assert_eq!(colors.special.italic, Some([0x50, 0x60, 0x70]));
    assert!(!colors.special.enabled[1]);
    assert!(!colors.special.enabled[4]);
    assert!(colors.special.override_ansi);
    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::PtyWrite("\x1b]5;0;rgb:1010/2020/3030\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]5;1;rgb:2020/3030/4040\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]5;2;rgb:3030/4040/5050\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]5;3;rgb:4040/5050/6060\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]5;4;rgb:5050/6060/7070\x1b\\".into()),
        ]
    );

    backend.advance(b"\x1b]30001\x07\x1b]105;0;4\x1b\\\x1b]106;5;0\x07");
    assert_eq!(backend.render_colors().special.bold, None);
    assert_eq!(backend.render_colors().special.italic, None);
    assert!(!backend.render_colors().special.override_ansi);
    backend.advance(b"\x1b]30101\x1b\\");
    assert_eq!(backend.render_colors().special, colors.special);

    backend.advance(b"\x1b]105\x07");
    let reset = backend.render_colors().special;
    assert_eq!(reset.bold, None);
    assert_eq!(reset.underline, None);
    assert_eq!(reset.blink, None);
    assert_eq!(reset.reverse, None);
    assert_eq!(reset.italic, None);
}

#[test]
fn supports_explicit_kitty_selection_colors_and_resets() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    let ansi = [[0, 0, 0]; 16];
    backend.set_default_colors([1, 2, 3], [4, 5, 6], [7, 8, 9], [10, 11, 12], ansi);

    backend.advance(
            b"\x1b]21;selection_background=#abc;selection_foreground=rgb:1/2/3;cursor_text=#456;selection_background=?;selection_foreground=?;cursor_text=?\x1b\\",
        );
    backend.advance(b"\x1b]21;selection_background=;selection_background=?\x07");
    backend.advance(b"\x1b]21;selection_background;selection_foreground;cursor_text;selection_background=?;selection_foreground=?;cursor_text=?\x1b\\");

    assert_eq!(backend.render_colors().selection_background, None);
    assert_eq!(backend.render_colors().selection_foreground, None);
    assert_eq!(backend.render_colors().cursor_foreground, None);
    assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::PtyWrite(
                    "\x1b]21;selection_background=rgb:a0/b0/c0;selection_foreground=rgb:11/22/33;cursor_text=rgb:40/50/60\x1b\\"
                        .into()
                ),
                TerminalEvent::PtyWrite(
                    "\x1b]21;selection_background=\x1b\\".into()
                ),
                TerminalEvent::PtyWrite(
                    "\x1b]21;selection_background=rgb:0a/0b/0c;selection_foreground=;cursor_text=\x1b\\"
                        .into()
                ),
            ]
        );
}

#[test]
fn supports_explicit_and_dynamic_kitty_visual_bell_colors() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.set_default_colors(
        [220, 220, 220],
        [10, 20, 30],
        [255, 255, 255],
        [50, 60, 70],
        [[0, 0, 0]; 16],
    );

    backend.advance(b"\x1b]21;visual_bell=?;visual_bell=;visual_bell=?\x1b\\");
    assert_eq!(backend.render_colors().visual_bell, Some([255, 255, 255]));
    backend.advance(b"\x1b]21;visual_bell=#123456;visual_bell=?\x07");
    assert_eq!(
        backend.render_colors().visual_bell,
        Some([0x12, 0x34, 0x56])
    );
    backend.advance(b"\x1b]21;visual_bell;visual_bell=?\x1b\\");
    assert_eq!(backend.render_colors().visual_bell, None);

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::PtyWrite("\x1b]21;visual_bell=;visual_bell=\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]21;visual_bell=rgb:12/34/56\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]21;visual_bell=\x1b\\".into()),
        ]
    );
}

#[test]
fn supports_dynamic_kitty_cursor_and_selection_colors() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.set_default_colors(
        [1, 2, 3],
        [4, 5, 6],
        [7, 8, 9],
        [10, 11, 12],
        [[0, 0, 0]; 16],
    );

    backend.advance(
            b"\x1b]21;cursor=;cursor_text=;selection_background=;selection_foreground=;cursor=?;cursor_text=?;selection_background=?;selection_foreground=?\x1b\\",
        );
    let colors = backend.render_colors();
    assert_eq!(colors.cursor, [1, 2, 3]);
    assert_eq!(colors.cursor_foreground, Some([4, 5, 6]));
    assert!(colors.selection_background_dynamic);
    assert!(colors.selection_foreground_dynamic);
    assert_eq!(
        backend.drain_events(),
        vec![TerminalEvent::PtyWrite(
            "\x1b]21;cursor=;cursor_text=;selection_background=;selection_foreground=\x1b\\".into()
        )]
    );

    backend.advance(
            b"\x1b]30001\x1b\\\x1b]21;cursor=#111111;cursor_text=#222222;selection_background=#333333;selection_foreground=#444444\x1b\\\x1b]30101\x1b\\",
        );
    let colors = backend.render_colors();
    assert_eq!(colors.cursor, [1, 2, 3]);
    assert_eq!(colors.cursor_foreground, Some([4, 5, 6]));
    assert!(colors.selection_background_dynamic);
    assert!(colors.selection_foreground_dynamic);

    backend.advance(
            b"\x1b]21;cursor;cursor_text;selection_background;selection_foreground;cursor=?;cursor_text=?;selection_background=?;selection_foreground=?\x1b\\",
        );
    let colors = backend.render_colors();
    assert_eq!(colors.cursor, [7, 8, 9]);
    assert_eq!(colors.cursor_foreground, None);
    assert!(!colors.selection_background_dynamic);
    assert!(!colors.selection_foreground_dynamic);
    assert_eq!(
            backend.drain_events(),
            vec![TerminalEvent::PtyWrite(
                "\x1b]21;cursor=rgb:07/08/09;cursor_text=;selection_background=rgb:0a/0b/0c;selection_foreground=\x1b\\"
                    .into()
            )]
        );
}

#[test]
fn supports_kitty_transparent_background_color_slots() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(
            b"\x1b]21;transparent_background_color1=#abc@0.3;transparent_background_color7=blue@-1;transparent_background_color1=?;transparent_background_color7=?;transparent_background_color8=?\x1b\\",
        );
    assert_eq!(
        backend.render_colors().transparent_backgrounds,
        [
            Some(TerminalTransparentColor {
                color: [0xa0, 0xb0, 0xc0],
                opacity: Some(0.3),
            }),
            None,
            None,
            None,
            None,
            None,
            Some(TerminalTransparentColor {
                color: [0, 0, 255],
                opacity: None,
            }),
        ]
    );
    assert_eq!(
            backend.drain_events(),
            vec![TerminalEvent::PtyWrite(
                "\x1b]21;transparent_background_color1=rgb:a0/b0/c0@0.3;transparent_background_color7=rgb:00/00/ff;transparent_background_color8=?\x1b\\"
                    .into()
            )]
        );

    backend.advance(
        b"\x1b]30001\x1b\\\x1b]21;transparent_background_color1=red@2\x1b\\\x1b]30101\x1b\\",
    );
    assert_eq!(
        backend.render_colors().transparent_backgrounds[0].and_then(|color| color.opacity),
        Some(0.3)
    );

    backend.advance(
            b"\x1b]21;transparent_background_color1=;transparent_background_color7;transparent_background_color1=?;transparent_background_color7=?\x1b\\",
        );
    assert_eq!(backend.render_colors().transparent_backgrounds, [None; 7]);
    assert_eq!(
        backend.drain_events(),
        vec![TerminalEvent::PtyWrite(
            "\x1b]21;transparent_background_color1=;transparent_background_color7=\x1b\\".into()
        )]
    );
}

#[test]
fn supports_bounded_kitty_color_stack_with_bel_and_st() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]21;1=red\x1b\\\x1b]30001\x07\x1b]21;1=blue\x1b\\");
    backend.advance(b"\x1b]30001\x1b\\\x1b]21;1=green\x1b\\\x1b]30101\x07\x1b]21;1=?\x1b\\");
    backend.advance(b"\x1b]30101\x1b\\\x1b]21;1=?\x1b\\");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::PtyWrite("\x1b]21;1=rgb:00/00/ff\x1b\\".into()),
            TerminalEvent::PtyWrite("\x1b]21;1=rgb:ff/00/00\x1b\\".into()),
        ]
    );

    for _ in 0..12 {
        backend.advance(b"\x1b]30001\x1b\\");
    }
    assert_eq!(backend.color_stack.len(), 10);
}

#[test]
fn kitty_color_stack_preserves_iterm_session_colors() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(
            b"\x1b]1337;SetColors=bold=123456\x07\x1b]1337;SetColors=link=234567\x07\x1b]1337;SetColors=curfg=314159\x07\x1b]1337;SetColors=underline=271828\x07\x1b]1337;SetColors=selbg=345678\x07\x1b]1337;SetColors=selfg=456789\x07\x1b]30001\x07",
        );
    backend.advance(
            b"\x1b]1337;SetColors=bold=abcdef\x07\x1b]1337;SetColors=link=abcdef\x07\x1b]1337;SetColors=curfg=abcdef\x07\x1b]1337;SetColors=underline=abcdef\x07\x1b]1337;SetColors=selbg=abcdef\x07\x1b]1337;SetColors=selfg=abcdef\x07\x1b]30101\x07",
        );

    let colors = backend.render_colors();
    assert_eq!(colors.bold, [0x12, 0x34, 0x56]);
    assert_eq!(colors.link, Some([0x23, 0x45, 0x67]));
    assert_eq!(colors.cursor_foreground, Some([0x31, 0x41, 0x59]));
    assert_eq!(colors.underline, Some([0x27, 0x18, 0x28]));
    assert_eq!(colors.selection_background, Some([0x34, 0x56, 0x78]));
    assert_eq!(colors.selection_foreground, Some([0x45, 0x67, 0x89]));
}

#[test]
fn ignores_malformed_and_oversized_osc21_controls() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]21;1=not-a-color;foreground=rgb:ff/00\x07");
    let oversized = "x".repeat(8_193);
    backend.advance(format!("\x1b]21;{oversized}\x07").as_bytes());

    assert!(backend.drain_events().is_empty());
}

#[test]
fn emits_osc22_mouse_cursor_changes() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]22;pointer\x07\x1b]22;text\x1b\\\x1b]22;invalid-shape\x07");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::MouseCursorChanged(CursorIcon::Pointer),
            TerminalEvent::MouseCursorChanged(CursorIcon::Text),
        ]
    );
}

#[test]
fn supports_osc22_cursor_stacks_and_queries() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(
            b"\x1b]22;pointer\x1b\\\x1b]22;>wait,text\x1b\\\x1b]22;?__current__\x1b\\\x1b]22;<\x1b\\\x1b]22;?pointer,zoom-in,nope\x1b\\",
        );

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::MouseCursorChanged(CursorIcon::Pointer),
            TerminalEvent::MouseCursorChanged(CursorIcon::Text),
            TerminalEvent::PtyWrite("\x1b]22;text\x1b\\".into()),
            TerminalEvent::MouseCursorChanged(CursorIcon::Wait),
            TerminalEvent::PtyWrite("\x1b]22;1,1,0\x1b\\".into()),
        ]
    );
}

#[test]
fn keeps_osc22_cursor_stacks_separate_per_screen() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]22;pointer\x1b\\\x1b[?1049h\x1b]22;wait\x1b\\\x1b[?1049l");

    let events = backend.drain_events();
    assert_eq!(
        events.last(),
        Some(&TerminalEvent::MouseCursorChanged(CursorIcon::Pointer))
    );
}

#[test]
fn supports_iterm_text_cursor_shape_extension() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]1337;CursorShape=1\x07");
    assert_eq!(backend.cursor().shape, CursorShape::Beam);

    backend.advance(b"\x1b]1337;CursorShape=2\x1b\\");
    assert_eq!(backend.cursor().shape, CursorShape::Underline);

    backend.advance(b"\x1b]1337;CursorShape=0\x07");
    assert_eq!(backend.cursor().shape, CursorShape::Block);
}

#[test]
fn supports_bounded_xterm_font_names_menu_indices_and_queries() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.set_font_menu("Primary Mono", &["Fallback Mono".into()]);

    backend.advance(b"\x1b]50;?\x07\x1b]50;#+\x1b\\\x1b]50;#?\x07");
    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::PtyWrite("\x1b]50;Primary Mono\x1b\\".into()),
            TerminalEvent::FontFamilyChanged("Fallback Mono".into()),
            TerminalEvent::PtyWrite("\x1b]50;#1 Fallback Mono\x1b\\".into()),
        ]
    );

    backend.advance(b"\x1b]50;#-\x07\x1b]50;Custom Mono\x1b\\\x1b]50;#99\x07");
    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::FontFamilyChanged("Primary Mono".into()),
            TerminalEvent::FontFamilyChanged("Custom Mono".into()),
        ]
    );
}

#[test]
fn osc_palette_queries_report_overrides_and_resets() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    let mut ansi = [[0, 0, 0]; 16];
    ansi[1] = [1, 2, 3];
    backend.set_default_colors([4, 5, 6], [7, 8, 9], [10, 11, 12], [13, 14, 15], ansi);

    backend.advance(b"\x1b]4;1;#abcdef\x07\x1b]4;1;?\x07\x1b]104;1\x07\x1b]4;1;?\x07");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::PtyWrite("\x1b]4;1;rgb:abab/cdcd/efef\x07".into()),
            TerminalEvent::PtyWrite("\x1b]4;1;rgb:0101/0202/0303\x07".into()),
        ]
    );
}

#[test]
fn literal_search_navigates_scrollback_and_marks_visible_matches() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"needle one\r\nother\r\nneedle two");

    assert_eq!(
        backend.search("needle", SearchDirection::Next),
        SearchResult {
            current: 1,
            total: 2
        }
    );
    let snapshot = backend.snapshot();
    assert_eq!(snapshot.lines[0], "needle one");
    assert!(
        snapshot
            .search_matches
            .iter()
            .any(|found| found.active && found.row == 0)
    );

    assert_eq!(
        backend.search("needle", SearchDirection::Next),
        SearchResult {
            current: 2,
            total: 2
        }
    );
    assert_eq!(backend.snapshot().lines[1], "needle two");
    assert_eq!(
        backend.search("needle", SearchDirection::Previous),
        SearchResult {
            current: 1,
            total: 2
        }
    );
    backend.clear_search();
    assert!(backend.snapshot().search_matches.is_empty());
}

#[test]
fn visible_text_tracks_the_current_viewport_without_cell_metadata() {
    let mut backend = AlacrittyTerminalBackend::new(12, 2);
    backend.advance("one\r\n日本語\r\nthree".as_bytes());

    assert_eq!(backend.visible_text(), "日本語\nthree");
    backend.scroll_display(1);
    assert_eq!(backend.visible_text(), "one\n日本語");
    assert_eq!(backend.visible_text(), backend.snapshot().lines.join("\n"));
}

#[test]
fn exposes_true_color_and_reset_attributes() {
    let mut backend = AlacrittyTerminalBackend::new(10, 2);
    backend.advance(b"\x1b[38;2;12;34;56mA\x1b[0mB");

    let snapshot = backend.snapshot();
    assert_eq!(
        snapshot.cells[0][0].attributes.foreground,
        CellColor::Rgb(12, 34, 56)
    );
    assert_eq!(snapshot.cells[0][1].attributes, CellAttributes::default());
}

#[test]
fn tracks_terminal_modes_and_cursor_visibility() {
    let mut backend = AlacrittyTerminalBackend::new(10, 2);
    backend.advance(b"\x1b[?1h\x1b[?2004h\x1b[?1000h\x1b[?25l");
    assert_eq!(
        backend.mode(),
        TerminalMode {
            application_cursor: true,
            bracketed_paste: true,
            mouse_reporting: true,
            alternate_scroll: true,
            ..TerminalMode::default()
        }
    );
    assert!(!backend.cursor().visible);

    // DECSET 1002 (Cell motion / drag)
    backend.advance(b"\x1b[?1002h\x1b[?1006h");
    let mode = backend.mode();
    assert!(mode.mouse_reporting);
    assert!(mode.mouse_drag);
    assert!(!mode.mouse_motion);
    assert!(mode.sgr_mouse);

    // DECSET 1003 (All motion)
    backend.advance(b"\x1b[?1003h");
    let mode = backend.mode();
    assert!(mode.mouse_reporting);
    assert!(!mode.mouse_drag);
    assert!(mode.mouse_motion);
    assert!(mode.sgr_mouse);

    // DECRST 1003 (Reset motion)
    backend.advance(b"\x1b[?1003l\x1b[?1006l");
    let mode = backend.mode();
    assert!(!mode.mouse_reporting);
    assert!(!mode.mouse_drag);
    assert!(!mode.mouse_motion);
    assert!(!mode.sgr_mouse);
}

#[test]
fn tracks_keypad_focus_and_alternate_screen_modes() {
    let mut backend = AlacrittyTerminalBackend::new(10, 2);
    backend.advance(b"\x1b=\x1b[?1004h\x1b[?1049h");
    let mode = backend.mode();
    assert!(mode.application_keypad);
    assert!(mode.focus_reporting);
    assert!(mode.alternate_screen);
    assert!(mode.alternate_scroll);

    backend.advance(b"\x1b>\x1b[?1004l\x1b[?1049l");
    let mode = backend.mode();
    assert!(!mode.application_keypad);
    assert!(!mode.focus_reporting);
    assert!(!mode.alternate_screen);
}

#[test]
fn exposes_and_flushes_unterminated_synchronized_updates() {
    let mut backend = AlacrittyTerminalBackend::new(80, 24);
    backend.advance(b"ready");
    backend.advance(b"\x1b[?2026h held");

    assert!(backend.synchronized_update_deadline().is_some());
    assert!(!backend.visible_text().contains("held"));

    backend.stop_synchronized_update();

    assert!(backend.synchronized_update_deadline().is_none());
    assert!(backend.visible_text().contains("ready held"));
}

#[test]
fn handles_dec_autowrap_origin_and_bracketed_paste_modes() {
    let mut backend = AlacrittyTerminalBackend::new(5, 4);
    backend.advance(b"\x1b[?7labcdeX");
    assert_eq!(backend.snapshot().lines[0], "abcdX");

    backend.advance(b"\x1b[2J\x1b[2;3r\x1b[?6h\x1b[H");
    assert_eq!(backend.cursor().row, 1);

    backend.advance(b"\x1b[?2004h");
    assert!(backend.mode().bracketed_paste);
    backend.advance(b"\x1b[?2004l");
    assert!(!backend.mode().bracketed_paste);
}

#[test]
fn resize_updates_snapshot_dimensions() {
    let mut backend = AlacrittyTerminalBackend::new(80, 24);
    backend.resize(120, 40);
    let snapshot = backend.snapshot();
    assert_eq!((snapshot.columns, snapshot.rows), (120, 40));
    assert_eq!(snapshot.lines.len(), 40);
}

#[test]
fn disables_osc52_clipboard_access_without_disrupting_terminal_output() {
    assert_eq!(
        terminal_config(DEFAULT_SCROLLBACK_LINES).osc52,
        Osc52::Disabled
    );

    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"before\x1b]52;c;dG95b3Rlcm0=\x07after");

    assert_eq!(backend.snapshot().lines[0], "beforeafter");
    assert!(backend.drain_events().is_empty());
}

#[test]
fn permits_bounded_osc52_copies_without_clipboard_reads() {
    use base64::Engine as _;

    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.set_osc52_copy_enabled(true);
    backend.advance(b"\x1b]52;c;dG95b3Rlcm0=\x07\x1b]52;c;?\x1b\\");

    assert_eq!(
        backend.drain_events(),
        vec![TerminalEvent::ClipboardStore("toyoterm".into())]
    );

    let oversized =
        base64::engine::general_purpose::STANDARD.encode(vec![b'x'; MAX_OSC52_COPY_BYTES + 1]);
    backend.advance(format!("\x1b]52;c;{oversized}\x07").as_bytes());
    assert!(backend.drain_events().is_empty());
}

#[test]
fn parses_bounded_legacy_osc_notifications() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]9;build finished\x07\x1b]777;notify;Deploy;Production is ready\x1b\\");
    backend.advance(b"\x1b]9;4;1;50\x07\x1b]9;bad\nmessage\x07");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::Notification {
                id: None,
                title: None,
                body: "build finished".into(),
                occasion: NotificationOccasion::Always,
                urgency: NotificationUrgency::Normal,
                timeout_ms: None,
                sound: NotificationSound::System,
                icon_name: None,
                icon: None,
                buttons: Vec::new(),
                reporting: NotificationReporting::default(),
            },
            TerminalEvent::Notification {
                id: None,
                title: Some("Deploy".into()),
                body: "Production is ready".into(),
                occasion: NotificationOccasion::Always,
                urgency: NotificationUrgency::Normal,
                timeout_ms: None,
                sound: NotificationSound::System,
                icon_name: None,
                icon: None,
                buttons: Vec::new(),
                reporting: NotificationReporting::default(),
            },
            TerminalEvent::ProgressChanged(TerminalProgress::Normal(50)),
        ]
    );

    let oversized = "x".repeat(MAX_OSC_NOTIFICATION_BYTES + 1);
    backend.advance(format!("\x1b]9;{oversized}\x07").as_bytes());
    assert!(backend.drain_events().is_empty());
}

#[test]
fn parses_osc9_progress_states_and_rejects_invalid_values() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(
            b"\x1b]9;4;1;42\x07\x1b]9;4;2;100\x1b\\\x1b]9;4;2\x07\x1b]9;4;3\x07\x1b]9;4;4;7\x1b\\\x1b]9;4;4\x07\x1b]9;4\x07",
        );
    backend.advance(b"\x1b]9;4;1;101\x07\x1b]9;4;1;50;extra\x07\x1b]9;4;bogus\x07");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::ProgressChanged(TerminalProgress::Normal(42)),
            TerminalEvent::ProgressChanged(TerminalProgress::Error(Some(100))),
            TerminalEvent::ProgressChanged(TerminalProgress::Error(None)),
            TerminalEvent::ProgressChanged(TerminalProgress::Indeterminate),
            TerminalEvent::ProgressChanged(TerminalProgress::Warning(Some(7))),
            TerminalEvent::ProgressChanged(TerminalProgress::Warning(None)),
            TerminalEvent::ProgressChanged(TerminalProgress::Hidden),
        ]
    );
}

#[test]
fn parses_iterm_osc6_tab_colors_and_reset() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(
            b"\x1b]6;1;bg;red;brightness;255\x07\x1b]6;1;bg;green;brightness;32\x1b\\\x1b]6;1;bg;blue;brightness;128\x07",
        );
    backend.advance(b"\x1b]6;1;bg;red;brightness;256\x07\x1b]6;other\x07");
    backend.advance(b"\x1b]6;1;bg;*;default\x1b\\");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::TabColorChanged {
                component: TabColorComponent::Red,
                value: 255,
            },
            TerminalEvent::TabColorChanged {
                component: TabColorComponent::Green,
                value: 32,
            },
            TerminalEvent::TabColorChanged {
                component: TabColorComponent::Blue,
                value: 128,
            },
            TerminalEvent::TabColorReset,
            TerminalEvent::TitleReset,
        ]
    );
}

#[test]
fn parses_iterm_osc21337_session_status_updates_and_clears() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]21337;indicator=#12aBcF;status=deploying");
    backend.advance(b";status-color=#ff8800\x1b\\");
    backend.advance(b"\x1b]21337;status=ready\x07\x1b]21337;indicator=;status=;status-color=\x07");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::SessionStatusChanged(SessionStatusUpdate {
                indicator: Some(Some([0x12, 0xab, 0xcf])),
                status: Some(Some("deploying".into())),
                status_color: Some(Some([0xff, 0x88, 0x00])),
            }),
            TerminalEvent::SessionStatusChanged(SessionStatusUpdate {
                status: Some(Some("ready".into())),
                ..SessionStatusUpdate::default()
            }),
            TerminalEvent::SessionStatusChanged(SessionStatusUpdate {
                indicator: Some(None),
                status: Some(None),
                status_color: Some(None),
            }),
        ]
    );
}

#[test]
fn rejects_invalid_iterm_osc21337_session_status_updates() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]21337;indicator=red;status=ignored\x07");
    backend.advance(b"\x1b]21337;status=line\nbreak\x07");
    backend.advance(b"\x1b]21337;status-color=#12345g\x07");
    backend.advance(b"\x1b]21337;unknown=value\x07");
    let oversized = "x".repeat(MAX_OSC_SESSION_STATUS_BYTES + 1);
    backend.advance(format!("\x1b]21337;status={oversized}\x07").as_bytes());

    assert!(backend.drain_events().is_empty());
}

#[test]
fn parses_iterm_osc1337_tab_colors_and_reset() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]1337;SetColors=tab=f0a\x07\x1b]1337;SetColors=tab=srgb:102030\x1b\\");
    backend.advance(b"\x1b]1337;SetColors=tab=p3:ffffff\x07\x1b]1337;SetColors=tab=xyz\x07");
    backend.advance(b"\x1b]1337;SetColors=tab=default\x1b\\");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::TabColorSet([255, 0, 170]),
            TerminalEvent::TabColorSet([16, 32, 48]),
            TerminalEvent::TabColorSet([255, 255, 255]),
            TerminalEvent::TabColorReset,
        ]
    );
}

#[test]
fn converts_iterm_display_p3_colors_to_srgb() {
    assert_eq!(parse_iterm_color(b"p3:808080"), Some([128, 128, 128]));
    assert_eq!(parse_iterm_color(b"p3:ff8000"), Some([255, 119, 0]));
    assert_eq!(parse_iterm_color(b"p3:fg0"), None);
}

#[test]
fn applies_iterm_osc1337_terminal_and_ansi_colors() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(
            b"\x1b]1337;SetColors=fg=f0a\x07\x1b]1337;SetColors=bg=srgb:102030\x1b\\\x1b]1337;SetColors=curbg=rgb:abcdef\x07",
        );
    backend.advance(b"\x1b]1337;SetColors=red=123\x07\x1b]1337;SetColors=br_white=ffffff\x1b\\");
    backend.advance(b"\x1b]1337;SetColors=bold=ffffff\x07\x1b]1337;SetColors=fg=invalid\x07");
    backend.advance(
            b"\x1b]1337;SetColors=link=112233\x07\x1b]1337;SetColors=curfg=223344\x07\x1b]1337;SetColors=underline=334455\x07\x1b]1337;SetColors=selbg=445566\x07\x1b]1337;SetColors=selfg=778899\x07",
        );
    let colors = backend.render_colors();
    assert_eq!(colors.foreground, [0xff, 0x00, 0xaa]);
    assert_eq!(colors.bold, [0xff, 0xff, 0xff]);
    assert_eq!(colors.background, [0x10, 0x20, 0x30]);
    assert_eq!(colors.cursor, [0xab, 0xcd, 0xef]);
    assert_eq!(colors.ansi[1], [0x11, 0x22, 0x33]);
    assert_eq!(colors.ansi[15], [0xff, 0xff, 0xff]);
    assert_eq!(colors.link, Some([0x11, 0x22, 0x33]));
    assert_eq!(colors.cursor_foreground, Some([0x22, 0x33, 0x44]));
    assert_eq!(colors.underline, Some([0x33, 0x44, 0x55]));
    assert_eq!(colors.selection_background, Some([0x44, 0x55, 0x66]));
    assert_eq!(colors.selection_foreground, Some([0x77, 0x88, 0x99]));
    backend.advance(b"\x1b]21;foreground=?;background=?;cursor=?;1=?;15=?\x1b\\");

    assert_eq!(
            backend.drain_events(),
            vec![TerminalEvent::PtyWrite(
                "\x1b]21;foreground=rgb:ff/00/aa;background=rgb:10/20/30;cursor=rgb:ab/cd/ef;1=rgb:11/22/33;15=rgb:ff/ff/ff\x1b\\"
                    .into()
            )]
        );
}

#[test]
fn applies_registered_iterm_color_presets_atomically() {
    let mut backend = AlacrittyTerminalBackend::new(2, 1);
    let mut ansi = [[0, 0, 0]; 16];
    ansi[1] = [21, 22, 23];
    backend.set_color_presets([(
        "Night Sky".into(),
        TerminalColorPreset {
            foreground: [1, 2, 3],
            background: [4, 5, 6],
            cursor: [7, 8, 9],
            selection: [10, 11, 12],
            ansi,
        },
    )]);
    backend.advance(b"\x1b]1337;SetColors=fg=ffffff\x07\x1b]1337;SetColors=preset=Night Sky\x1b\\");

    let colors = backend.render_colors();
    assert_eq!(colors.foreground, [1, 2, 3]);
    assert_eq!(colors.background, [4, 5, 6]);
    assert_eq!(colors.cursor, [7, 8, 9]);
    assert_eq!(colors.ansi[1], [21, 22, 23]);
    backend.advance(b"\x1b]17;?\x07");
    assert!(matches!(
        &backend.drain_events()[0],
        TerminalEvent::PtyWrite(value) if value.contains("rgb:0a0a/0b0b/0c0c")
    ));

    backend.advance(b"\x1b]1337;SetColors=preset=Missing\x07");
    assert_eq!(backend.render_colors().foreground, [1, 2, 3]);

    backend.advance(b"\x1b]10;#ffffff\x07\x1b]1337;SetProfile=Night Sky\x07");
    assert_eq!(backend.render_colors().foreground, [1, 2, 3]);
}

#[test]
fn parses_osc99_simple_chunked_and_encoded_notifications() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]99;;Simple title\x1b\\");
    backend.advance(b"\x1b]99;i=job:d=0;Build \x1b\\");
    backend.advance(b"\x1b]99;i=job:p=title:d=0;succeeded\x1b\\");
    backend.advance(b"\x1b]99;i=job:p=body:e=1;QWxsIHRlc3RzIHBhc3NlZA\x1b\\");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::Notification {
                id: None,
                title: Some("Simple title".into()),
                body: String::new(),
                occasion: NotificationOccasion::Always,
                urgency: NotificationUrgency::Normal,
                timeout_ms: None,
                sound: NotificationSound::System,
                icon_name: None,
                icon: None,
                buttons: Vec::new(),
                reporting: NotificationReporting::default(),
            },
            TerminalEvent::Notification {
                id: Some("job".into()),
                title: Some("Build succeeded".into()),
                body: "All tests passed".into(),
                occasion: NotificationOccasion::Always,
                urgency: NotificationUrgency::Normal,
                timeout_ms: None,
                sound: NotificationSound::System,
                icon_name: None,
                icon: None,
                buttons: Vec::new(),
                reporting: NotificationReporting::default(),
            },
        ]
    );
}

#[test]
fn preserves_osc99_occasion_urgency_expiry_and_silence_across_chunks() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]99;i=job-1:d=0:o=invisible:u=2:w=5000;Deploy\x1b\\");
    backend.advance(b"\x1b]99;i=job-1:p=body:s=c2lsZW50;Production is ready\x1b\\");
    backend.advance(b"\x1b]99;longkey=value;ignored\x1b\\");
    backend.advance(b"\x1b]99;s=not-base64!;ignored\x1b\\");

    assert_eq!(
        backend.drain_events(),
        vec![TerminalEvent::Notification {
            id: Some("job-1".into()),
            title: Some("Deploy".into()),
            body: "Production is ready".into(),
            occasion: NotificationOccasion::Invisible,
            urgency: NotificationUrgency::Critical,
            timeout_ms: Some(5_000),
            sound: NotificationSound::Silent,
            icon_name: None,
            icon: None,
            buttons: Vec::new(),
            reporting: NotificationReporting::default(),
        }]
    );
}

#[test]
fn parses_bounded_osc99_notification_buttons() {
    use base64::Engine as _;

    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    let buttons = base64::engine::general_purpose::STANDARD.encode("Open\u{2028}Dismiss");
    backend.advance(b"\x1b]99;i=job:d=0;Deploy\x1b\\");
    backend.advance(format!("\x1b]99;i=job:p=buttons:e=1;{buttons}\x1b\\").as_bytes());

    let events = backend.drain_events();
    let [
        TerminalEvent::Notification {
            buttons: parsed_buttons,
            ..
        },
    ] = events.as_slice()
    else {
        panic!("expected one notification");
    };
    assert_eq!(parsed_buttons, &["Open", "Dismiss"]);
}

#[test]
fn preserves_supported_osc99_reporting_requests_across_chunks() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]99;i=job:a=report,-focus:c=1:d=0;Deploy\x1b\\");
    backend.advance(b"\x1b]99;i=job:p=body;Finished\x1b\\");

    let events = backend.drain_events();
    let [TerminalEvent::Notification { reporting, .. }] = events.as_slice() else {
        panic!("expected one notification");
    };
    assert_eq!(
        *reporting,
        NotificationReporting {
            activation: cfg!(any(windows, unix)),
            close: cfg!(any(windows, unix)),
        }
    );
}

#[test]
fn rejects_too_many_or_oversized_osc99_notification_buttons() {
    use base64::Engine as _;

    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    let too_many = base64::engine::general_purpose::STANDARD.encode("1\u{2028}2\u{2028}3\u{2028}4");
    backend.advance(b"\x1b]99;i=many:d=0;Many\x1b\\");
    backend.advance(format!("\x1b]99;i=many:p=buttons:e=1;{too_many}\x1b\\").as_bytes());
    let oversized = base64::engine::general_purpose::STANDARD.encode("x".repeat(129));
    backend.advance(b"\x1b]99;i=large:d=0;Large\x1b\\");
    backend.advance(format!("\x1b]99;i=large:p=buttons:e=1;{oversized}\x1b\\").as_bytes());

    assert!(backend.drain_events().is_empty());
}

#[test]
fn accepts_standard_osc99_named_sounds_on_every_platform() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]99;s=ZXJyb3I=;Build failed\x1b\\");

    let events = backend.drain_events();
    assert_eq!(
        events,
        vec![TerminalEvent::Notification {
            id: None,
            title: Some("Build failed".into()),
            body: String::new(),
            occasion: NotificationOccasion::Always,
            urgency: NotificationUrgency::Normal,
            timeout_ms: None,
            sound: NotificationSound::Error,
            icon_name: None,
            icon: None,
            buttons: Vec::new(),
            reporting: NotificationReporting::default(),
        }]
    );
}

#[test]
fn maps_safe_osc99_named_icons_on_every_platform() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]99;n=ZXJyb3I=;Failure\x1b\\");

    assert_eq!(
        backend.drain_events(),
        vec![TerminalEvent::Notification {
            id: None,
            title: Some("Failure".into()),
            body: String::new(),
            occasion: NotificationOccasion::Always,
            urgency: NotificationUrgency::Normal,
            timeout_ms: None,
            sound: NotificationSound::System,
            icon_name: Some("dialog-error".into()),
            icon: None,
            buttons: Vec::new(),
            reporting: NotificationReporting::default(),
        }]
    );
}

fn encoded_notification_icon(rgba: [u8; 8]) -> (Vec<u8>, String) {
    use base64::Engine as _;
    use image::ImageEncoder as _;

    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&rgba, 2, 1, image::ColorType::Rgba8.into())
        .unwrap();
    let encoded = base64::engine::general_purpose::STANDARD.encode(&png);
    (png, encoded)
}

#[test]
fn decodes_and_reuses_cached_osc99_notification_icons() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    let (_, encoded) = encoded_notification_icon([255, 0, 0, 255, 0, 255, 0, 128]);
    backend.advance(
        format!("\x1b]99;g=logo:p=icon:e=1;{encoded}\x1b\\\x1b]99;i=job:g=logo;Build\x1b\\")
            .as_bytes(),
    );

    let events = backend.drain_events();
    let [
        TerminalEvent::Notification {
            icon: Some(icon), ..
        },
    ] = events.as_slice()
    else {
        panic!("expected one notification with a decoded icon");
    };
    assert_eq!((icon.width, icon.height), (2, 1));
    assert_eq!(icon.rgba.as_ref(), [255, 0, 0, 255, 0, 255, 0, 128]);
}

#[test]
fn accepts_individually_encoded_osc99_icon_chunks() {
    use base64::Engine as _;

    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    let (png, _) = encoded_notification_icon([1, 2, 3, 255, 4, 5, 6, 255]);
    let split = png.len() / 2;
    let first = base64::engine::general_purpose::STANDARD.encode(&png[..split]);
    let second = base64::engine::general_purpose::STANDARD.encode(&png[split..]);
    backend.advance(format!("\x1b]99;g=chunked:p=icon:e=1:d=0;{first}\x1b\\").as_bytes());
    backend.advance(
        format!("\x1b]99;g=chunked:p=icon:e=1;{second}\x1b\\\x1b]99;g=chunked;Ready\x1b\\")
            .as_bytes(),
    );

    let events = backend.drain_events();
    let [
        TerminalEvent::Notification {
            icon: Some(icon), ..
        },
    ] = events.as_slice()
    else {
        panic!("expected one notification with a chunked icon");
    };
    assert_eq!(icon.rgba.as_ref(), [1, 2, 3, 255, 4, 5, 6, 255]);
}

#[test]
fn rejects_invalid_and_clears_cached_osc99_notification_icons() {
    use base64::Engine as _;

    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    let invalid = base64::engine::general_purpose::STANDARD.encode(b"not an image");
    backend.advance(
        format!("\x1b]99;g=bad:p=icon:e=1;{invalid}\x1b\\\x1b]99;g=bad;Ignored\x1b\\").as_bytes(),
    );
    let (_, encoded) = encoded_notification_icon([9, 8, 7, 255, 6, 5, 4, 255]);
    backend.advance(
            format!(
                "\x1b]99;g=clear:p=icon:e=1;{encoded}\x1b\\\x1b]99;g=clear:p=icon:e=1;\x1b\\\x1b]99;g=clear;Cleared\x1b\\"
            )
            .as_bytes(),
        );

    let events = backend.drain_events();
    assert_eq!(events.len(), 2);
    assert!(
        events
            .iter()
            .all(|event| matches!(event, TerminalEvent::Notification { icon: None, .. }))
    );
}

#[test]
fn bounds_the_osc99_notification_icon_cache() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    for index in 0_u8..17 {
        let (_, encoded) = encoded_notification_icon([index, 0, 0, 255, index, 0, 0, 255]);
        backend.advance(format!("\x1b]99;g=icon-{index}:p=icon:e=1;{encoded}\x1b\\").as_bytes());
    }
    backend.advance(b"\x1b]99;g=icon-0;Oldest\x1b\\\x1b]99;g=icon-16;Newest\x1b\\");

    let events = backend.drain_events();
    assert!(matches!(
        &events[0],
        TerminalEvent::Notification { icon: None, .. }
    ));
    assert!(matches!(
        &events[1],
        TerminalEvent::Notification { icon: Some(_), .. }
    ));
}

#[test]
fn answers_osc99_capability_queries_and_rejects_unsafe_payloads() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]99;i=probe:p=?;\x1b\\");
    backend.advance(b"\x1b]99;i=bad id;ignored\x1b\\");
    backend.advance(b"\x1b]99;i=bad/id;ignored\x1b\\");
    backend.advance(b"\x1b]99;e=1;%%%\x1b\\");

    let payload_types = "title,body,close,icon,buttons,alive";
    let sounds = "system,silent,error,warn,warning,info,question";
    let expiry = ":w=1";
    let reports = if cfg!(any(windows, unix)) {
        ":a=report:c=1"
    } else {
        ""
    };
    assert_eq!(
        backend.drain_events(),
        vec![TerminalEvent::PtyWrite(format!(
            "\x1b]99;i=probe:p=?;o=always,unfocused,invisible:p={payload_types}:s={sounds}:u=0,1,2{expiry}{reports}\x1b\\"
        ))]
    );
}

#[test]
fn parses_osc99_explicit_close_requests_with_valid_ids() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]99;i=job:p=title:d=0;Build\x1b\\");
    backend.advance(b"\x1b]99;i=job:p=close;\x1b\\");
    backend.advance(b"\x1b]99;p=close;\x1b\\");

    assert_eq!(
        backend.drain_events(),
        vec![TerminalEvent::NotificationClose("job".into())]
    );
}

#[test]
fn parses_osc99_alive_queries_with_correlation_ids() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]99;i=probe:p=alive;\x1b\\");
    backend.advance(b"\x1b]99;p=alive;\x07");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::NotificationAliveQuery("probe".into()),
            TerminalEvent::NotificationAliveQuery("0".into()),
        ]
    );
}

#[test]
fn enforces_alacritty_minimum_dimensions() {
    let backend = AlacrittyTerminalBackend::new(0, 0);
    let snapshot = backend.snapshot();
    assert_eq!(
        (snapshot.columns, snapshot.rows),
        (MIN_COLUMNS as u16, MIN_SCREEN_LINES as u16)
    );
}

#[test]
fn scrolls_through_saved_history() {
    let mut backend = AlacrittyTerminalBackend::new(10, 2);
    backend.advance(b"one\r\ntwo\r\nthree");
    assert_eq!(backend.snapshot().lines, ["two", "three"]);

    backend.scroll_display(1);
    assert_eq!(backend.snapshot().lines, ["one", "two"]);
    backend.scroll_display(-1);
    assert_eq!(backend.snapshot().lines, ["two", "three"]);
}

#[test]
fn iterm_clear_scrollback_preserves_viewport_and_discards_history() {
    let mut backend = AlacrittyTerminalBackend::new(10, 2);
    backend.advance(b"\x1b]1337;SetMark\x07");
    backend.advance(b"one\r\ntwo\r\nthree");
    assert!(backend.navigate_mark(SearchDirection::Previous));
    assert_eq!(backend.snapshot().lines, ["one", "two"]);

    backend.advance(b"\x1b]1337;ClearScrollback\x1b\\");
    backend.scroll_display(i32::MAX);

    assert_eq!(backend.snapshot().lines, ["two", "three"]);
    assert!(!backend.navigate_mark(SearchDirection::Previous));
}

#[test]
fn scrolls_directly_to_the_bottom_of_saved_history() {
    let mut backend = AlacrittyTerminalBackend::new(10, 2);
    backend.advance(b"one\r\ntwo\r\nthree");
    backend.scroll_display(1);

    backend.scroll_to_bottom();

    assert_eq!(backend.snapshot().lines, ["two", "three"]);
}

#[test]
fn selects_words_from_the_scrollback_viewport() {
    let mut backend = AlacrittyTerminalBackend::new(12, 2);
    backend.advance(b"first line\r\nsecond line\r\nthird line");
    backend.scroll_display(1);

    backend.start_selection(2, 0, SelectionKind::Word);

    assert_eq!(backend.selected_text().as_deref(), Some("first"));
    assert_eq!(backend.snapshot().selection[0].row, 0);
}

#[test]
fn selects_text_on_the_alternate_screen_and_restores_primary_screen() {
    let mut backend = AlacrittyTerminalBackend::new(12, 2);
    backend.advance(b"primary");
    backend.advance(b"\x1b[?1049h\x1b[Halternate");

    backend.start_selection(2, 0, SelectionKind::Word);
    assert_eq!(backend.selected_text().as_deref(), Some("alternate"));

    backend.advance(b"\x1b[?1049l");
    assert_eq!(backend.snapshot().lines[0], "primary");
}

#[test]
fn supports_a_configurable_scrollback_limit() {
    let mut backend = AlacrittyTerminalBackend::with_scrollback(10, 2, 1);
    backend.advance(b"one\r\ntwo\r\nthree\r\nfour");
    backend.scroll_display(i32::MAX);
    assert_eq!(backend.snapshot().lines, ["two", "three"]);
}

#[test]
fn sustained_output_does_not_grow_history_past_the_scrollback_limit() {
    const SCROLLBACK: usize = 32;
    let mut backend = AlacrittyTerminalBackend::with_scrollback(20, 4, SCROLLBACK);
    for line in 0..20_000 {
        backend.advance(format!("line {line}\r\n").as_bytes());
    }

    assert_eq!(backend.terminal.grid().history_size(), SCROLLBACK);
}

#[test]
fn updates_the_scrollback_limit_without_replacing_the_terminal() {
    let mut backend = AlacrittyTerminalBackend::with_scrollback(10, 2, 10);
    backend.advance(b"one\r\ntwo\r\nthree\r\nfour");

    backend.set_scrollback_lines(1);
    backend.scroll_display(i32::MAX);

    assert_eq!(backend.snapshot().lines, ["two", "three"]);
}

#[test]
fn selects_visible_cells_and_extracts_text() {
    let mut backend = AlacrittyTerminalBackend::new(10, 2);
    backend.advance(b"hello world");
    backend.start_selection(1, 0, SelectionKind::Simple);
    backend.update_selection(3, 0);

    assert_eq!(backend.selected_text().as_deref(), Some("ell"));
    assert_eq!(
        backend.snapshot().selection,
        [SelectionSpan {
            row: 0,
            start_column: 1,
            end_column: 3,
        }]
    );
    backend.clear_selection();
    assert_eq!(backend.selected_text(), None);
}

#[test]
fn selection_drag_includes_both_endpoints_in_either_direction() {
    let mut backend = AlacrittyTerminalBackend::new(10, 2);
    backend.advance(b"abcdefghij");

    backend.start_selection(2, 0, SelectionKind::Simple);
    backend.update_selection(5, 0);
    assert_eq!(backend.selected_text().as_deref(), Some("cdef"));

    backend.start_selection(5, 0, SelectionKind::Simple);
    backend.update_selection(2, 0);
    assert_eq!(backend.selected_text().as_deref(), Some("cdef"));

    backend.start_selection(4, 0, SelectionKind::Simple);
    backend.update_selection(4, 0);
    assert_eq!(backend.selected_text().as_deref(), Some("e"));
}

#[test]
fn selection_includes_full_wide_characters_in_either_direction() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance("\u{3042}\u{3044}\u{3046}".as_bytes());

    // Forward selection: start at 'あ' (col 0), update to 'い' (col 2)
    backend.start_selection(0, 0, SelectionKind::Simple);
    assert_eq!(backend.selected_text().as_deref(), Some("\u{3042}"));
    assert_eq!(
        backend.snapshot().selection,
        [SelectionSpan {
            row: 0,
            start_column: 0,
            end_column: 1,
        }]
    );

    backend.update_selection(2, 0);
    assert_eq!(backend.selected_text().as_deref(), Some("\u{3042}\u{3044}"));
    assert_eq!(
        backend.snapshot().selection,
        [SelectionSpan {
            row: 0,
            start_column: 0,
            end_column: 3,
        }]
    );

    // Backward selection: start at 'う' (col 4), update to 'い' (col 2)
    backend.start_selection(4, 0, SelectionKind::Simple);
    assert_eq!(backend.selected_text().as_deref(), Some("\u{3046}"));
    assert_eq!(
        backend.snapshot().selection,
        [SelectionSpan {
            row: 0,
            start_column: 4,
            end_column: 5,
        }]
    );

    backend.update_selection(2, 0);
    assert_eq!(backend.selected_text().as_deref(), Some("\u{3044}\u{3046}"));
    assert_eq!(
        backend.snapshot().selection,
        [SelectionSpan {
            row: 0,
            start_column: 2,
            end_column: 5,
        }]
    );
}

#[test]
fn simple_selection_marks_the_anchor_and_spans_multiple_rows_exactly() {
    let mut backend = AlacrittyTerminalBackend::new(8, 3);
    backend.advance(b"abcdef\r\nghijkl\r\nmnopqr");

    backend.start_selection(2, 0, SelectionKind::Simple);
    assert_eq!(
        backend.snapshot().selection,
        [SelectionSpan {
            row: 0,
            start_column: 2,
            end_column: 2,
        }]
    );

    backend.update_selection(3, 2);
    assert_eq!(
        backend.snapshot().selection,
        [
            SelectionSpan {
                row: 0,
                start_column: 2,
                end_column: 7,
            },
            SelectionSpan {
                row: 1,
                start_column: 0,
                end_column: 7,
            },
            SelectionSpan {
                row: 2,
                start_column: 0,
                end_column: 3,
            },
        ]
    );

    backend.start_selection(3, 2, SelectionKind::Simple);
    backend.update_selection(2, 0);
    assert_eq!(
        backend.snapshot().selection,
        [
            SelectionSpan {
                row: 0,
                start_column: 2,
                end_column: 7,
            },
            SelectionSpan {
                row: 1,
                start_column: 0,
                end_column: 7,
            },
            SelectionSpan {
                row: 2,
                start_column: 0,
                end_column: 3,
            },
        ]
    );
}

#[test]
fn selects_words_and_lines() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"alpha beta\r\nsecond line");

    backend.start_selection(7, 0, SelectionKind::Word);
    assert_eq!(backend.selected_text().as_deref(), Some("beta"));

    backend.start_selection(3, 1, SelectionKind::Line);
    assert_eq!(backend.selected_text().as_deref(), Some("second line\n"));
}

#[test]
fn exposes_title_cwd_command_and_bell_terminal_events() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]0;build server\x07");
    backend.advance(b"\x1b]7;file://localhost/srv/my%20app");
    backend.advance(
        b"\x1b\\\x1b]133;A\x1b\\\x1b]133;B;aid=7\x07\x1b]133;C\x1b\\\x1b]133;D;17\x07\x07",
    );

    let events = backend.drain_events();
    assert!(events.contains(&TerminalEvent::TitleChanged("build server".into())));
    assert!(events.contains(&TerminalEvent::CwdChanged("/srv/my app".into())));
    assert!(events.contains(&TerminalEvent::PromptStarted));
    assert!(events.contains(&TerminalEvent::CommandLineStarted));
    assert!(events.contains(&TerminalEvent::CommandStarted));
    assert!(events.contains(&TerminalEvent::CommandFinished(Some(17))));
    assert!(events.contains(&TerminalEvent::Bell { visual_bell: None }));
    assert!(backend.drain_events().is_empty());
}

#[test]
fn captures_the_visual_bell_color_at_bell_time() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]21;visual_bell=#123456\x1b\\\x07\x1b]21;visual_bell\x1b\\");

    assert_eq!(
        backend.drain_events(),
        vec![TerminalEvent::Bell {
            visual_bell: Some([0x12, 0x34, 0x56]),
        }]
    );
    assert_eq!(backend.render_colors().visual_bell, None);
}

#[test]
fn exposes_bounded_icon_title_metadata() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]1;build icon\x1b\\");
    backend.advance(b"\x1b]1;bad\ntitle\x07");
    let oversized = format!("\x1b]1;{}\x07", "x".repeat(MAX_OSC_ICON_TITLE_BYTES + 1));
    backend.advance(oversized.as_bytes());

    assert_eq!(
        backend.drain_events(),
        vec![TerminalEvent::IconTitleChanged("build icon".into())]
    );
}

#[test]
fn reports_cursor_position_to_the_pty() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"hello\x1b[6n");

    assert_eq!(
        backend.drain_events(),
        vec![TerminalEvent::PtyWrite("\x1b[1;6R".into())]
    );
}

#[test]
fn accepts_command_end_without_status_and_ignores_invalid_status() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]133;D\x07\x1b]133;D;invalid\x1b\\\x1b]133;DX\x07");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::CommandFinished(None),
            TerminalEvent::CommandFinished(None),
        ]
    );
}

#[test]
fn rejects_malformed_osc133_prompt_markers() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]133;AX\x07\x1b]133;BX\x1b\\");

    assert!(backend.drain_events().is_empty());
}

#[test]
fn accepts_bounded_iterm_current_directory_reports() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]1337;CurrentDir=/srv/my project\x1b\\");
    backend.advance(b"\x1b]1337;CurrentDir=\x07");

    assert_eq!(
        backend.drain_events(),
        vec![TerminalEvent::CwdChanged("/srv/my project".into())]
    );
}

#[test]
fn normalizes_windows_drive_paths_for_osc7_and_current_dir() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]7;file:///C%3A/Users/my%20app\x1b\\");
    backend.advance(b"\x1b]7;file://localhost/D:/work/repo\x1b\\");
    backend.advance(b"\x1b]7;file:///c:/users/lower\x1b\\");
    backend.advance(b"\x1b]7;file:///E|/legacy/drive\x1b\\");
    backend.advance(b"\x1b]1337;CurrentDir=/F:/iterm/current\x1b\\");
    backend.advance(b"\x1b]1337;CurrentDir=G:\\native\\windows\\path\x1b\\");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::CwdChanged("C:/Users/my app".into()),
            TerminalEvent::CwdChanged("D:/work/repo".into()),
            TerminalEvent::CwdChanged("c:/users/lower".into()),
            TerminalEvent::CwdChanged("E:/legacy/drive".into()),
            TerminalEvent::CwdChanged("F:/iterm/current".into()),
            TerminalEvent::CwdChanged("G:\\native\\windows\\path".into()),
        ]
    );
}

#[test]
fn accepts_bounded_iterm_remote_host_reports() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]1337;RemoteHost=alice@build.example.com\x1b\\");
    backend.advance(b"\x1b]1337;RemoteHost=@host-only.example.com\x07");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::RemoteHostChanged("alice@build.example.com".into()),
            TerminalEvent::RemoteHostChanged("@host-only.example.com".into()),
        ]
    );
}

#[test]
fn parses_bounded_iterm_shell_integration_versions() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]1337;ShellIntegrationVersion=12;fish\x1b\\");
    backend.advance(b"\x1b]1337;ShellIntegrationVersion=7\x07");
    backend.advance(b"\x1b]1337;ShellIntegrationVersion=bad;zsh\x07");
    backend.advance(b"\x1b]1337;ShellIntegrationVersion=3;bad;shell\x07");
    let oversized = format!(
        "\x1b]1337;ShellIntegrationVersion=3;{}\x07",
        "s".repeat(MAX_OSC_SHELL_NAME_BYTES + 1)
    );
    backend.advance(oversized.as_bytes());

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::ShellIntegrationChanged {
                version: 12,
                shell: Some("fish".into()),
            },
            TerminalEvent::ShellIntegrationChanged {
                version: 7,
                shell: None,
            },
        ]
    );
}

#[test]
fn parses_bounded_iterm_variable_queries() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]1337;ReportVariable=c2Vzc2lvbi5uYW1l\x1b\\");
    backend.advance(b"\x1b]1337;ReportVariable=c2Vzc2lvbi51c2VyLmdpdEJyYW5jaA==\x07");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::ItermVariableQuery("session.name".into()),
            TerminalEvent::ItermVariableQuery("session.user.gitBranch".into()),
        ]
    );
}

#[test]
fn parses_fragmented_iterm_variable_queries() {
    let sequence = b"\x1b]1337;ReportVariable=c2Vzc2lvbi5wYXRo\x1b\\";
    for split in 0..=sequence.len() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(&sequence[..split]);
        backend.advance(&sequence[split..]);
        assert_eq!(
            backend.drain_events(),
            vec![TerminalEvent::ItermVariableQuery("session.path".into())],
            "split at {split}"
        );
    }
}

#[test]
fn rejects_malformed_iterm_variable_queries() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]1337;ReportVariable=\x07");
    backend.advance(b"\x1b]1337;ReportVariable=not-base64!\x07");
    backend.advance(b"\x1b]1337;ReportVariable=bGluZQpicmVhaw==\x07");
    let oversized = base64::engine::general_purpose::STANDARD.encode(vec![
        b'x';
        MAX_OSC_REPORT_VARIABLE_NAME_BYTES
            + 1
    ]);
    backend.advance(format!("\x1b]1337;ReportVariable={oversized}\x07").as_bytes());

    assert!(backend.drain_events().is_empty());
}

#[test]
fn parses_bounded_iterm_badge_formats() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    let format = base64::engine::general_purpose::STANDARD
        .encode(r"build \(session.name) \(user.gitBranch)");
    backend.advance(format!("\x1b]1337;SetBadgeFormat={format}\x1b\\").as_bytes());
    backend.advance(b"\x1b]1337;SetBadgeFormat=\x07");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::ItermBadgeFormatChanged(
                r"build \(session.name) \(user.gitBranch)".into()
            ),
            TerminalEvent::ItermBadgeFormatChanged(String::new()),
        ]
    );
}

#[test]
fn rejects_malformed_iterm_badge_formats() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]1337;SetBadgeFormat=not-base64!\x07");
    backend.advance(b"\x1b]1337;SetBadgeFormat=bGluZQpicmVhaw==\x07");
    let oversized =
        base64::engine::general_purpose::STANDARD
            .encode(vec![b'x'; MAX_OSC_BADGE_FORMAT_BYTES + 1]);
    backend.advance(format!("\x1b]1337;SetBadgeFormat={oversized}\x07").as_bytes());

    assert!(backend.drain_events().is_empty());
}

#[test]
fn parses_iterm_cursor_line_highlight_toggles() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]1337;HighlightCursorLine=yes\x1b\\");
    backend.advance(b"\x1b]1337;HighlightCursorLine=no\x07");
    backend.advance(b"\x1b]1337;HighlightCursorLine=true\x07");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::CursorLineHighlightChanged(true),
            TerminalEvent::CursorLineHighlightChanged(false),
        ]
    );
}

#[test]
fn parses_iterm_attention_requests_including_cursor_fireworks() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]1337;RequestAttention=yes\x1b\\");
    backend.advance(b"\x1b]1337;RequestAttention=once\x07");
    backend.advance(b"\x1b]1337;RequestAttention=no\x07");
    backend.advance(b"\x1b]1337;RequestAttention=fireworks\x07");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::AttentionRequested(TerminalAttention::Indefinite),
            TerminalEvent::AttentionRequested(TerminalAttention::Once),
            TerminalEvent::AttentionRequested(TerminalAttention::Cancel),
            TerminalEvent::AttentionRequested(TerminalAttention::Fireworks),
        ]
    );
}

#[test]
fn parses_iterm_focus_requests() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]1337;StealFocus\x1b\\");
    backend.advance(b"\x1b]1337;Disinter\x07");
    backend.advance(b"\x1b]1337;StealFocus=yes\x07");
    backend.advance(b"\x1b]1337;Disinter=1\x07");

    assert_eq!(
        backend.drain_events(),
        vec![TerminalEvent::FocusRequested, TerminalEvent::FocusRequested]
    );
}

#[test]
fn parses_bounded_iterm_open_url_requests() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    let url = base64::engine::general_purpose::STANDARD.encode("https://example.com/docs");
    backend.advance(format!("\x1b]1337;OpenURL=:{url}\x1b\\").as_bytes());
    backend.advance(b"\x1b]1337;OpenURL=:not-base64!\x07");
    let control = base64::engine::general_purpose::STANDARD.encode("https://example.com\ncmd");
    backend.advance(format!("\x1b]1337;OpenURL=:{control}\x07").as_bytes());
    let oversized =
        base64::engine::general_purpose::STANDARD.encode(vec![b'x'; MAX_OSC_URL_BYTES + 1]);
    backend.advance(format!("\x1b]1337;OpenURL=:{oversized}\x07").as_bytes());

    assert_eq!(
        backend.drain_events(),
        vec![TerminalEvent::OpenUrlRequested(
            "https://example.com/docs".into()
        )]
    );
}

#[test]
fn parses_bounded_iterm_background_image_requests_and_clear() {
    let mut backend = AlacrittyTerminalBackend::new(2, 1);
    backend.advance(
        b"\x1b]1337;SetBackgroundImageFile=L3dhbGxwYXBlci5wbmc=\x07\x1b]1337;SetBackgroundImageFile=\x1b\\",
    );
    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::BackgroundImageRequested(Some("/wallpaper.png".into())),
            TerminalEvent::BackgroundImageRequested(None),
        ]
    );

    backend.advance(b"\x1b]1337;SetBackgroundImageFile=not-base64!\x07");
    assert!(backend.drain_events().is_empty());
}

#[test]
fn permits_bounded_iterm_clipboard_copies_only_when_enabled() {
    let first = base64::engine::general_purpose::STANDARD.encode("first");
    let second = base64::engine::general_purpose::STANDARD_NO_PAD.encode("second");
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(format!("\x1b]1337;Copy=:{first}\x1b\\").as_bytes());
    assert!(backend.drain_events().is_empty());

    backend.set_osc52_copy_enabled(true);
    backend.advance(format!("\x1b]1337;Copy=:{first}\x1b\\").as_bytes());
    backend.advance(format!("\x1b]1337;Copy=:{second}\x07").as_bytes());
    backend.advance(b"\x1b]1337;Copy=:not-base64!\x07");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::ClipboardStore("first".into()),
            TerminalEvent::ClipboardStore("second".into()),
        ]
    );

    let oversized =
        base64::engine::general_purpose::STANDARD.encode(vec![b'x'; MAX_OSC52_COPY_BYTES + 1]);
    backend.advance(format!("\x1b]1337;Copy=:{oversized}\x07").as_bytes());
    assert!(backend.drain_events().is_empty());
}

#[test]
fn captures_bounded_iterm_clipboard_text_across_fragments() {
    let sequence = b"\x1b]1337;CopyToClipboard=\x07plain \x1b[31mred\x1b[0m\r\n\twide \xf0\x9f\x99\x82\x1b]1337;EndCopy\x1b\\";
    for split in 0..=sequence.len() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.set_osc52_copy_enabled(true);
        backend.advance(&sequence[..split]);
        backend.advance(&sequence[split..]);
        assert_eq!(
            backend.drain_events(),
            vec![TerminalEvent::ClipboardStore(
                "plain red\r\n\twide 🙂".into()
            )],
            "split at {split}"
        );
    }
}

#[test]
fn discards_oversized_or_cancelled_iterm_clipboard_captures() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.set_osc52_copy_enabled(true);
    backend.advance(b"\x1b]1337;CopyToClipboard=find\x07ignored\x1b]1337;EndCopy\x07");
    assert!(backend.drain_events().is_empty());

    backend.advance(b"\x1b]1337;CopyToClipboard=\x07");
    backend.advance(&vec![b'x'; MAX_OSC52_COPY_BYTES + 1]);
    backend.advance(b"\x1b]1337;EndCopy\x07");
    assert!(backend.drain_events().is_empty());

    backend.advance(b"\x1b]1337;CopyToClipboard=\x07partial");
    backend.set_osc52_copy_enabled(false);
    backend.advance(b"\x1b]1337;EndCopy\x07");
    assert!(backend.drain_events().is_empty());
}

#[test]
fn rejects_malformed_iterm_remote_host_reports() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]1337;RemoteHost=no-separator\x07");
    backend.advance(b"\x1b]1337;RemoteHost=alice@\x07");
    backend.advance(b"\x1b]1337;RemoteHost=alice@host\nname\x07");
    let oversized = format!("\x1b]1337;RemoteHost=user@{}\x07", "h".repeat(1024));
    backend.advance(oversized.as_bytes());

    assert!(backend.drain_events().is_empty());
}

#[test]
fn parses_bounded_iterm_user_variables() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]1337;SetUserVar=gitBranch=bWFpbg==\x1b\\");
    backend.advance(b"\x1b]1337;SetUserVar=empty=\x07");

    assert_eq!(
        backend.drain_events(),
        vec![
            TerminalEvent::UserVarChanged {
                name: "gitBranch".into(),
                value: "main".into(),
            },
            TerminalEvent::UserVarChanged {
                name: "empty".into(),
                value: String::new(),
            },
        ]
    );
}

#[test]
fn rejects_malformed_iterm_user_variables() {
    let mut backend = AlacrittyTerminalBackend::new(20, 2);
    backend.advance(b"\x1b]1337;SetUserVar==bWFpbg==\x07");
    backend.advance(b"\x1b]1337;SetUserVar=bad\nname=bWFpbg==\x07");
    backend.advance(b"\x1b]1337;SetUserVar=name=not-base64!\x07");
    let oversized = base64::engine::general_purpose::STANDARD.encode(vec![b'x'; 4097]);
    backend.advance(format!("\x1b]1337;SetUserVar=name={oversized}\x07").as_bytes());

    assert!(backend.drain_events().is_empty());
}

#[test]
fn navigates_osc133_prompts_in_scrollback_order() {
    let mut backend = AlacrittyTerminalBackend::with_scrollback(20, 3, 20);
    backend.advance(
            b"\x1b]133;A\x1b\\prompt-one\r\nout-1\r\nout-2\r\n\x1b]133;A\x07prompt-two\r\ntail-1\r\ntail-2\r\ntail-3",
        );

    assert!(backend.navigate_prompt(SearchDirection::Previous));
    assert!(backend.visible_text().contains("prompt-two"));
    assert!(backend.navigate_prompt(SearchDirection::Previous));
    assert!(backend.visible_text().contains("prompt-one"));
    assert!(backend.navigate_prompt(SearchDirection::Next));
    assert!(backend.visible_text().contains("prompt-two"));

    backend.resize(21, 3);
    assert!(!backend.navigate_prompt(SearchDirection::Previous));
}

#[test]
fn navigates_iterm_set_marks_in_scrollback_order() {
    let mut backend = AlacrittyTerminalBackend::with_scrollback(20, 3, 20);
    backend.advance(
            b"\x1b]1337;SetMark\x1b\\mark-one\r\nout-1\r\nout-2\r\n\x1b]1337;SetMark\x07mark-two\r\ntail-1\r\ntail-2\r\ntail-3",
        );
    backend.advance(b"\x1b]1337;SetMark=invalid\x07");

    assert!(backend.navigate_mark(SearchDirection::Previous));
    assert!(backend.visible_text().contains("mark-two"));
    assert!(backend.navigate_mark(SearchDirection::Previous));
    assert!(backend.visible_text().contains("mark-one"));
    assert!(backend.navigate_mark(SearchDirection::Next));
    assert!(backend.visible_text().contains("mark-two"));

    backend.resize(21, 3);
    assert!(!backend.navigate_mark(SearchDirection::Previous));
}

#[test]
fn selects_the_last_completed_osc133_command_output() {
    let mut backend = AlacrittyTerminalBackend::with_scrollback(20, 3, 20);
    backend.advance(
        b"older\r\n\x1b]133;C\x1b\\output-a\r\noutput-b\r\n\x1b]133;D;0\x07\x1b]133;A\x07prompt",
    );

    assert!(backend.select_last_command_output());
    let selected = backend.selected_text().unwrap();
    assert!(selected.contains("output-a"), "{selected:?}");
    assert!(selected.contains("output-b"), "{selected:?}");
    assert!(!selected.contains("older"), "{selected:?}");
    assert!(!selected.contains("prompt"), "{selected:?}");
}

#[test]
fn exposes_completed_osc133_command_zones_with_exit_status() {
    let mut backend = AlacrittyTerminalBackend::new(20, 5);
    backend.advance(b"\x1b]133;C\x1b\\output-a\r\noutput-b\x1b]133;D;7\x07");

    assert_eq!(
        backend.snapshot().command_zones,
        [CommandZoneSpan {
            start_row: 0,
            end_row: 1,
            exit_status: Some(7),
        }]
    );
}

#[test]
fn iterm_clear_captured_output_removes_only_command_ranges() {
    let mut backend = AlacrittyTerminalBackend::new(20, 5);
    backend.advance(
            b"\x1b]1337;SetMark\x07\x1b]133;C\x1b\\output\x1b]133;D;0\x07\x1b]1337;ClearCapturedOutput\x1b\\",
        );

    assert!(backend.snapshot().command_zones.is_empty());
    assert!(!backend.select_last_command_output());
    assert!(backend.navigate_mark(SearchDirection::Previous));
    assert!(
        backend
            .drain_events()
            .contains(&TerminalEvent::CapturedOutputCleared)
    );
}

#[test]
fn cycles_selection_across_completed_osc133_command_outputs() {
    let mut backend = AlacrittyTerminalBackend::new(20, 5);
    backend.advance(
        b"\x1b]133;C\x1b\\first\x1b]133;D;0\x1b\\\r\n\x1b]133;C\x1b\\second\x1b]133;D;1\x1b\\",
    );

    assert!(backend.select_command_output(SearchDirection::Next));
    assert_eq!(backend.selected_text().as_deref(), Some("first"));
    assert!(backend.select_command_output(SearchDirection::Next));
    assert_eq!(backend.selected_text().as_deref(), Some("second"));
    assert!(backend.select_command_output(SearchDirection::Previous));
    assert_eq!(backend.selected_text().as_deref(), Some("first"));
}
