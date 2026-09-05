use super::*;

/// Keep non-ASCII cells (including their combining marks) independent of the
/// surrounding text. Font fallback advances need not match terminal widths.
/// Contiguous ASCII cells can still share shaping work and rich attributes.
pub(super) fn terminal_cell_runs(
    snapshot: &TerminalSnapshot,
) -> Vec<(u16, &[toyoterm_terminal::TerminalCell])> {
    let mut runs = Vec::new();
    for (row, cells) in snapshot
        .cells
        .iter()
        .take(usize::from(snapshot.rows))
        .enumerate()
    {
        let mut start = 0;
        while start < cells.len() {
            let mut end = start + 1;
            let ascii_cell = |cell: &toyoterm_terminal::TerminalCell| {
                cell.width == 1 && cell.text.len() == 1 && cell.text.is_ascii()
            };
            if ascii_cell(&cells[start]) {
                while end < cells.len()
                    && ascii_cell(&cells[end])
                    && cells[end].column == cells[end - 1].column.saturating_add(1)
                {
                    end += 1;
                }
            }
            runs.push((row as u16, &cells[start..end]));
            start = end;
        }
    }
    runs
}

pub(super) fn update_terminal_cell_buffer(
    buffer: &mut Buffer,
    font_system: &mut FontSystem,
    cells: &[toyoterm_terminal::TerminalCell],
    layout: TextLayout,
    style: &RenderStyle,
) {
    buffer.set_wrap(Wrap::None);
    buffer.set_monospace_width(Some(layout.cell_width));
    buffer.set_metrics_and_size(
        Metrics::new(layout.font_size.max(1.0), layout.line_height.max(1.0)),
        None,
        None,
    );
    buffer.set_rich_text(
        cells.iter().map(|cell| {
            let mut attributes = cell.attributes;
            if cell.hyperlink.is_some() {
                attributes.underline = true;
            }
            (
                cell.text.as_str(),
                glyph_attrs(
                    attributes,
                    &style.font_family,
                    style.font_weight,
                    style.foreground,
                    style.background,
                    &style.ansi,
                ),
            )
        }),
        &Attrs::new()
            .family(resolve_font_family(&style.font_family))
            .weight(Weight(style.font_weight)),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);
}

pub(super) fn pane_text_placement(
    rect: PaneRect,
    layout: TextLayout,
    cursor: CursorState,
    cursor_x: f32,
) -> PaneTextPlacement {
    let text_left = rect.x as f32 + layout.horizontal_padding;
    let text_top = rect.y as f32 + layout.vertical_padding;
    PaneTextPlacement {
        bounds: rect,
        text_left,
        text_top,
        cursor_left: text_left + cursor_x,
        cursor_top: text_top + f32::from(cursor.row) * layout.line_height,
    }
}

pub(super) fn surface_resize(size: PhysicalSize<u32>) -> SurfaceResize {
    if size.width == 0 || size.height == 0 {
        SurfaceResize::Suspend
    } else {
        SurfaceResize::Configure {
            width: size.width,
            height: size.height,
        }
    }
}

pub(super) fn surface_recovery_action(
    status: &CurrentSurfaceTexture,
) -> Option<SurfaceRecoveryAction> {
    match status {
        CurrentSurfaceTexture::Success(_) | CurrentSurfaceTexture::Suboptimal(_) => None,
        CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => {
            Some(SurfaceRecoveryAction::Skip)
        }
        CurrentSurfaceTexture::Outdated => Some(SurfaceRecoveryAction::Reconfigure),
        CurrentSurfaceTexture::Lost => Some(SurfaceRecoveryAction::Recreate),
        CurrentSurfaceTexture::Validation => Some(SurfaceRecoveryAction::Fail),
    }
}

pub(super) fn pane_bounds(rect: PaneRect) -> TextBounds {
    TextBounds {
        left: rect.x.min(i32::MAX as u32) as i32,
        top: rect.y.min(i32::MAX as u32) as i32,
        right: rect.x.saturating_add(rect.width).min(i32::MAX as u32) as i32,
        bottom: rect.y.saturating_add(rect.height).min(i32::MAX as u32) as i32,
    }
}

pub(super) fn terminal_cursor_x(
    buffer: &Buffer,
    snapshot: &TerminalSnapshot,
    cursor: CursorState,
) -> Option<f32> {
    let text_cursor = TextCursor::new(
        usize::from(cursor.row),
        terminal_cursor_byte_index(snapshot, cursor),
    );
    buffer
        .layout_runs()
        .find(|run| run.line_i == usize::from(cursor.row))
        .and_then(|run| run.cursor_position(&text_cursor))
}

