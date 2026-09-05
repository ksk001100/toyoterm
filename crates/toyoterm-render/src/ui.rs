use super::*;

pub(super) fn pane_border_rects(rect: PaneRect, width: u32) -> [PaneRect; 4] {
    let top = width.min(rect.height);
    let bottom = width.min(rect.height - top);
    let left = width.min(rect.width);
    let right = width.min(rect.width - left);
    let middle_height = rect.height - top - bottom;
    [
        PaneRect::new(rect.x, rect.y, rect.width, top),
        PaneRect::new(
            rect.x,
            rect.y.saturating_add(rect.height - bottom),
            rect.width,
            bottom,
        ),
        PaneRect::new(rect.x, rect.y.saturating_add(top), left, middle_height),
        PaneRect::new(
            rect.x.saturating_add(rect.width - right),
            rect.y.saturating_add(top),
            right,
            middle_height,
        ),
    ]
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borders_cover_each_edge_once_and_leave_the_interior_clear() {
        // Includes hidden borders, empty/tiny panes, and widths larger than a pane.
        for (w, h) in [(10, 8), (1, 1), (0, 5), (5, 0), (3, 7)] {
            for width in [0, 1, 2, 20, u32::MAX] {
                let pane = PaneRect::new(3, 4, w, h);
                let borders = pane_border_rects(pane, width);
                for edge in borders.iter().filter(|r| r.width > 0 && r.height > 0) {
                    assert!(edge.x >= pane.x && edge.y >= pane.y);
                    assert!(edge.x + edge.width <= pane.x + w);
                    assert!(edge.y + edge.height <= pane.y + h);
                }
                for y in 0..h {
                    for x in 0..w {
                        let count = borders
                            .iter()
                            .filter(|r| {
                                let px = pane.x + x;
                                let py = pane.y + y;
                                px >= r.x && px < r.x + r.width && py >= r.y && py < r.y + r.height
                            })
                            .count();
                        let on_edge =
                            x < width || y < width || w - 1 - x < width || h - 1 - y < width;
                        assert_eq!(
                            count,
                            usize::from(on_edge),
                            "{w}x{h}, width {width}, ({x}, {y})"
                        );
                    }
                }
            }
        }
    }
}
