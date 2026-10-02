//! Shared public-boundary harness, also replayed by normal terminal tests.
use std::hint::black_box;
use toyoterm_terminal::{AlacrittyTerminalBackend, TerminalBackend};

#[derive(Clone, Copy)]
pub enum Target {
    RawTerminal,
    Osc,
    KittyGraphics,
    Sixel,
    ItermImage,
    KittyFileTransfer,
}

fn terminal() -> AlacrittyTerminalBackend {
    let mut terminal = AlacrittyTerminalBackend::with_scrollback(80, 24, 64);
    terminal.set_cell_size(8, 16);
    terminal.set_osc52_copy_enabled(true);
    // These only enable parser/state-machine events. Never apply the events to
    // an app, PTY, filesystem, clipboard, shell, or notification service.
    terminal.set_osc_file_download_enabled(true);
    terminal.set_osc_file_upload_enabled(true);
    terminal
}

fn inspect(terminal: &mut AlacrittyTerminalBackend) {
    let snapshot = terminal.snapshot();
    assert_eq!(terminal.dimensions(), (snapshot.columns, snapshot.rows));
    assert_eq!(snapshot.cells.len(), usize::from(snapshot.rows));
    let cursor = terminal.cursor();
    assert!(cursor.column < snapshot.columns);
    assert!(cursor.row < snapshot.rows);
    for row in &snapshot.cells {
        for cell in row {
            assert!(cell.column < snapshot.columns);
            black_box((
                &cell.text,
                &cell.attributes,
                &cell.hyperlink,
                cell.text_size,
            ));
        }
    }
    for image in &snapshot.images {
        let length = usize::try_from(image.width)
            .ok()
            .and_then(|width| width.checked_mul(usize::try_from(image.height).ok()?))
            .and_then(|pixels| pixels.checked_mul(4));
        assert_eq!(length, Some(image.rgba.len()));
        black_box(image.rgba.as_ref());
    }
    black_box(snapshot);
    black_box(terminal.visible_text());
    black_box(terminal.render_colors());
    black_box(terminal.mode());
    for event in terminal.drain_events() {
        black_box(event);
    }
}

fn feed(terminal: &mut AlacrittyTerminalBackend, bytes: &[u8], chunk_size: usize) {
    for chunk in bytes.chunks(chunk_size) {
        terminal.advance(chunk);
    }
}

fn recover(terminal: &mut AlacrittyTerminalBackend) {
    // Observe pending/unterminated state first, then exercise cancellation, RIS,
    // and an ordinary subsequent stream. No panic catching or input filtering.
    inspect(terminal);
    terminal.advance(b"\x18\x1b\\\x1b[c\x1bcafter reset\r\n");
    terminal.stop_synchronized_update();
    inspect(terminal);
}

pub fn run(target: Target, data: &[u8]) {
    let chunk_size = 1 + usize::from(data.first().copied().unwrap_or(0) % 64);
    // Every target accepts complete wire streams unchanged, including multi-
    // message sessions and arbitrary bytes outside its preferred protocol.
    for size in [data.len().max(1), chunk_size] {
        let mut terminal = terminal();
        feed(&mut terminal, data, size);
        recover(&mut terminal);
    }
    if matches!(target, Target::RawTerminal) {
        return;
    }

    // Newline-separated payload records make mutations reach protocol handlers
    // without first having to discover framing. Keep state across records.
    let mut terminal = terminal();
    for record in data.split(|byte| *byte == b'\n') {
        let prefix: &[u8] = match target {
            Target::RawTerminal => unreachable!(),
            Target::Osc => {
                const PREFIXES: [&[u8]; 14] = [
                    b"\x1b]0;",
                    b"\x1b]2;",
                    b"\x1b]4;",
                    b"\x1b]7;",
                    b"\x1b]8;",
                    b"\x1b]9;",
                    b"\x1b]21;",
                    b"\x1b]50;",
                    b"\x1b]52;",
                    b"\x1b]66;",
                    b"\x1b]99;",
                    b"\x1b]133;",
                    b"\x1b]1337;",
                    b"\x1b]21337;",
                ];
                PREFIXES[usize::from(record.first().copied().unwrap_or(0)) % PREFIXES.len()]
            }
            Target::KittyGraphics => b"\x1b_G",
            Target::Sixel => b"\x1bP0;1q",
            Target::ItermImage => b"\x1b]1337;",
            Target::KittyFileTransfer => b"\x1b]5113;",
        };
        terminal.advance(prefix);
        feed(&mut terminal, record, chunk_size);
        terminal.advance(b"\x1b\\");
    }
    recover(&mut terminal);
}