pub(super) fn selection_highlight_rects(
    snapshot: &TerminalSnapshot,
    pane: PaneRect,
    layout: TextLayout,
) -> Vec<PaneRect> {
    let origin_x = pane.x as f32 + layout.horizontal_padding;
    let origin_y = pane.y as f32 + layout.vertical_padding;
    let pane_right = pane.x.saturating_add(pane.width);
    let pane_bottom = pane.y.saturating_add(pane.height);
    snapshot
        .selection
        .iter()
        .filter_map(|span| {
            let left = (origin_x + f32::from(span.start_column) * layout.cell_width)
                .floor()
                .max(0.0) as u32;
            let right = (origin_x
                + f32::from(span.end_column.saturating_add(1)) * layout.cell_width)
                .ceil()
                .max(0.0) as u32;
            let top = (origin_y + f32::from(span.row) * layout.line_height)
                .floor()
                .max(0.0) as u32;
            let bottom = (origin_y + f32::from(span.row.saturating_add(1)) * layout.line_height)
                .ceil()
                .max(0.0) as u32;
            let left = left.max(pane.x);
            let top = top.max(pane.y);
            let right = right.min(pane_right);
            let bottom = bottom.min(pane_bottom);
            (right > left && bottom > top)
                .then(|| PaneRect::new(left, top, right - left, bottom - top))
        })
        .collect()
}

pub(super) fn pane_cursor_x(
    buffer: &Buffer,
    snapshot: &TerminalSnapshot,
    cursor: CursorState,
    cell_width: f32,
    cursor_uses_grid: bool,
) -> f32 {
    if cursor_uses_grid {
        f32::from(cursor.column) * cell_width
    } else {
        terminal_cursor_x(buffer, snapshot, cursor)
            .unwrap_or_else(|| f32::from(cursor.column) * cell_width)
    }
}

pub(super) fn terminal_cursor_byte_index(
    snapshot: &TerminalSnapshot,
    cursor: CursorState,
) -> usize {
    let Some(cells) = snapshot.cells.get(usize::from(cursor.row)) else {
        return usize::from(cursor.column);
    };
    let mut byte_index = 0;
    let mut column = 0;
    for cell in cells {
        if cursor.column <= cell.column {
            return byte_index + usize::from(cursor.column.saturating_sub(column));
        }
        byte_index += usize::from(cell.column.saturating_sub(column));
        let cell_end = cell.column.saturating_add(u16::from(cell.width.max(1)));
        if cursor.column < cell_end {
            return byte_index;
        }
        byte_index += cell.text.len();
        column = cell_end;
    }
    byte_index + usize::from(cursor.column.saturating_sub(column))
}

pub(super) fn terminal_backgrounds(
    snapshot: &TerminalSnapshot,
    pane: PaneRect,
    layout: TextLayout,
    default_background: [u8; 3],
    default_foreground: [u8; 3],
    ansi: &[[u8; 3]; 16],
    has_background_image: bool,
) -> Vec<(PaneRect, [u8; 3])> {
    let pane_right = pane.x.saturating_add(pane.width);
    let pane_bottom = pane.y.saturating_add(pane.height);
    let origin_x = pane.x as f32 + layout.horizontal_padding;
    let origin_y = pane.y as f32 + layout.vertical_padding;
    let mut backgrounds = Vec::new();

    for (row, cells) in snapshot.cells.iter().enumerate() {
        let top = (origin_y + row as f32 * layout.line_height)
            .floor()
            .max(0.0) as u32;
        let bottom = (origin_y + (row + 1) as f32 * layout.line_height)
            .ceil()
            .max(0.0) as u32;
        if top >= pane_bottom {
            break;
        }
        for cell in cells {
            let color = if cell.attributes.inverse {
                resolve_cell_color(cell.attributes.foreground, default_foreground, ansi)
            } else {
                resolve_cell_color(cell.attributes.background, default_background, ansi)
            };
            let default_cell =
                !cell.attributes.inverse && cell.attributes.background == CellColor::Default;
            if default_cell || (!has_background_image && color == default_background) {
                continue;
            }
            let left = (origin_x + f32::from(cell.column) * layout.cell_width)
                .floor()
                .max(0.0) as u32;
            let right = (origin_x
                + f32::from(cell.column.saturating_add(u16::from(cell.width.max(1))))
                    * layout.cell_width)
                .ceil()
                .max(0.0) as u32;
            let left = left.max(pane.x);
            let top = top.max(pane.y);
            let right = right.min(pane_right);
            let bottom = bottom.min(pane_bottom);
            if right > left && bottom > top {
                backgrounds.push((PaneRect::new(left, top, right - left, bottom - top), color));
            }
        }
    }
    backgrounds
}

