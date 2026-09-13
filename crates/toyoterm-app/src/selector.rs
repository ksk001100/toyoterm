const PAGE_SIZE: usize = 10;

#[derive(Clone, Debug)]
pub(super) struct SelectorOverlay {
    pub(super) id: u64,
    pub(super) title: String,
    pub(super) items: Vec<String>,
    pub(super) query: String,
    selected: usize,
}

impl SelectorOverlay {
    pub(super) fn new(id: u64, title: String, items: Vec<String>) -> Self {
        Self {
            id,
            title,
            items,
            query: String::new(),
            selected: 0,
        }
    }

    pub(super) fn append_query(&mut self, text: &str) {
        self.query.push_str(text);
        self.selected = 0;
    }

    pub(super) fn pop_query(&mut self) {
        self.query.pop();
        self.selected = 0;
    }

    pub(super) fn move_previous(&mut self) {
        let count = self.filtered_indices().len();
        if count > 0 {
            self.selected = self.selected.checked_sub(1).unwrap_or(count - 1);
        }
    }

    pub(super) fn move_next(&mut self) {
        let count = self.filtered_indices().len();
        if count > 0 {
            self.selected = (self.selected + 1) % count;
        }
    }

    pub(super) fn page_previous(&mut self) {
        self.selected = self.selected.saturating_sub(PAGE_SIZE);
    }

    pub(super) fn page_next(&mut self) {
        let count = self.filtered_indices().len();
        if count > 0 {
            self.selected = self.selected.saturating_add(PAGE_SIZE).min(count - 1);
        }
    }

    pub(super) fn move_first(&mut self) {
        self.selected = 0;
    }

    pub(super) fn move_last(&mut self) {
        self.selected = self.filtered_indices().len().saturating_sub(1);
    }

    pub(super) fn selected_item(&self) -> Option<String> {
        let indices = self.filtered_indices();
        indices
            .get(self.selected)
            .and_then(|index| self.items.get(*index))
            .cloned()
    }

    pub(super) fn render_lines(&self, max_items: usize) -> SelectorLines<'_> {
        let indices = self.filtered_indices();
        let count = indices.len();
        let max_items = max_items.max(1);
        let start = self
            .selected
            .saturating_sub(max_items / 2)
            .min(count.saturating_sub(max_items));
        let items = indices
            .into_iter()
            .skip(start)
            .take(max_items)
            .filter_map(|index| self.items.get(index).map(String::as_str))
            .collect();
        SelectorLines {
            items,
            selected: (count > 0).then_some(self.selected.saturating_sub(start)),
            total: count,
        }
    }

    fn filtered_indices(&self) -> Vec<usize> {
        let query = self.query.to_lowercase();
        self.items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| item.to_lowercase().contains(&query).then_some(index))
            .collect()
    }
}

pub(super) struct SelectorLines<'a> {
    pub(super) items: Vec<&'a str>,
    pub(super) selected: Option<usize>,
    pub(super) total: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selector() -> SelectorOverlay {
        SelectorOverlay::new(
            7,
            "Theme".into(),
            vec![
                "Ayu".into(),
                "Solarized Dark".into(),
                "Solarized Light".into(),
            ],
        )
    }

    #[test]
    fn filters_case_insensitively_and_resets_selection() {
        let mut selector = selector();
        selector.move_next();
        selector.append_query("LIGHT");
        assert_eq!(selector.selected_item().as_deref(), Some("Solarized Light"));
        assert_eq!(selector.render_lines(10).total, 1);
    }

    #[test]
    fn navigation_wraps_and_pages_within_results() {
        let mut selector = selector();
        selector.move_previous();
        assert_eq!(selector.selected_item().as_deref(), Some("Solarized Light"));
        selector.move_next();
        assert_eq!(selector.selected_item().as_deref(), Some("Ayu"));
        selector.page_next();
        assert_eq!(selector.selected_item().as_deref(), Some("Solarized Light"));
        selector.move_first();
        selector.move_last();
        assert_eq!(selector.selected_item().as_deref(), Some("Solarized Light"));
    }

    #[test]
    fn reports_no_selection_when_filter_has_no_matches() {
        let mut selector = selector();
        selector.append_query("missing");
        assert_eq!(selector.selected_item(), None);
        assert_eq!(selector.render_lines(10).selected, None);
    }
}
