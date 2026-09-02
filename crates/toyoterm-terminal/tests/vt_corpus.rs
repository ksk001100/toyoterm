use toyoterm_terminal::{AlacrittyTerminalBackend, CellAttributes, TerminalBackend, TerminalCell};

struct CorpusCase {
    name: &'static str,
    columns: u16,
    rows: u16,
    chunks: &'static [&'static [u8]],
    expected: &'static str,
}

static UNICODE_CHUNKS: [&[u8]; 2] = [
    "ASCII 界 e".as_bytes(),
    "\u{301} 😀 👩\u{200d}💻".as_bytes(),
];

#[test]
fn vt_input_corpus_matches_terminal_snapshots() {
    let cases = [
        CorpusCase {
            name: "real_app_output",
            columns: 32,
            rows: 4,
            chunks: &[
                b"\x1b]0;cargo test\x07\x1b[?25l\r\x1b[2K",
                b"\x1b[1;32m   Compiling\x1b[0m toyoterm v0.0.1\r\n",
                b"\x1b[1;32m    Finished\x1b[0m test profile\r\n\x1b[?25h$ ",
            ],
            expected: r#"lines=["   Compiling toyoterm v0.0.1", "    Finished test profile", "$", ""]
special_cells=[]
styled_runs=["0:0..12=\"   Compiling\" fg=Indexed(2),bold", "1:0..12=\"    Finished\" fg=Indexed(2),bold"]
cursor=CursorState { column: 2, row: 2, visible: true, shape: Block }
mode=TerminalMode { application_cursor: false, application_keypad: false, bracketed_paste: false, mouse_reporting: false, sgr_mouse: false, focus_reporting: false, alternate_screen: false, alternate_scroll: true }
events=[TitleChanged("cargo test")]"#,
        },
        CorpusCase {
            name: "alternate_screen",
            columns: 20,
            rows: 4,
            chunks: &[
                b"primary shell prompt",
                b"\x1b[?1049h\x1b[2J\x1b[Hmenu item 1\r\nmenu item 2",
                b"\x1b[1;1H>\x1b[?25l",
            ],
            expected: r#"lines=[">enu item 1", "menu item 2", "", ""]
special_cells=[]
styled_runs=[]
cursor=CursorState { column: 1, row: 0, visible: false, shape: Block }
mode=TerminalMode { application_cursor: false, application_keypad: false, bracketed_paste: false, mouse_reporting: false, sgr_mouse: false, focus_reporting: false, alternate_screen: true, alternate_scroll: true }
events=[]"#,
        },
        CorpusCase {
            name: "wide_combining_and_emoji",
            columns: 20,
            rows: 3,
            chunks: &UNICODE_CHUNKS,
            expected: r#"lines=["ASCII 界 e\u{301} 😀 👩\u{200d}💻", "", ""]
special_cells=["0:6=\"界\"/w2", "0:9=\"e\\u{301}\"/w1", "0:11=\"😀\"/w2", "0:14=\"👩\\u{200d}\"/w2", "0:16=\"💻\"/w2"]
styled_runs=[]
cursor=CursorState { column: 18, row: 0, visible: true, shape: Block }
mode=TerminalMode { application_cursor: false, application_keypad: false, bracketed_paste: false, mouse_reporting: false, sgr_mouse: false, focus_reporting: false, alternate_screen: false, alternate_scroll: true }
events=[]"#,
        },
        CorpusCase {
            name: "dec_private_modes",
            columns: 12,
            rows: 4,
            chunks: &[
                b"\x1b[?1h\x1b=\x1b[?7l\x1b[?1000h\x1b[?1006h\x1b[?1004h\x1b[?2004h",
                b"\x1b[2;4r\x1b[?6h\x1b[Horigin",
            ],
            expected: r#"lines=["", "origin", "", ""]
special_cells=[]
styled_runs=[]
cursor=CursorState { column: 6, row: 1, visible: true, shape: Block }
mode=TerminalMode { application_cursor: true, application_keypad: true, bracketed_paste: true, mouse_reporting: true, sgr_mouse: true, focus_reporting: true, alternate_screen: false, alternate_scroll: true }
events=[]"#,
        },
        CorpusCase {
            name: "malformed_and_chunked_sequences",
            columns: 24,
            rows: 3,
            chunks: &[
                b"before\x1b[?9999h",
                b"\x1b[999999999999999999999999999999mstill ",
                b"\xf0\x9f",
                b"\x98\x80 after\x1b[38;2;255",
                b";0;0m!\x1b[0m",
            ],
            expected: r#"lines=["beforestill 😀 after!", "", ""]
special_cells=["0:12=\"😀\"/w2"]
styled_runs=["0:20..21=\"!\" fg=Rgb(255, 0, 0)"]
cursor=CursorState { column: 21, row: 0, visible: true, shape: Block }
mode=TerminalMode { application_cursor: false, application_keypad: false, bracketed_paste: false, mouse_reporting: false, sgr_mouse: false, focus_reporting: false, alternate_screen: false, alternate_scroll: true }
events=[]"#,
        },
    ];

    let mut failures = Vec::new();
    for case in cases {
        let mut backend = AlacrittyTerminalBackend::new(case.columns, case.rows);
        for chunk in case.chunks {
            backend.advance(chunk);
        }
        let actual = corpus_snapshot(&mut backend);
        if actual != case.expected {
            failures.push(format!("{}:\n{actual}", case.name));
        }
    }

    assert!(
        failures.is_empty(),
        "VT corpus mismatches:\n\n{}",
        failures.join("\n\n")
    );
}

