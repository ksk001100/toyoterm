use super::*;

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
