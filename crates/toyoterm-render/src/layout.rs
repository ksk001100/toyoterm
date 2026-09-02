use toyoterm_api::{PaneId, SplitDirection, TabId, WorkspaceId};
use toyoterm_mux::PaneNode;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PaneRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl PaneRect {
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= f64::from(self.x)
            && y >= f64::from(self.y)
            && x < f64::from(self.x.saturating_add(self.width))
            && y < f64::from(self.y.saturating_add(self.height))
    }

    pub fn center(self) -> (f64, f64) {
        (
            f64::from(self.x) + f64::from(self.width) / 2.0,
            f64::from(self.y) + f64::from(self.height) / 2.0,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PanePlacement {
    pub pane: PaneId,
    pub rect: PaneRect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SplitBoundary {
    pub axis: SplitAxis,
    pub rect: PaneRect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TabPlacement {
    pub tab: TabId,
    pub rect: PaneRect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspacePlacement {
    pub workspace: WorkspaceId,
    pub rect: PaneRect,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConfigErrorLayout {
    notice: PaneRect,
    open_log: PaneRect,
    dismiss: PaneRect,
}

impl ConfigErrorLayout {
    pub fn calculate(bounds: PaneRect, action_height: u32) -> Self {
        let action_height = action_height.min(bounds.height);
        let action_y = bounds
            .y
            .saturating_add(bounds.height.saturating_sub(action_height));
        let first_width = bounds.width / 2;
        let second_width = bounds.width.saturating_sub(first_width);
        Self {
            notice: bounds,
            open_log: PaneRect::new(bounds.x, action_y, first_width, action_height),
            dismiss: PaneRect::new(
                bounds.x.saturating_add(first_width),
                action_y,
                second_width,
                action_height,
            ),
        }
    }

    pub fn notice(&self) -> PaneRect {
        self.notice
    }

    pub fn open_log(&self) -> PaneRect {
        self.open_log
    }

    pub fn dismiss(&self) -> PaneRect {
        self.dismiss
    }

    pub fn open_log_contains(&self, x: f64, y: f64) -> bool {
        self.open_log.contains(x, y)
    }

    pub fn dismiss_contains(&self, x: f64, y: f64) -> bool {
        self.dismiss.contains(x, y)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkspaceStripLayout {
    workspaces: Vec<WorkspacePlacement>,
}

impl WorkspaceStripLayout {
    pub fn calculate(workspaces: &[WorkspaceId], bounds: PaneRect, preferred_width: u32) -> Self {
        if workspaces.is_empty() {
            return Self::default();
        }
        let available_per_workspace = bounds.width.div_ceil(workspaces.len() as u32);
        let width = preferred_width.min(available_per_workspace).max(1);
        let workspaces = workspaces
            .iter()
            .enumerate()
            .map(|(index, workspace)| {
                let x = bounds
                    .x
                    .saturating_add((index as u32).saturating_mul(width));
                WorkspacePlacement {
                    workspace: *workspace,
                    rect: PaneRect::new(
                        x,
                        bounds.y,
                        width.min(bounds.x.saturating_add(bounds.width).saturating_sub(x)),
                        bounds.height,
                    ),
                }
            })
            .collect();
        Self { workspaces }
    }

    pub fn workspaces(&self) -> &[WorkspacePlacement] {
        &self.workspaces
    }

    pub fn workspace_at(&self, x: f64, y: f64) -> Option<WorkspaceId> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.rect.contains(x, y))
            .map(|workspace| workspace.workspace)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TabStripLayout {
    tabs: Vec<TabPlacement>,
}

impl TabStripLayout {
    pub fn calculate(tabs: &[TabId], bounds: PaneRect, preferred_width: u32) -> Self {
        if tabs.is_empty() {
            return Self::default();
        }
        let available_per_tab = bounds.width.div_ceil(tabs.len() as u32);
        let tab_width = preferred_width.min(available_per_tab).max(1);
        let tabs = tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let x = bounds
                    .x
                    .saturating_add((index as u32).saturating_mul(tab_width));
                TabPlacement {
                    tab: *tab,
                    rect: PaneRect::new(
                        x,
                        bounds.y,
                        tab_width.min(bounds.x.saturating_add(bounds.width).saturating_sub(x)),
                        bounds.height,
                    ),
                }
            })
            .collect();
        Self { tabs }
    }

    pub fn tabs(&self) -> &[TabPlacement] {
        &self.tabs
    }

    pub fn tab_at(&self, x: f64, y: f64) -> Option<TabId> {
        self.tabs
            .iter()
            .find(|tab| tab.rect.contains(x, y))
            .map(|tab| tab.tab)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PaneLayout {
    panes: Vec<PanePlacement>,
    boundaries: Vec<SplitBoundary>,
}

impl PaneLayout {
    pub fn calculate(root: &PaneNode, bounds: PaneRect, divider_width: u32) -> Self {
        let mut layout = Self::default();
        layout.visit(root, bounds, divider_width);
        layout
    }

    pub fn panes(&self) -> &[PanePlacement] {
        &self.panes
    }

    pub fn boundaries(&self) -> &[SplitBoundary] {
        &self.boundaries
    }

    pub fn rect(&self, pane: PaneId) -> Option<PaneRect> {
        self.panes
            .iter()
            .find(|placement| placement.pane == pane)
            .map(|placement| placement.rect)
    }

    pub fn pane_at(&self, x: f64, y: f64) -> Option<PaneId> {
        self.panes
            .iter()
            .find(|placement| placement.rect.contains(x, y))
            .map(|placement| placement.pane)
    }

    pub fn boundary_at(&self, x: f64, y: f64) -> Option<SplitBoundary> {
        self.boundaries
            .iter()
            .find(|boundary| boundary.rect.contains(x, y))
            .copied()
    }

    pub fn neighbor(&self, pane: PaneId, direction: SplitDirection) -> Option<PaneId> {
        let source = self.rect(pane)?;
        let (source_x, source_y) = source.center();
        self.panes
            .iter()
            .filter(|candidate| candidate.pane != pane)
            .filter_map(|candidate| {
                let (x, y) = candidate.rect.center();
                let (primary, secondary) = match direction {
                    SplitDirection::Left if x < source_x => (source_x - x, (source_y - y).abs()),
                    SplitDirection::Right if x > source_x => (x - source_x, (source_y - y).abs()),
                    SplitDirection::Up if y < source_y => (source_y - y, (source_x - x).abs()),
                    SplitDirection::Down if y > source_y => (y - source_y, (source_x - x).abs()),
                    _ => return None,
                };
                Some((primary + secondary * 2.0, candidate.pane))
            })
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .map(|(_, pane)| pane)
    }

    fn visit(&mut self, node: &PaneNode, bounds: PaneRect, divider_width: u32) {
        match node {
            PaneNode::Leaf(pane) => self.panes.push(PanePlacement {
                pane: *pane,
                rect: bounds,
            }),
            PaneNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let ratio = ratio.clamp(0.05, 0.95);
                match direction {
                    SplitDirection::Left | SplitDirection::Right => {
                        let divider = divider_width.min(bounds.width);
                        let available = bounds.width.saturating_sub(divider);
                        let first_width = (available as f32 * ratio).round() as u32;
                        let second_width = available.saturating_sub(first_width);
                        let second_x = bounds.x.saturating_add(first_width).saturating_add(divider);
                        self.visit(
                            first,
                            PaneRect::new(bounds.x, bounds.y, first_width, bounds.height),
                            divider_width,
                        );
                        self.boundaries.push(SplitBoundary {
                            axis: SplitAxis::Vertical,
                            rect: PaneRect::new(
                                bounds.x.saturating_add(first_width),
                                bounds.y,
                                divider,
                                bounds.height,
                            ),
                        });
                        self.visit(
                            second,
                            PaneRect::new(second_x, bounds.y, second_width, bounds.height),
                            divider_width,
                        );
                    }
                    SplitDirection::Up | SplitDirection::Down => {
                        let divider = divider_width.min(bounds.height);
                        let available = bounds.height.saturating_sub(divider);
                        let first_height = (available as f32 * ratio).round() as u32;
                        let second_height = available.saturating_sub(first_height);
                        let second_y = bounds
                            .y
                            .saturating_add(first_height)
                            .saturating_add(divider);
                        self.visit(
                            first,
                            PaneRect::new(bounds.x, bounds.y, bounds.width, first_height),
                            divider_width,
                        );
                        self.boundaries.push(SplitBoundary {
                            axis: SplitAxis::Horizontal,
                            rect: PaneRect::new(
                                bounds.x,
                                bounds.y.saturating_add(first_height),
                                bounds.width,
                                divider,
                            ),
                        });
                        self.visit(
                            second,
                            PaneRect::new(bounds.x, second_y, bounds.width, second_height),
                            divider_width,
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Cases(u64);

    impl Cases {
        fn next(&mut self) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            (self.0 >> 32) as u32
        }

        fn below(&mut self, upper: u32) -> u32 {
            self.next() % upper
        }
    }

    fn generated_tree(cases: &mut Cases, next_pane: &mut u64, depth: u8) -> PaneNode {
        if depth == 0 || cases.below(4) == 0 {
            let pane = PaneId(*next_pane);
            *next_pane += 1;
            return PaneNode::Leaf(pane);
        }

        let direction = match cases.below(4) {
            0 => SplitDirection::Left,
            1 => SplitDirection::Right,
            2 => SplitDirection::Up,
            _ => SplitDirection::Down,
        };
        PaneNode::Split {
            direction,
            // Include values outside the accepted range to exercise clamping.
            ratio: cases.below(141) as f32 / 100.0 - 0.2,
            first: Box::new(generated_tree(cases, next_pane, depth - 1)),
            second: Box::new(generated_tree(cases, next_pane, depth - 1)),
        }
    }

    fn rect_is_inside(inner: PaneRect, outer: PaneRect) -> bool {
        inner.x >= outer.x
            && inner.y >= outer.y
            && inner.x.saturating_add(inner.width) <= outer.x.saturating_add(outer.width)
            && inner.y.saturating_add(inner.height) <= outer.y.saturating_add(outer.height)
    }

    fn rects_overlap(left: PaneRect, right: PaneRect) -> bool {
        left.x < right.x.saturating_add(right.width)
            && right.x < left.x.saturating_add(left.width)
            && left.y < right.y.saturating_add(right.height)
            && right.y < left.y.saturating_add(left.height)
    }

    fn split(direction: SplitDirection, ratio: f32) -> PaneNode {
        PaneNode::Split {
            direction,
            ratio,
            first: Box::new(PaneNode::Leaf(PaneId(1))),
            second: Box::new(PaneNode::Leaf(PaneId(2))),
        }
    }

    #[test]
    fn lays_out_left_and_right_splits_with_a_separate_boundary() {
        let layout = PaneLayout::calculate(
            &split(SplitDirection::Right, 0.25),
            PaneRect::new(0, 0, 100, 40),
            2,
        );
        assert_eq!(layout.rect(PaneId(1)), Some(PaneRect::new(0, 0, 25, 40)));
        assert_eq!(layout.rect(PaneId(2)), Some(PaneRect::new(27, 0, 73, 40)));
        assert_eq!(
            layout.boundaries(),
            &[SplitBoundary {
                axis: SplitAxis::Vertical,
                rect: PaneRect::new(25, 0, 2, 40),
            }]
        );
    }

    #[test]
    fn lays_out_up_and_down_splits() {
        let layout = PaneLayout::calculate(
            &split(SplitDirection::Down, 0.5),
            PaneRect::new(10, 20, 80, 42),
            2,
        );
        assert_eq!(layout.rect(PaneId(1)), Some(PaneRect::new(10, 20, 80, 20)));
        assert_eq!(layout.rect(PaneId(2)), Some(PaneRect::new(10, 42, 80, 20)));
    }

    #[test]
    fn hit_tests_panes_and_boundaries_independently() {
        let layout = PaneLayout::calculate(
            &split(SplitDirection::Right, 0.5),
            PaneRect::new(0, 0, 100, 50),
            4,
        );
        assert_eq!(layout.pane_at(10.0, 10.0), Some(PaneId(1)));
        assert_eq!(layout.pane_at(90.0, 10.0), Some(PaneId(2)));
        assert_eq!(layout.pane_at(49.0, 10.0), None);
        assert!(layout.boundary_at(49.0, 10.0).is_some());
    }

    #[test]
    fn finds_the_nearest_pane_in_a_direction() {
        let root = PaneNode::Split {
            direction: SplitDirection::Right,
            ratio: 0.5,
            first: Box::new(PaneNode::Leaf(PaneId(1))),
            second: Box::new(PaneNode::Split {
                direction: SplitDirection::Down,
                ratio: 0.5,
                first: Box::new(PaneNode::Leaf(PaneId(2))),
                second: Box::new(PaneNode::Leaf(PaneId(3))),
            }),
        };
        let layout = PaneLayout::calculate(&root, PaneRect::new(0, 0, 200, 100), 2);
        assert_eq!(
            layout.neighbor(PaneId(1), SplitDirection::Right),
            Some(PaneId(2))
        );
        assert_eq!(
            layout.neighbor(PaneId(2), SplitDirection::Down),
            Some(PaneId(3))
        );
    }

    #[test]
    fn generated_split_layouts_preserve_geometry_and_directional_focus() {
        let mut cases = Cases(0x5eed_f00d_cafe_beef);

        for case in 0..256 {
            let mut next_pane = 1;
            let tree = generated_tree(&mut cases, &mut next_pane, 4);
            let bounds = PaneRect::new(
                cases.below(40),
                cases.below(40),
                80 + cases.below(720),
                60 + cases.below(540),
            );
            let layout = PaneLayout::calculate(&tree, bounds, cases.below(7));
            let expected_panes = tree.panes();

            assert_eq!(layout.panes().len(), expected_panes.len(), "case {case}");
            assert_eq!(
                layout.boundaries().len(),
                expected_panes.len().saturating_sub(1),
                "case {case}"
            );
            for placement in layout.panes() {
                assert!(expected_panes.contains(&placement.pane), "case {case}");
                assert!(rect_is_inside(placement.rect, bounds), "case {case}");
                if placement.rect.width > 0 && placement.rect.height > 0 {
                    let (x, y) = placement.rect.center();
                    assert_eq!(layout.pane_at(x, y), Some(placement.pane), "case {case}");
                }
            }
            for (index, left) in layout.panes().iter().enumerate() {
                for right in &layout.panes()[index + 1..] {
                    assert!(
                        !rects_overlap(left.rect, right.rect),
                        "panes overlap in case {case}: {left:?} and {right:?}"
                    );
                }
            }
            for boundary in layout.boundaries() {
                assert!(rect_is_inside(boundary.rect, bounds), "case {case}");
            }

            for placement in layout.panes() {
                let (source_x, source_y) = placement.rect.center();
                for direction in [
                    SplitDirection::Left,
                    SplitDirection::Right,
                    SplitDirection::Up,
                    SplitDirection::Down,
                ] {
                    let Some(neighbor) = layout.neighbor(placement.pane, direction) else {
                        continue;
                    };
                    let (neighbor_x, neighbor_y) = layout
                        .rect(neighbor)
                        .expect("a neighbor is part of the layout")
                        .center();
                    let points_in_direction = match direction {
                        SplitDirection::Left => neighbor_x < source_x,
                        SplitDirection::Right => neighbor_x > source_x,
                        SplitDirection::Up => neighbor_y < source_y,
                        SplitDirection::Down => neighbor_y > source_y,
                    };
                    assert!(points_in_direction, "case {case}, direction {direction:?}");
                }
            }
        }
    }

    #[test]
    fn lays_out_and_hit_tests_a_tab_strip() {
        let tabs = [TabId(1), TabId(2), TabId(3)];
        let layout = TabStripLayout::calculate(&tabs, PaneRect::new(0, 0, 300, 30), 120);
        assert_eq!(layout.tabs()[0].rect, PaneRect::new(0, 0, 100, 30));
        assert_eq!(layout.tabs()[2].rect, PaneRect::new(200, 0, 100, 30));
        assert_eq!(layout.tab_at(150.0, 10.0), Some(TabId(2)));
        assert_eq!(layout.tab_at(150.0, 31.0), None);
    }

    #[test]
    fn lays_out_and_hit_tests_a_workspace_strip() {
        let workspaces = [WorkspaceId(1), WorkspaceId(5)];
        let layout =
            WorkspaceStripLayout::calculate(&workspaces, PaneRect::new(0, 0, 400, 24), 140);
        assert_eq!(layout.workspaces()[1].rect, PaneRect::new(140, 0, 140, 24));
        assert_eq!(layout.workspace_at(160.0, 10.0), Some(WorkspaceId(5)));
    }

    #[test]
    fn lays_out_config_error_actions_without_gaps() {
        let layout = ConfigErrorLayout::calculate(PaneRect::new(0, 54, 401, 120), 30);

        assert_eq!(layout.notice(), PaneRect::new(0, 54, 401, 120));
        assert_eq!(layout.open_log(), PaneRect::new(0, 144, 200, 30));
        assert_eq!(layout.dismiss(), PaneRect::new(200, 144, 201, 30));
        assert!(layout.open_log_contains(199.0, 150.0));
        assert!(layout.dismiss_contains(400.0, 150.0));
    }
}