fn corpus_snapshot(backend: &mut AlacrittyTerminalBackend) -> String {
    let snapshot = backend.snapshot();
    let special_cells = snapshot
        .cells
        .iter()
        .enumerate()
        .flat_map(|(row, cells)| {
            cells
                .iter()
                .filter(|cell| cell.width != 1 || !cell.text.is_ascii() || cell.hyperlink.is_some())
                .map(move |cell| {
                    format!(
                        "{row}:{}={:?}/w{}{}",
                        cell.column,
                        cell.text,
                        cell.width,
                        cell.hyperlink
                            .as_ref()
                            .map_or_else(String::new, |link| format!(" -> {link}"))
                    )
                })
        })
        .collect::<Vec<_>>();
    let styled_runs = snapshot
        .cells
        .iter()
        .enumerate()
        .flat_map(|(row, cells)| styled_runs(row, cells))
        .collect::<Vec<_>>();

    format!(
        "lines={:?}\nspecial_cells={special_cells:?}\nstyled_runs={styled_runs:?}\ncursor={:?}\nmode={:?}\nevents={:?}",
        snapshot.lines,
        backend.cursor(),
        backend.mode(),
        backend.drain_events()
    )
}

fn styled_runs(row: usize, cells: &[TerminalCell]) -> Vec<String> {
    let mut runs = Vec::new();
    let mut start = 0;
    while start < cells.len() {
        let attributes = cells[start].attributes;
        let mut end = start + 1;
        while end < cells.len()
            && cells[end].column == cells[end - 1].column + u16::from(cells[end - 1].width)
            && cells[end].attributes == attributes
        {
            end += 1;
        }
        if attributes != CellAttributes::default() {
            let text = cells[start..end]
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<String>();
            runs.push(format!(
                "{row}:{}..{}={text:?} {}",
                cells[start].column,
                cells[end - 1].column + u16::from(cells[end - 1].width),
                compact_attributes(attributes)
            ));
        }
        start = end;
    }
    runs
}

fn compact_attributes(attributes: CellAttributes) -> String {
    let defaults = CellAttributes::default();
    let mut parts = Vec::new();
    if attributes.foreground != defaults.foreground {
        parts.push(format!("fg={:?}", attributes.foreground));
    }
    if attributes.background != defaults.background {
        parts.push(format!("bg={:?}", attributes.background));
    }
    for (enabled, name) in [
        (attributes.bold, "bold"),
        (attributes.italic, "italic"),
        (attributes.underline, "underline"),
        (attributes.strikethrough, "strikethrough"),
        (attributes.dim, "dim"),
        (attributes.inverse, "inverse"),
        (attributes.hidden, "hidden"),
    ] {
        if enabled {
            parts.push(name.to_owned());
        }
    }
    parts.join(",")
}
