use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use toyoterm_terminal::{SelectionSpan, TerminalCell};

const GLYPH_WIDTH: u32 = 5;
const GLYPH_HEIGHT: u32 = 7;

#[test]
#[ignore = "requires a working GPU or software adapter"]
fn gpu_inactive_window_tint_preserves_transparency() {
    pollster::block_on(async {
        let instance = Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter
            .request_device(&DeviceDescriptor::default())
            .await
            .unwrap();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let pipeline = create_inactive_window_pipeline(&device, format);
        let vertices = inactive_window_vertices(1, 1, false);
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("inactive tint test"),
            contents: bytemuck::cast_slice(&vertices),
            usage: BufferUsages::VERTEX,
        });
        for alpha in [0.0, 0.5, 1.0] {
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("inactive tint target"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = target.create_view(&Default::default());
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("inactive tint pixels"),
                size: 256,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let attachments = [Some(RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color {
                            r: alpha,
                            g: 0.0,
                            b: 0.0,
                            a: alpha,
                        }),
                        store: StoreOp::Store,
                    },
                })];
                let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    color_attachments: &attachments,
                    ..Default::default()
                });
                pass.set_pipeline(&pipeline);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..vertices.len() as u32, 0..1);
            }
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(1),
                    },
                },
                target.size(),
            );
            queue.submit([encoder.finish()]);
            let (sender, receiver) = std::sync::mpsc::channel();
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    sender.send(result).unwrap()
                });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            receiver.recv().unwrap().unwrap();
            let bytes = readback.slice(..).get_mapped_range().unwrap();
            let gray = f64::from(srgb_channel_to_linear(128)) * 0.16;
            let expected = [(0.84 + gray) * alpha, gray * alpha, gray * alpha, alpha];
            for (actual, expected) in bytes[..4].iter().zip(expected) {
                assert!(actual.abs_diff((expected * 255.0).round() as u8) <= 1);
            }
        }
    });
}

#[test]
fn unicode_render_colors_and_wide_cursor_have_stable_pixels() {
    unicode_pixel_fixtures();
}

fn unicode_pixel_fixtures() -> Vec<TestImage> {
    // Fixed bitmap masks test composition, not machine-dependent font outlines.
    use toyoterm_terminal::{AlacrittyTerminalBackend, SelectionKind, TerminalBackend};
    let mut terminal = AlacrittyTerminalBackend::new(20, 4);
    terminal.advance("A界e\u{301}😀Z".as_bytes());
    terminal.start_selection(2, 0, SelectionKind::Simple);
    terminal.update_selection(5, 0);
    let snapshot = terminal.snapshot();
    let style = RenderStyle::default();
    let layout = TextLayout {
        font_size: 7.0,
        line_height: 14.0,
        cell_width: 9.0,
        horizontal_padding: 0.0,
        vertical_padding: 0.0,
    };
    let pane = PaneRect::new(0, 0, 72, 28);
    let mut colors = TerminalColors {
        foreground: [20, 40, 60],
        bold: [20, 40, 60],
        background: [1, 2, 3],
        cursor: [255; 3],
        ansi: style.ansi,
        link: None,
        cursor_foreground: None,
        underline: None,
        selection_background: None,
        selection_foreground: Some([70, 80, 90]),
        selection_background_dynamic: false,
        selection_foreground_dynamic: false,
        visual_bell: None,
        transparent_backgrounds: [None; 7],
        special: Default::default(),
    };
    let mut image = TestImage::new(72, 28, colors.background);
    for rect in selection_highlight_rects(&snapshot, pane, layout) {
        image.fill_rect(rect, [30, 50, 70], 255);
    }
    for cell in &snapshot.cells[0] {
        let mut attributes = cell.attributes;
        apply_selection_foreground(&mut attributes, &snapshot.selection, 0, cell, &colors);
        let color = glyph_attrs(attributes, false, "monospace", 400, &colors)
            .color_opt
            .unwrap();
        image.draw_glyph(
            u32::from(cell.column) * 9 + 2,
            3,
            'A',
            [color.r(), color.g(), color.b()],
        );
    }
    let pixel = |image: &TestImage, x: usize, y: usize| -> [u8; 3] {
        image.rgb[(y * 72 + x) * 3..][..3].try_into().unwrap()
    };
    assert_eq!(
        pixel(&image, 3, 3),
        colors.foreground,
        "default/fallback foreground"
    );
    assert_eq!(
        pixel(&image, 12, 3),
        [70, 80, 90],
        "wide selected foreground"
    );
    assert_eq!(
        pixel(&image, 30, 3),
        [70, 80, 90],
        "combining selected foreground"
    );
    assert_eq!(
        pixel(&image, 39, 3),
        [70, 80, 90],
        "emoji selected foreground"
    );
    assert_eq!(pixel(&image, 9, 0), [30, 50, 70]);
    assert_eq!(pixel(&image, 54, 0), colors.background);
    let cursor = CursorState {
        column: 1,
        row: 0,
        shape: CursorShape::Block,
        visible: true,
    };
    let cell = cursor_text_cell(&snapshot, cursor).unwrap();
    assert_eq!(cell.width, 2);
    assert_eq!(cursor_glyph(cursor.shape, usize::from(cell.width), 1), "██");
    let mut frames = vec![image.clone()];
    for (background, explicit, expected) in [
        ([255; 3], None, [0; 3]),
        ([0; 3], None, [255; 3]),
        ([255; 3], Some([12, 34, 56]), [12, 34, 56]),
    ] {
        colors.cursor = background;
        colors.cursor_foreground = explicit;
        image.fill_rect(
            PaneRect::new(9, 0, u32::from(cell.width) * 9, 14),
            background,
            255,
        );
        image.draw_glyph(11, 3, 'A', cursor_text_color(&colors));
        assert_eq!(pixel(&image, 9, 0), background);
        assert_eq!(pixel(&image, 26, 13), background, "second cursor cell");
        assert_eq!(pixel(&image, 12, 3), expected, "block text contrast");
        assert_eq!(
            pixel(&image, 27, 0),
            [30, 50, 70],
            "cursor stops before next cell"
        );
        frames.push(image.clone());
    }
    frames
}