pub(super) fn search_highlight_rects(
    snapshot: &TerminalSnapshot,
    pane: PaneRect,
    layout: TextLayout,
) -> Vec<(PaneRect, bool)> {
    let origin_x = pane.x as f32 + layout.horizontal_padding;
    let origin_y = pane.y as f32 + layout.vertical_padding;
    let right = pane.x.saturating_add(pane.width);
    let bottom = pane.y.saturating_add(pane.height);
    snapshot
        .search_matches
        .iter()
        .filter_map(|found| {
            let left =
                (origin_x + f32::from(found.start_column) * layout.cell_width).floor() as u32;
            let match_right = (origin_x
                + f32::from(found.end_column.saturating_add(1)) * layout.cell_width)
                .ceil() as u32;
            let top = (origin_y + f32::from(found.row) * layout.line_height).floor() as u32;
            let match_bottom = (origin_y
                + f32::from(found.row.saturating_add(1)) * layout.line_height)
                .ceil() as u32;
            let left = left.max(pane.x);
            let top = top.max(pane.y);
            let match_right = match_right.min(right);
            let match_bottom = match_bottom.min(bottom);
            (match_right > left && match_bottom > top).then(|| {
                (
                    PaneRect::new(left, top, match_right - left, match_bottom - top),
                    found.active,
                )
            })
        })
        .collect()
}

pub(super) fn terminal_rich_text<'a>(
    snapshot: &TerminalSnapshot,
    cursor: Option<CursorState>,
    font_family: &'a str,
    font_weight: u16,
    default_foreground: [u8; 3],
    default_background: [u8; 3],
    ansi: &[[u8; 3]; 16],
) -> Vec<(String, Attrs<'a>)> {
    if snapshot.cells.is_empty() {
        return Vec::new();
    }

    let default_attrs = Attrs::new()
        .family(resolve_font_family(font_family))
        .weight(Weight(font_weight));
    let mut spans = Vec::new();
    for row in 0..snapshot.rows {
        if row > 0 {
            push_rich_span(&mut spans, "\n", default_attrs.clone());
        }
        let Some(cells) = snapshot.cells.get(usize::from(row)) else {
            continue;
        };
        let mut column = 0;
        for cell in cells {
            if cell.column > column {
                push_rich_spaces(
                    &mut spans,
                    usize::from(cell.column - column),
                    default_attrs.clone(),
                );
            }
            let mut attributes = cell.attributes;
            if cell.hyperlink.is_some() {
                attributes.underline = true;
            }
            push_rich_span(
                &mut spans,
                &cell.text,
                glyph_attrs(
                    attributes,
                    font_family,
                    font_weight,
                    default_foreground,
                    default_background,
                    ansi,
                ),
            );
            column = cell.column.saturating_add(u16::from(cell.width.max(1)));
        }
        if let Some(cursor) = cursor.filter(|cursor| cursor.row == row)
            && cursor.column > column
        {
            push_rich_spaces(
                &mut spans,
                usize::from(cursor.column - column),
                default_attrs.clone(),
            );
        }
    }
    spans
}

fn push_rich_span<'a>(spans: &mut Vec<(String, Attrs<'a>)>, text: &str, attrs: Attrs<'a>) {
    if text.is_empty() {
        return;
    }
    if let Some((previous, previous_attrs)) = spans.last_mut()
        && *previous_attrs == attrs
    {
        previous.push_str(text);
    } else {
        spans.push((text.to_owned(), attrs));
    }
}

fn push_rich_spaces<'a>(spans: &mut Vec<(String, Attrs<'a>)>, count: usize, attrs: Attrs<'a>) {
    if count == 0 {
        return;
    }
    if let Some((previous, previous_attrs)) = spans.last_mut()
        && *previous_attrs == attrs
    {
        previous.extend(std::iter::repeat_n(' ', count));
    } else {
        spans.push((" ".repeat(count), attrs));
    }
}

