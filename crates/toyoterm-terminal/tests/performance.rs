//! Manual measurements of the native terminal hot path. Run with --release and --nocapture.

use std::hint::black_box;
use std::time::Instant;

use toyoterm_terminal::{AlacrittyTerminalBackend, SearchDirection, TerminalBackend};

const SAMPLES: usize = 5;

fn measure(
    name: &str,
    iterations: usize,
    bytes_per_iteration: Option<usize>,
    mut run: impl FnMut(),
) {
    // Warm the code and allocations before collecting samples. No wall-clock
    // assertion: results depend on the host, build profile, and system load.
    for _ in 0..iterations {
        run();
    }
    let mut samples = [0_u128; SAMPLES];
    for sample in &mut samples {
        let start = Instant::now();
        for _ in 0..iterations {
            run();
        }
        *sample = start.elapsed().as_nanos();
    }
    samples.sort_unstable();
    let median_ns = samples[SAMPLES / 2] as f64 / iterations as f64;
    if let Some(bytes) = bytes_per_iteration {
        let mib_per_second = bytes as f64 * 1e9 / median_ns / (1024.0 * 1024.0);
        eprintln!("{name}: {median_ns:.0} ns/op, {mib_per_second:.1} MiB/s");
    } else {
        eprintln!("{name}: {median_ns:.0} ns/op");
    }
}

#[test]
#[ignore = "manual performance measurement"]
fn vt_input_plain_and_styled() {
    let plain = "plain terminal output with a fixed width payload 0123456789\r\n".repeat(80);
    let styled = "\x1b[1;32mcolored\x1b[0m output 界 😀 0123456789\r\n".repeat(80);
    for (name, input) in [("vt_plain", plain), ("vt_styled_unicode", styled)] {
        let mut backend = AlacrittyTerminalBackend::with_scrollback(100, 40, 2000);
        measure(name, 100, Some(input.len()), || {
            backend.advance(black_box(input.as_bytes()));
            black_box(backend.cursor());
        });
        assert!(backend.visible_text().contains("0123456789"));
    }
}

#[test]
#[ignore = "manual performance measurement"]
fn snapshots_plain_and_urls() {
    for (name, line) in [
        (
            "snapshot_plain",
            "ordinary text and numbers 0123456789 ".repeat(4),
        ),
        ("snapshot_urls", "https://example.com/a ".repeat(7)),
    ] {
        let mut backend = AlacrittyTerminalBackend::new(160, 40);
        for row in 1..=40 {
            backend.advance(format!("\x1b[{row};1H{line}").as_bytes());
        }
        let snapshot = backend.snapshot();
        assert_eq!(snapshot.cells.len(), 40);
        assert!(snapshot.cells.iter().all(|row| !row.is_empty()));
        if name == "snapshot_urls" {
            assert!(
                snapshot.cells[0]
                    .iter()
                    .any(|cell| cell.hyperlink.is_some())
            );
        }
        measure(name, 300, None, || {
            black_box(backend.snapshot());
        });
    }
}

#[test]
#[ignore = "manual performance measurement"]
fn cold_scrollback_search() {
    let mut backend = AlacrittyTerminalBackend::with_scrollback(100, 40, 2000);
    for row in 0..1000 {
        let marker = if row % 10 == 0 { "needle" } else { "other" };
        backend.advance(format!("record {row:04} {marker}\r\n").as_bytes());
    }
    backend.clear_search();
    assert_eq!(backend.search("needle", SearchDirection::Next).total, 100);
    measure("scrollback_search_1000_lines", 30, None, || {
        backend.clear_search();
        black_box(backend.search(black_box("needle"), SearchDirection::Next));
    });
}