#[test]
#[ignore = "requires a working GPU or software adapter"]
fn gpu_unicode_bitmap_composition_matches_offscreen_pixels() {
    // The same font-independent masks/rectangles go through the production UI
    // pipeline. This tests GPU composition, not glyphon font rasterization.
    pollster::block_on(async {
        let instance = Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter
            .request_device(&DeviceDescriptor::default())
            .await
            .unwrap();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let pipeline = create_ui_pipeline(&device, format);
        for image in unicode_pixel_fixtures() {
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Unicode bitmap composition"),
                size: wgpu::Extent3d {
                    width: image.width,
                    height: image.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = target.create_view(&Default::default());
            let mut vertices = Vec::new();
            for (rect, rgb, alpha) in &image.operations {
                let color = [
                    f32::from(rgb[0]) / 255.0,
                    f32::from(rgb[1]) / 255.0,
                    f32::from(rgb[2]) / 255.0,
                    f32::from(*alpha) / 255.0,
                ];
                push_ui_rect(&mut vertices, *rect, color, image.width, image.height);
            }
            let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Unicode mask vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: BufferUsages::VERTEX,
            });
            let stride = (image.width * 4).div_ceil(256) * 256;
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Unicode pixels"),
                size: u64::from(stride * image.height),
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let attachments = [Some(RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color::BLACK),
                        store: StoreOp::Store,
                    },
                })];
                let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    color_attachments: &attachments,
                    ..Default::default()
                });
                pass.set_pipeline(&pipeline);
                pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                pass.draw(0..vertices.len() as u32, 0..1);
            }
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(stride),
                        rows_per_image: Some(image.height),
                    },
                },
                target.size(),
            );
            queue.submit([encoder.finish()]);
            let (sender, receiver) = std::sync::mpsc::channel();
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    sender.send(result).unwrap()
                });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            receiver.recv().unwrap().unwrap();
            let bytes = readback.slice(..).get_mapped_range().unwrap();
            for y in 0..image.height as usize {
                for x in 0..image.width as usize {
                    let actual = &bytes[y * stride as usize + x * 4..][..3];
                    let expected = &image.rgb[(y * image.width as usize + x) * 3..][..3];
                    for (actual, expected) in actual.iter().zip(expected) {
                        assert!(
                            actual.abs_diff(*expected) <= 1,
                            "pixel ({x}, {y}): {actual} vs {expected}"
                        );
                    }
                }
            }
        }
    });
}

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
        images: Vec::new(),
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
        command_zones: Vec::new(),
    }
}

fn cell(column: u16, text: &str, background: CellColor) -> TerminalCell {
    TerminalCell {
        column,
        text: text.into(),
        width: 1,
        text_size: None,
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
    let colors = TerminalColors {
        foreground: style.foreground,
        bold: style.foreground,
        background: style.background,
        cursor: style.cursor,
        ansi: style.ansi,
        link: None,
        cursor_foreground: None,
        underline: None,
        selection_background: None,
        selection_foreground: None,
        selection_background_dynamic: false,
        selection_foreground_dynamic: false,
        visual_bell: None,
        transparent_backgrounds: [None; 7],
        special: Default::default(),
    };
    for (rect, color) in
        terminal_backgrounds(snapshot, pane, layout, &colors, false, false, style.opacity)
    {
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

#[derive(Clone)]
struct TestImage {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
    operations: Vec<(PaneRect, [u8; 3], u8)>,
}

impl TestImage {
    fn new(width: u32, height: u32, color: [u8; 3]) -> Self {
        let mut rgb = vec![0; (width * height * 3) as usize];
        for pixel in rgb.as_chunks_mut::<3>().0 {
            pixel.copy_from_slice(&color);
        }
        Self {
            width,
            height,
            rgb,
            operations: vec![(PaneRect::new(0, 0, width, height), color, 255)],
        }
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
        self.operations
            .push((PaneRect::new(x, y, 1, 1), color, alpha));
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
