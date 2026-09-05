use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use toyoterm_terminal::{SelectionSpan, TerminalCell};

const GLYPH_WIDTH: u32 = 5;
const GLYPH_HEIGHT: u32 = 7;

#[test]
fn terminal_frame_matches_offscreen_image_snapshots() {
    let snapshot = fixture_snapshot();
    let style = RenderStyle {
        background: [12, 18, 28],
        foreground: [224, 231, 239],
        cursor: [255, 196, 72],
        selection: [52, 112, 180],
        ..RenderStyle::default()
    };

    assert_image_snapshot(
        "terminal-frame.ppm",
        &render_fixture(&snapshot, &style, 96, 64),
    );
    assert_image_snapshot(
        "terminal-frame-resized.ppm",
        &render_fixture(&snapshot, &style, 128, 80),
    );
}

fn fixture_snapshot() -> TerminalSnapshot {
    TerminalSnapshot {
        columns: 8,
        rows: 3,
        lines: vec!["ABC".into(), "CAB".into(), "BCA".into()],
        cells: vec![
            vec![
                cell(0, "A", CellColor::Default),
                cell(1, "B", CellColor::Rgb(92, 36, 48)),
                cell(2, "C", CellColor::Default),
            ],
            vec![
                cell(0, "C", CellColor::Default),
                cell(1, "A", CellColor::Default),
                cell(2, "B", CellColor::Default),
            ],
            vec![
                cell(0, "B", CellColor::Default),
                cell(1, "C", CellColor::Default),
                cell(2, "A", CellColor::Default),
            ],
        ],
        selection: vec![
            SelectionSpan {
                row: 0,
                start_column: 1,
                end_column: 2,
            },
            SelectionSpan {
                row: 1,
                start_column: 0,
                end_column: 1,
            },
        ],
        search_matches: Vec::new(),
    }
}

fn cell(column: u16, text: &str, background: CellColor) -> TerminalCell {
    TerminalCell {
        column,
        text: text.into(),
        width: 1,
        attributes: CellAttributes {
            background,
            ..CellAttributes::default()
        },
        hyperlink: None,
    }
}

fn render_fixture(
    snapshot: &TerminalSnapshot,
    style: &RenderStyle,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let mut image = TestImage::new(width, height, style.background);
    let pane = PaneRect::new(8, 7, width.saturating_sub(16), height.saturating_sub(14));
    let layout = TextLayout {
        font_size: 7.0,
        line_height: 14.0,
        cell_width: 9.0,
        horizontal_padding: 4.0,
        vertical_padding: 4.0,
    };

    image.fill_rect(
        PaneRect::new(pane.x, pane.y, pane.width, 2),
        style.selection,
        242,
    );
    for (rect, color) in terminal_backgrounds(
        snapshot,
        pane,
        layout,
        style.background,
        style.foreground,
        &style.ansi,
        false,
    ) {
        image.fill_rect(rect, color, 255);
    }
    for rect in selection_highlight_rects(snapshot, pane, layout) {
        image.fill_rect(rect, style.selection, 210);
    }

    let text_x = pane.x + layout.horizontal_padding as u32;
    let text_y = pane.y + layout.vertical_padding as u32;
    for (row, line) in snapshot.lines.iter().enumerate() {
        for (column, glyph) in line.chars().enumerate() {
            image.draw_glyph(
                text_x + column as u32 * layout.cell_width as u32 + 2,
                text_y + row as u32 * layout.line_height as u32 + 3,
                glyph,
                style.foreground,
            );
        }
    }

    let cursor = CursorState {
        column: 3,
        row: 1,
        visible: true,
        shape: CursorShape::Beam,
    };
    let placement = pane_text_placement(
        pane,
        layout,
        cursor,
        f32::from(cursor.column) * layout.cell_width,
    );
    image.fill_rect(
        PaneRect::new(
            placement.cursor_left as u32,
            placement.cursor_top as u32,
            2,
            layout.line_height as u32,
        ),
        style.cursor,
        255,
    );
    image.to_ppm()
}

fn assert_image_snapshot(name: &str, actual: &[u8]) {
    let path = snapshot_path(name);
    if std::env::var_os("UPDATE_RENDER_SNAPSHOTS").is_some() {
        fs::write(&path, actual).expect("write updated render snapshot");
        return;
    }

    let expected = fs::read(&path).expect("read render snapshot");
    if actual != expected {
        let actual_path = std::env::temp_dir().join(format!("toyoterm-{name}"));
        fs::write(&actual_path, actual).expect("write failed render output");
        panic!(
            "offscreen image differs from {}; actual image written to {} (run with \
             UPDATE_RENDER_SNAPSHOTS=1 to accept it)",
            path.display(),
            actual_path.display()
        );
    }
}

fn snapshot_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("snapshots")
        .join(name)
}

struct TestImage {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

impl TestImage {
    fn new(width: u32, height: u32, color: [u8; 3]) -> Self {
        let mut rgb = vec![0; (width * height * 3) as usize];
        for pixel in rgb.as_chunks_mut::<3>().0 {
            pixel.copy_from_slice(&color);
        }
        Self { width, height, rgb }
    }

    fn fill_rect(&mut self, rect: PaneRect, color: [u8; 3], alpha: u8) {
        let right = rect.x.saturating_add(rect.width).min(self.width);
        let bottom = rect.y.saturating_add(rect.height).min(self.height);
        for y in rect.y.min(self.height)..bottom {
            for x in rect.x.min(self.width)..right {
                self.blend_pixel(x, y, color, alpha);
            }
        }
    }

    fn draw_glyph(&mut self, x: u32, y: u32, glyph: char, color: [u8; 3]) {
        let rows = match glyph {
            'A' => [
                0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
            ],
            'B' => [
                0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
            ],
            'C' => [
                0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111,
            ],
            _ => [0; GLYPH_HEIGHT as usize],
        };
        for (row, bits) in rows.into_iter().enumerate() {
            for column in 0..GLYPH_WIDTH {
                if bits & (1 << (GLYPH_WIDTH - column - 1)) != 0 {
                    self.blend_pixel(x + column, y + row as u32, color, 255);
                }
            }
        }
    }

    fn blend_pixel(&mut self, x: u32, y: u32, color: [u8; 3], alpha: u8) {
        if x >= self.width || y >= self.height {
            return;
        }
        let offset = ((y * self.width + x) * 3) as usize;
        let alpha = u16::from(alpha);
        for (channel, source) in color.into_iter().enumerate() {
            let destination = u16::from(self.rgb[offset + channel]);
            self.rgb[offset + channel] =
                ((u16::from(source) * alpha + destination * (255 - alpha)) / 255) as u8;
        }
    }

    fn to_ppm(&self) -> Vec<u8> {
        let mut ppm = format!("P6\n{} {}\n255\n", self.width, self.height).into_bytes();
        ppm.extend_from_slice(&self.rgb);
        ppm
    }
}