pub(super) fn glyph_attrs<'a>(
    attributes: CellAttributes,
    font_family: &'a str,
    font_weight: u16,
    default_foreground: [u8; 3],
    default_background: [u8; 3],
    ansi: &[[u8; 3]; 16],
) -> Attrs<'a> {
    let foreground = if attributes.inverse {
        resolve_cell_color(attributes.background, default_background, ansi)
    } else {
        resolve_cell_color(attributes.foreground, default_foreground, ansi)
    };
    let alpha = if attributes.hidden {
        0
    } else if attributes.dim {
        150
    } else {
        255
    };
    let mut attrs = Attrs::new()
        .family(resolve_font_family(font_family))
        .weight(Weight(font_weight))
        .color(glyph_color(foreground, alpha));
    if attributes.bold {
        attrs = attrs.weight(Weight(font_weight.saturating_add(300).min(1000)));
    }
    if attributes.italic {
        attrs = attrs.style(Style::Italic);
    }
    if attributes.underline {
        attrs = attrs.underline(glyphon::cosmic_text::UnderlineStyle::Single);
    }
    if attributes.strikethrough {
        attrs = attrs.strikethrough();
    }
    attrs
}

pub(super) fn resolve_cell_color(
    color: CellColor,
    default: [u8; 3],
    ansi: &[[u8; 3]; 16],
) -> [u8; 3] {
    match color {
        CellColor::Default => default,
        CellColor::Rgb(red, green, blue) => [red, green, blue],
        CellColor::Indexed(index) => xterm_color(index, ansi),
    }
}

pub(super) const fn default_ansi_palette() -> [[u8; 3]; 16] {
    [
        [0, 0, 0],
        [205, 0, 0],
        [0, 205, 0],
        [205, 205, 0],
        [0, 0, 238],
        [205, 0, 205],
        [0, 205, 205],
        [229, 229, 229],
        [127, 127, 127],
        [255, 0, 0],
        [0, 255, 0],
        [255, 255, 0],
        [92, 92, 255],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 255],
    ]
}

pub(super) fn xterm_color(index: u8, ansi: &[[u8; 3]; 16]) -> [u8; 3] {
    match index {
        0..=15 => ansi[usize::from(index)],
        16..=231 => {
            let index = index - 16;
            let component = |value: u8| if value == 0 { 0 } else { value * 40 + 55 };
            [
                component(index / 36),
                component(index / 6 % 6),
                component(index % 6),
            ]
        }
        232..=255 => {
            let level = (index - 232) * 10 + 8;
            [level, level, level]
        }
    }
}

pub(super) fn parse_rgb(value: &str) -> Result<[u8; 3], RenderError> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 {
        return Err(RenderError::new(
            "parse color",
            format!("expected #RRGGBB, got `{value}`"),
        ));
    }
    let parse = |range| {
        u8::from_str_radix(&hex[range], 16)
            .map_err(|_| RenderError::new("parse color", format!("invalid color `{value}`")))
    };
    Ok([parse(0..2)?, parse(2..4)?, parse(4..6)?])
}

pub(super) fn clear_color(style: &RenderStyle, alpha_mode: CompositeAlphaMode) -> Color {
    let alpha = f64::from(style.opacity.clamp(0.0, 1.0));
    let multiplier = if alpha_mode == CompositeAlphaMode::PreMultiplied {
        alpha
    } else {
        1.0
    };
    Color {
        r: f64::from(srgb_channel_to_linear(style.background[0])) * multiplier,
        g: f64::from(srgb_channel_to_linear(style.background[1])) * multiplier,
        b: f64::from(srgb_channel_to_linear(style.background[2])) * multiplier,
        a: alpha,
    }
}

pub(super) fn preferred_alpha_mode(
    supported: &[CompositeAlphaMode],
    opacity: f32,
) -> CompositeAlphaMode {
    // Keep the Windows swapchain's alpha mode stable across opacity 1.0.
    // An alpha-capable swapchain also renders fully opaque when clear alpha is 1.
    let preferences: &[CompositeAlphaMode] = if cfg!(target_os = "windows") {
        // DirectComposition consumes premultiplied pixels, matching the
        // source-over output of our UI and glyph pipelines.
        &[
            CompositeAlphaMode::PreMultiplied,
            CompositeAlphaMode::PostMultiplied,
            CompositeAlphaMode::Inherit,
        ]
    } else if opacity < 1.0 {
        &[
            CompositeAlphaMode::PostMultiplied,
            CompositeAlphaMode::PreMultiplied,
            CompositeAlphaMode::Inherit,
        ]
    } else {
        &[CompositeAlphaMode::Opaque, CompositeAlphaMode::Inherit]
    };
    preferences
        .iter()
        .copied()
        .find(|mode| supported.contains(mode))
        .unwrap_or(CompositeAlphaMode::Auto)
}

pub(super) fn glyph_color(color: [u8; 3], alpha: u8) -> GlyphColor {
    GlyphColor::rgba(color[0], color[1], color[2], alpha)
}
