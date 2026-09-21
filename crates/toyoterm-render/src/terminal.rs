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
                cell.text_size.is_none()
                    && cell.width == 1
                    && cell.text.len() == 1
                    && cell.text.is_ascii()
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

pub(super) fn cell_run_cache_matches(
    pane_cache_matches: bool,
    cached_column: u16,
    cached_row: u16,
    cached_cells: &[toyoterm_terminal::TerminalCell],
    row: u16,
    cells: &[toyoterm_terminal::TerminalCell],
) -> bool {
    pane_cache_matches
        && cached_column == cells[0].column
        && cached_row == row
        && cached_cells == cells
}

pub(super) fn update_terminal_cell_buffer(
    buffer: &mut Buffer,
    font_system: &mut FontSystem,
    cells: &[toyoterm_terminal::TerminalCell],
    layout: TextLayout,
    style: &RenderStyle,
    context: CellRenderContext<'_>,
) {
    buffer.set_wrap(Wrap::None);
    let text_size = cells.first().and_then(|cell| cell.text_size);
    let scale = text_size.map_or(1.0, toyoterm_terminal::TextSize::rendered_scale);
    buffer.set_monospace_width(Some((layout.cell_width * scale).max(0.01)));
    buffer.set_metrics_and_size(
        Metrics::new(
            (layout.font_size * scale).max(0.01),
            (layout.line_height * scale).max(0.01),
        ),
        text_size.map(|_| f32::from(cells[0].width) * layout.cell_width),
        text_size.map(|size| f32::from(size.rows) * layout.line_height),
    );
    buffer.set_rich_text(
        cells.iter().map(|cell| {
            let mut attributes = cell.attributes;
            let hyperlink = cell.hyperlink.is_some();
            if hyperlink {
                attributes.underline = true;
            }
            apply_selection_foreground(
                &mut attributes,
                context.selection,
                context.row,
                cell,
                context.colors,
            );
            (
                cell.text.as_str(),
                glyph_attrs(
                    attributes,
                    hyperlink,
                    &style.font_family,
                    style.font_weight,
                    context.colors,
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

pub(super) fn sized_text_offsets(
    cell: &toyoterm_terminal::TerminalCell,
    layout: TextLayout,
) -> (f32, f32) {
    let Some(size) = cell.text_size else {
        return (0.0, 0.0);
    };
    let fraction = if size.denominator == 0 {
        1.0
    } else {
        f32::from(size.numerator) / f32::from(size.denominator)
    };
    let free_width = f32::from(cell.width) * layout.cell_width * (1.0 - fraction);
    let free_height = f32::from(size.rows) * layout.line_height * (1.0 - fraction);
    (
        alignment_offset(size.horizontal_alignment, free_width),
        alignment_offset(size.vertical_alignment, free_height),
    )
}

fn alignment_offset(alignment: toyoterm_terminal::TextAlignment, free_space: f32) -> f32 {
    match alignment {
        toyoterm_terminal::TextAlignment::Start => 0.0,
        toyoterm_terminal::TextAlignment::End => free_space,
        toyoterm_terminal::TextAlignment::Center => free_space / 2.0,
    }
}

#[derive(Clone, Copy)]
pub(super) struct CellRenderContext<'a> {
    pub(super) row: u16,
    pub(super) selection: &'a [toyoterm_terminal::SelectionSpan],
    pub(super) colors: &'a TerminalColors,
}

pub(super) fn cursor_cell(
    snapshot: &TerminalSnapshot,
    cursor: CursorState,
) -> Option<&toyoterm_terminal::TerminalCell> {
    snapshot
        .cells
        .get(usize::from(cursor.row))?
        .iter()
        .find(|cell| cell.column == cursor.column)
}

pub(super) fn cursor_text_block(
    snapshot: &TerminalSnapshot,
    cursor: CursorState,
) -> Option<(u16, &toyoterm_terminal::TerminalCell)> {
    snapshot.cells.iter().enumerate().find_map(|(row, cells)| {
        cells.iter().find_map(|cell| {
            let size = cell.text_size?;
            let row = u16::try_from(row).ok()?;
            (cursor.row >= row
                && cursor.row < row.saturating_add(u16::from(size.rows))
                && cursor.column >= cell.column
                && cursor.column < cell.column.saturating_add(u16::from(cell.width)))
            .then_some((row, cell))
        })
    })
}

pub(super) fn apply_selection_foreground(
    attributes: &mut CellAttributes,
    selection: &[toyoterm_terminal::SelectionSpan],
    row: u16,
    cell: &toyoterm_terminal::TerminalCell,
    colors: &TerminalColors,
) {
    let cell_end = cell
        .column
        .saturating_add(u16::from(cell.width.max(1)))
        .saturating_sub(1);
    if !selection.iter().any(|span| {
        span.row == row && span.start_column <= cell_end && span.end_column >= cell.column
    }) {
        return;
    }
    let color = colors.selection_foreground.or_else(|| {
        colors.selection_foreground_dynamic.then(|| {
            if attributes.inverse {
                resolve_cell_color(attributes.foreground, colors.foreground, &colors.ansi)
            } else {
                resolve_cell_color(attributes.background, colors.background, &colors.ansi)
            }
        })
    });
    if let Some([red, green, blue]) = color {
        attributes.foreground = CellColor::Rgb(red, green, blue);
        attributes.inverse = false;
    }
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

pub(super) fn selection_reverse_backgrounds(
    snapshot: &TerminalSnapshot,
    pane: PaneRect,
    layout: TextLayout,
    colors: &TerminalColors,
) -> Vec<(PaneRect, [u8; 3])> {
    if !colors.selection_background_dynamic {
        return Vec::new();
    }
    let mut backgrounds = Vec::new();
    for span in &snapshot.selection {
        let cells = snapshot.cells.get(usize::from(span.row));
        let mut run_start = span.start_column;
        let mut run_color = None;
        for column in span.start_column..=span.end_column {
            let cell = cells.and_then(|cells| {
                cells.iter().find(|cell| {
                    let end = cell
                        .column
                        .saturating_add(u16::from(cell.width.max(1)))
                        .saturating_sub(1);
                    cell.column <= column && end >= column
                })
            });
            let color = cell.map_or(colors.foreground, |cell| {
                displayed_foreground(cell.attributes, cell.hyperlink.is_some(), colors)
            });
            if let Some(previous) = run_color
                && previous != color
            {
                if let Some(rect) = selection_cell_range_rect(
                    pane,
                    layout,
                    span.row,
                    run_start,
                    column.saturating_sub(1),
                ) {
                    backgrounds.push((rect, previous));
                }
                run_start = column;
            }
            run_color = Some(color);
        }
        if let Some(color) = run_color
            && let Some(rect) =
                selection_cell_range_rect(pane, layout, span.row, run_start, span.end_column)
        {
            backgrounds.push((rect, color));
        }
    }
    backgrounds
}

fn selection_cell_range_rect(
    pane: PaneRect,
    layout: TextLayout,
    row: u16,
    start_column: u16,
    end_column: u16,
) -> Option<PaneRect> {
    let origin_x = pane.x as f32 + layout.horizontal_padding;
    let origin_y = pane.y as f32 + layout.vertical_padding;
    let left = (origin_x + f32::from(start_column) * layout.cell_width)
        .floor()
        .max(0.0) as u32;
    let right = (origin_x + f32::from(end_column.saturating_add(1)) * layout.cell_width)
        .ceil()
        .max(0.0) as u32;
    let top = (origin_y + f32::from(row) * layout.line_height)
        .floor()
        .max(0.0) as u32;
    let bottom = (origin_y + f32::from(row.saturating_add(1)) * layout.line_height)
        .ceil()
        .max(0.0) as u32;
    let left = left.max(pane.x);
    let top = top.max(pane.y);
    let right = right.min(pane.x.saturating_add(pane.width));
    let bottom = bottom.min(pane.y.saturating_add(pane.height));
    (right > left && bottom > top).then(|| PaneRect::new(left, top, right - left, bottom - top))
}

pub(super) fn cursor_line_highlight_rect(
    pane: PaneRect,
    layout: TextLayout,
    row: u16,
) -> Option<PaneRect> {
    let left = (pane.x as f32 + layout.horizontal_padding).floor().max(0.0) as u32;
    let right = (pane.x.saturating_add(pane.width) as f32 - layout.horizontal_padding)
        .ceil()
        .max(0.0) as u32;
    let top = (pane.y as f32 + layout.vertical_padding + f32::from(row) * layout.line_height)
        .floor()
        .max(0.0) as u32;
    let bottom = (pane.y as f32
        + layout.vertical_padding
        + f32::from(row.saturating_add(1)) * layout.line_height)
        .ceil()
        .max(0.0) as u32;
    let left = left.max(pane.x);
    let top = top.max(pane.y);
    let right = right.min(pane.x.saturating_add(pane.width));
    let bottom = bottom.min(pane.y.saturating_add(pane.height));
    (right > left && bottom > top).then(|| PaneRect::new(left, top, right - left, bottom - top))
}

pub(super) fn cursor_firework_rects(
    pane: PaneRect,
    layout: TextLayout,
    cursor: CursorState,
) -> Vec<(PaneRect, [u8; 3])> {
    let center_x = pane.x as f32
        + layout.horizontal_padding
        + (f32::from(cursor.column) + 0.5) * layout.cell_width;
    let center_y = pane.y as f32
        + layout.vertical_padding
        + (f32::from(cursor.row) + 0.5) * layout.line_height;
    let radius_x = layout.cell_width.max(2.0) * 1.4;
    let radius_y = layout.line_height.max(2.0) * 0.85;
    let spark = ((layout.cell_width.min(layout.line_height) * 0.24).round() as u32).clamp(2, 5);
    let offsets = [
        (0.0, -1.0, [255, 214, 74]),
        (0.72, -0.72, [255, 94, 91]),
        (1.0, 0.0, [89, 214, 255]),
        (0.72, 0.72, [174, 112, 255]),
        (0.0, 1.0, [104, 232, 149]),
        (-0.72, 0.72, [255, 151, 72]),
        (-1.0, 0.0, [255, 105, 180]),
        (-0.72, -0.72, [116, 185, 255]),
    ];
    let pane_right = pane.x.saturating_add(pane.width);
    let pane_bottom = pane.y.saturating_add(pane.height);
    offsets
        .into_iter()
        .filter_map(|(x, y, color)| {
            let left = (center_x + x * radius_x - spark as f32 * 0.5)
                .round()
                .max(pane.x as f32) as u32;
            let top = (center_y + y * radius_y - spark as f32 * 0.5)
                .round()
                .max(pane.y as f32) as u32;
            let right = left.saturating_add(spark).min(pane_right);
            let bottom = top.saturating_add(spark).min(pane_bottom);
            (right > left && bottom > top)
                .then(|| (PaneRect::new(left, top, right - left, bottom - top), color))
        })
        .collect()
}

pub(super) fn command_zone_marker_rects(
    snapshot: &TerminalSnapshot,
    pane: PaneRect,
    layout: TextLayout,
) -> Vec<(PaneRect, Option<i32>)> {
    let origin_y = pane.y as f32 + layout.vertical_padding;
    let pane_bottom = pane.y.saturating_add(pane.height);
    let width = ((layout.cell_width * 0.12).round() as u32).clamp(2, 4);
    let inset = ((layout.horizontal_padding.max(width as f32) - width as f32) * 0.5)
        .round()
        .max(0.0) as u32;
    let left = pane.x.saturating_add(inset);
    snapshot
        .command_zones
        .iter()
        .filter_map(|zone| {
            let top = (origin_y + f32::from(zone.start_row) * layout.line_height)
                .floor()
                .max(0.0) as u32;
            let bottom = (origin_y + f32::from(zone.end_row.saturating_add(1)) * layout.line_height)
                .ceil()
                .max(0.0) as u32;
            let top = top.max(pane.y);
            let bottom = bottom.min(pane_bottom);
            (bottom > top).then(|| {
                (
                    PaneRect::new(left, top, width.min(pane.width), bottom - top),
                    zone.exit_status,
                )
            })
        })
        .collect()
}

pub(super) fn pane_cursor_x(cursor: CursorState, cell_width: f32) -> f32 {
    // The terminal, selections, backgrounds, and hit testing all use a fixed
    // cell grid. Shaped glyph positions can differ from that grid after a
    // window resize or font-size change due to font fallback and fractional
    // advance rounding, so they must not determine the cursor origin.
    f32::from(cursor.column) * cell_width
}

pub(super) fn terminal_backgrounds(
    snapshot: &TerminalSnapshot,
    pane: PaneRect,
    layout: TextLayout,
    colors: &TerminalColors,
    has_background_image: bool,
    render_default_cells: bool,
    default_opacity: f32,
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
            let cell_bottom = cell.text_size.map_or(bottom, |size| {
                (origin_y + (row + usize::from(size.rows)) as f32 * layout.line_height)
                    .ceil()
                    .max(0.0) as u32
            });
            let color = if cell.attributes.inverse {
                resolve_cell_color(cell.attributes.foreground, colors.foreground, &colors.ansi)
            } else {
                resolve_cell_color(cell.attributes.background, colors.background, &colors.ansi)
            };
            let default_cell =
                !cell.attributes.inverse && cell.attributes.background == CellColor::Default;
            if transparent_background_opacity(
                color,
                &colors.transparent_backgrounds,
                default_opacity,
            )
            .is_some_and(|opacity| opacity < 1.0)
            {
                continue;
            }
            if (default_cell && !render_default_cells)
                || (!has_background_image && color == colors.background && !render_default_cells)
            {
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
            let cell_bottom = cell_bottom.min(pane_bottom);
            if right > left && cell_bottom > top {
                backgrounds.push((
                    PaneRect::new(left, top, right - left, cell_bottom - top),
                    color,
                ));
            }
        }
    }
    let default_is_transparent = transparent_background_opacity(
        colors.background,
        &colors.transparent_backgrounds,
        default_opacity,
    )
    .is_some_and(|opacity| opacity < 1.0);
    if render_default_cells && !default_is_transparent {
        for row in 0..snapshot.rows {
            let cells = snapshot
                .cells
                .get(usize::from(row))
                .map(Vec::as_slice)
                .unwrap_or_default();
            let mut column = 0;
            for cell in cells {
                if cell.column > column
                    && let Some(rect) = selection_cell_range_rect(
                        pane,
                        layout,
                        row,
                        column,
                        cell.column.saturating_sub(1),
                    )
                {
                    backgrounds.push((rect, colors.background));
                }
                column = column.max(cell.column.saturating_add(u16::from(cell.width.max(1))));
            }
            if column < snapshot.columns
                && let Some(rect) = selection_cell_range_rect(
                    pane,
                    layout,
                    row,
                    column,
                    snapshot.columns.saturating_sub(1),
                )
            {
                backgrounds.push((rect, colors.background));
            }
        }
    }
    backgrounds
}

pub(super) fn terminal_transparent_backgrounds(
    snapshot: &TerminalSnapshot,
    pane: PaneRect,
    layout: TextLayout,
    colors: &TerminalColors,
    default_opacity: f32,
) -> Vec<(PaneRect, [u8; 3], f32)> {
    let mut backgrounds = Vec::new();
    for row in 0..snapshot.rows {
        let cells = snapshot
            .cells
            .get(usize::from(row))
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut cell_index = 0;
        let mut run_start = 0;
        let mut run_value = None;
        for column in 0..snapshot.columns {
            while cell_index < cells.len()
                && cells[cell_index]
                    .column
                    .saturating_add(u16::from(cells[cell_index].width.max(1)))
                    <= column
            {
                cell_index += 1;
            }
            let attributes = cells
                .get(cell_index)
                .filter(|cell| cell.column <= column)
                .map_or_else(CellAttributes::default, |cell| cell.attributes);
            let color = displayed_background(attributes, colors);
            let value = transparent_background_opacity(
                color,
                &colors.transparent_backgrounds,
                default_opacity,
            )
            .filter(|opacity| *opacity < 1.0)
            .map(|opacity| (color, opacity));
            if value != run_value {
                if let Some((color, opacity)) = run_value
                    && let Some(rect) = selection_cell_range_rect(
                        pane,
                        layout,
                        row,
                        run_start,
                        column.saturating_sub(1),
                    )
                {
                    backgrounds.push((rect, color, opacity));
                }
                run_start = column;
                run_value = value;
            }
        }
        if let Some((color, opacity)) = run_value
            && let Some(rect) = selection_cell_range_rect(
                pane,
                layout,
                row,
                run_start,
                snapshot.columns.saturating_sub(1),
            )
        {
            backgrounds.push((rect, color, opacity));
        }
    }
    backgrounds
}

fn transparent_background_opacity(
    color: [u8; 3],
    configured: &[Option<TerminalTransparentColor>; 7],
    default_opacity: f32,
) -> Option<f32> {
    configured
        .iter()
        .flatten()
        .find(|entry| entry.color == color)
        .map(|entry| entry.opacity.unwrap_or(default_opacity).clamp(0.0, 1.0))
}

fn displayed_background(attributes: CellAttributes, colors: &TerminalColors) -> [u8; 3] {
    if attributes.inverse {
        resolve_cell_color(attributes.foreground, colors.foreground, &colors.ansi)
    } else {
        resolve_cell_color(attributes.background, colors.background, &colors.ansi)
    }
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
    colors: &TerminalColors,
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
            let hyperlink = cell.hyperlink.is_some();
            if hyperlink {
                attributes.underline = true;
            }
            apply_selection_foreground(&mut attributes, &snapshot.selection, row, cell, colors);
            push_rich_span(
                &mut spans,
                &cell.text,
                glyph_attrs(attributes, hyperlink, font_family, font_weight, colors),
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
    hyperlink: bool,
    font_family: &'a str,
    font_weight: u16,
    colors: &TerminalColors,
) -> Attrs<'a> {
    let foreground = displayed_foreground(attributes, hyperlink, colors);
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
        if let Some(color) = colors.underline {
            attrs = attrs.underline_color(glyph_color(color, 255));
        }
    }
    if attributes.strikethrough {
        attrs = attrs.strikethrough();
    }
    attrs
}

fn displayed_foreground(
    attributes: CellAttributes,
    hyperlink: bool,
    colors: &TerminalColors,
) -> [u8; 3] {
    let default_foreground = if hyperlink && attributes.foreground == CellColor::Default {
        colors.link.unwrap_or(colors.foreground)
    } else if attributes.bold && attributes.foreground == CellColor::Default {
        colors.bold
    } else {
        colors.foreground
    };
    let foreground = if attributes.inverse {
        resolve_cell_color(attributes.background, colors.background, &colors.ansi)
    } else {
        resolve_cell_color(attributes.foreground, default_foreground, &colors.ansi)
    };
    special_attribute_foreground(attributes, foreground, colors)
}

fn special_attribute_foreground(
    attributes: CellAttributes,
    fallback: [u8; 3],
    colors: &TerminalColors,
) -> [u8; 3] {
    if attributes.foreground != CellColor::Default && !colors.special.override_ansi {
        return fallback;
    }
    let candidates = [
        (attributes.bold, 0, colors.special.bold),
        (attributes.underline, 1, colors.special.underline),
        (attributes.blink, 2, colors.special.blink),
        (attributes.inverse, 3, colors.special.reverse),
        (attributes.italic, 4, colors.special.italic),
    ];
    candidates
        .into_iter()
        .find_map(|(active, index, color)| {
            (active && colors.special.enabled[index])
                .then_some(color)
                .flatten()
        })
        .unwrap_or(fallback)
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

pub(super) fn preferred_alpha_mode_for_content(
    supported: &[CompositeAlphaMode],
    opacity: f32,
    transparent_cells: bool,
) -> CompositeAlphaMode {
    if !transparent_cells || cfg!(target_os = "windows") || opacity < 1.0 {
        return preferred_alpha_mode(supported, opacity);
    }
    [
        CompositeAlphaMode::PostMultiplied,
        CompositeAlphaMode::PreMultiplied,
        CompositeAlphaMode::Inherit,
    ]
    .into_iter()
    .find(|mode| supported.contains(mode))
    .unwrap_or_else(|| preferred_alpha_mode(supported, opacity))
}

pub(super) fn glyph_color(color: [u8; 3], alpha: u8) -> GlyphColor {
    GlyphColor::rgba(color[0], color[1], color[2], alpha)
}
