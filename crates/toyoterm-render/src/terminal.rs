use super::*;

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

pub(super) fn selection_text(snapshot: &TerminalSnapshot) -> String {
    if snapshot.selection.is_empty() {
        return String::new();
    }

    let mut text = String::new();
    for row in 0..snapshot.rows {
        if row > 0 {
            text.push('\n');
        }
        let Some(span) = snapshot.selection.iter().find(|span| span.row == row) else {
            continue;
        };
        text.extend(std::iter::repeat_n(' ', usize::from(span.start_column)));
        let width = span.end_column.saturating_sub(span.start_column) + 1;
        text.extend(std::iter::repeat_n('█', usize::from(width)));
    }
    text
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
            if color == default_background {
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
        .family(Family::Name(font_family))
        .weight(Weight(font_weight));
    let mut spans = Vec::new();
    for row in 0..snapshot.rows {
        if row > 0 {
            spans.push(("\n".to_owned(), default_attrs.clone()));
        }
        let Some(cells) = snapshot.cells.get(usize::from(row)) else {
            continue;
        };
        let mut column = 0;
        for cell in cells {
            if cell.column > column {
                spans.push((
                    " ".repeat(usize::from(cell.column - column)),
                    default_attrs.clone(),
                ));
            }
            let mut attributes = cell.attributes;
            if cell.hyperlink.is_some() {
                attributes.underline = true;
            }
            spans.push((
                cell.text.clone(),
                glyph_attrs(
                    attributes,
                    font_family,
                    font_weight,
                    default_foreground,
                    default_background,
                    ansi,
                ),
            ));
            column = cell.column.saturating_add(u16::from(cell.width.max(1)));
        }
        if let Some(cursor) = cursor.filter(|cursor| cursor.row == row)
            && cursor.column > column
        {
            spans.push((
                " ".repeat(usize::from(cursor.column - column)),
                default_attrs.clone(),
            ));
        }
    }
    spans
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
        .family(Family::Name(font_family))
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
    let preferences: &[CompositeAlphaMode] = if opacity < 1.0 {
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
