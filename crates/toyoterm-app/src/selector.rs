use std::time::{Duration, Instant};
use toyoterm_api::InputPrompt;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const PAGE_SIZE: usize = 10;
const MAX_INPUT_BYTES: usize = 4096;

pub(super) struct VisibleInputLine {
    pub(super) text: String,
    pub(super) caret_column: Option<usize>,
}

#[derive(Clone, Debug)]
struct CompletionState {
    revision: u64,
    query: String,
    pending: Option<u64>,
    deadline: Option<Instant>,
    ready: bool,
    error: bool,
}

#[derive(Clone, Debug)]
struct PromptState {
    require_match: bool,
    history: Vec<String>,
    history_index: Option<usize>,
    draft: String,
    completion: Option<CompletionState>,
}

#[derive(Clone, Debug)]
pub(super) struct SelectorOverlay {
    pub(super) id: u64,
    pub(super) title: String,
    pub(super) items: Vec<String>,
    pub(super) query: String,
    selected: usize,
    cursor: usize,
    prompt: Option<PromptState>,
    panel: bool,
}

impl SelectorOverlay {
    pub(super) fn new(id: u64, title: String, items: Vec<String>) -> Self {
        Self {
            id,
            title,
            items,
            query: String::new(),
            selected: 0,
            cursor: 0,
            prompt: None,
            panel: false,
        }
    }

    pub(super) fn prompt(id: u64, input: InputPrompt) -> Self {
        let mut overlay = Self::new(id, input.title, input.completions);
        overlay.cursor = input.initial.len();
        overlay.query = input.initial;
        overlay.prompt = Some(PromptState {
            require_match: input.require_match,
            history: input.history,
            history_index: None,
            draft: String::new(),
            completion: input.dynamic.then(|| CompletionState {
                revision: 0,
                query: overlay.query.clone(),
                pending: None,
                deadline: Some(Instant::now()),
                ready: false,
                error: false,
            }),
        });
        overlay
    }

    pub(super) fn is_prompt(&self) -> bool {
        self.prompt.is_some()
    }

    pub(super) fn panel(id: u64, title: String, lines: Vec<String>) -> Self {
        let mut overlay = Self::new(id, title, lines);
        overlay.panel = true;
        overlay
    }

    pub(super) fn is_panel(&self) -> bool {
        self.panel
    }

    pub(super) fn update_panel(&mut self, id: u64, title: Option<String>, lines: Vec<String>) {
        if self.id != id || !self.panel {
            return;
        }
        if let Some(title) = title {
            self.title = title;
        }
        self.items = lines;
        self.selected = self
            .selected
            .min(self.filtered_indices().len().saturating_sub(1));
    }

    pub(super) fn result(&self) -> Option<String> {
        if self.panel {
            return None;
        }
        match &self.prompt {
            Some(prompt) if !prompt.require_match => Some(self.query.clone()),
            _ => self.selected_item(),
        }
    }

    fn changed(&mut self) {
        self.selected = 0;
        if let Some(prompt) = self.prompt.as_mut() {
            prompt.history_index = None;
            prompt.draft.clear();
        }
        self.invalidate_completion();
    }

    fn invalidate_completion(&mut self) {
        if let Some(completion) = self
            .prompt
            .as_mut()
            .and_then(|prompt| prompt.completion.as_mut())
            && completion.query != self.query
        {
            completion.query = self.query.clone();
            completion.revision = completion.revision.wrapping_add(1);
            completion.ready = false;
            completion.error = false;
            completion.deadline = Some(Instant::now() + Duration::from_millis(80));
            self.items.clear();
        }
    }

    pub(super) fn completion_deadline(&self) -> Option<Instant> {
        let completion = self.prompt.as_ref()?.completion.as_ref()?;
        if completion.pending.is_some() {
            None
        } else {
            completion.deadline
        }
    }

    pub(super) fn take_completion(&mut self, now: Instant) -> Option<(u64, String)> {
        let completion = self.prompt.as_mut()?.completion.as_mut()?;
        if completion.pending.is_some() || completion.deadline.is_none_or(|deadline| deadline > now)
        {
            return None;
        }
        completion.deadline = None;
        completion.pending = Some(completion.revision);
        Some((completion.revision, self.query.clone()))
    }

    pub(super) fn finish_completion(&mut self, id: u64, revision: u64, success: bool) {
        if self.id != id {
            return;
        }
        let Some(completion) = self
            .prompt
            .as_mut()
            .and_then(|prompt| prompt.completion.as_mut())
        else {
            return;
        };
        if completion.pending == Some(revision) {
            completion.pending = None;
        }
        if completion.revision == revision {
            completion.ready = true;
            completion.error = !success;
        }
    }

    pub(super) fn update_completions(&mut self, id: u64, revision: u64, items: Vec<String>) {
        if self.id == id
            && self
                .prompt
                .as_ref()
                .and_then(|prompt| prompt.completion.as_ref())
                .is_some_and(|completion| completion.revision == revision)
        {
            self.finish_completion(id, revision, true);
            self.items = items;
            self.selected = 0;
        }
    }

    pub(super) fn completion_status(&self) -> Option<&'static str> {
        let completion = self.prompt.as_ref()?.completion.as_ref()?;
        if completion.error {
            Some("Completion failed — edit to retry")
        } else if !completion.ready {
            Some("Updating completions…")
        } else {
            None
        }
    }

    pub(super) fn append_query(&mut self, text: &str) {
        if text.chars().any(char::is_control)
            || self.query.len().saturating_add(text.len()) > MAX_INPUT_BYTES
        {
            return;
        }
        self.query.insert_str(self.cursor, text);
        self.cursor += text.len();
        if self.cursor < self.query.len()
            && !self
                .query
                .grapheme_indices(true)
                .any(|(index, _)| index == self.cursor)
        {
            self.cursor = self.next_cursor();
        }
        self.changed();
    }

    pub(super) fn pop_query(&mut self) {
        let previous = self.previous_cursor();
        self.query.replace_range(previous..self.cursor, "");
        self.cursor = previous;
        self.changed();
    }

    fn previous_cursor(&self) -> usize {
        self.query
            .grapheme_indices(true)
            .take_while(|(index, _)| *index < self.cursor)
            .last()
            .map_or(0, |(index, _)| index)
    }

    fn next_cursor(&self) -> usize {
        self.query
            .grapheme_indices(true)
            .find(|(index, _)| *index > self.cursor)
            .map_or(self.query.len(), |(index, _)| index)
    }

    pub(super) fn move_cursor(&mut self, forward: bool) {
        self.cursor = if forward {
            self.next_cursor()
        } else {
            self.previous_cursor()
        };
    }

    pub(super) fn cursor_edge(&mut self, end: bool) {
        self.cursor = if end { self.query.len() } else { 0 };
    }

    pub(super) fn delete_query(&mut self) {
        let next = self.next_cursor();
        self.query.replace_range(self.cursor..next, "");
        self.changed();
    }

    pub(super) fn clear_query(&mut self, before: bool) {
        if before {
            self.query.replace_range(..self.cursor, "");
            self.cursor = 0;
        } else {
            self.query.truncate(self.cursor);
        }
        self.changed();
    }

    pub(super) fn complete(&mut self) {
        if let Some(item) = self.selected_item() {
            self.cursor = item.len();
            self.query = item;
            self.changed();
        }
    }

    pub(super) fn recall_history(&mut self, previous: bool) {
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        let count = prompt.history.len();
        if count == 0 {
            return;
        }
        if previous {
            let index = match prompt.history_index {
                Some(index) => index.saturating_sub(1),
                None => {
                    prompt.draft = self.query.clone();
                    count - 1
                }
            };
            prompt.history_index = Some(index);
            self.query = prompt.history[index].clone();
        } else {
            match prompt.history_index {
                Some(index) if index + 1 < count => {
                    prompt.history_index = Some(index + 1);
                    self.query = prompt.history[index + 1].clone();
                }
                Some(_) => {
                    prompt.history_index = None;
                    self.query = prompt.draft.clone();
                }
                None => return,
            }
        }
        self.cursor = self.query.len();
        self.selected = 0;
        self.invalidate_completion();
    }

    pub(super) fn input_line(&self, preedit: Option<&str>) -> String {
        format!(
            "{}{}▏{}",
            &self.query[..self.cursor],
            preedit.unwrap_or(""),
            &self.query[self.cursor..]
        )
    }

    pub(super) fn visible_input_line(
        &self,
        preedit: Option<&str>,
        max_columns: usize,
    ) -> VisibleInputLine {
        if max_columns == 0 {
            return VisibleInputLine {
                text: String::new(),
                caret_column: None,
            };
        }
        let line = self.input_line(preedit);
        let caret_byte = self.cursor + preedit.unwrap_or("").len();
        let graphemes: Vec<_> = line.grapheme_indices(true).collect();
        let widths: Vec<_> = graphemes.iter().map(|(_, text)| text.width()).collect();
        if widths.iter().sum::<usize>() <= max_columns {
            return VisibleInputLine {
                caret_column: Some(line[..caret_byte].width()),
                text: line,
            };
        }
        let caret = graphemes
            .iter()
            .position(|(byte, _)| *byte == caret_byte)
            .unwrap_or(0);
        // Reserve both continuation marks. Tiny views retain just the caret.
        if max_columns < 3 {
            return VisibleInputLine {
                text: "▏".into(),
                caret_column: Some(0),
            };
        }
        let available = max_columns - 2;
        let mut start = caret;
        let mut end = caret + 1;
        let mut used = widths[caret];
        let left_target = available.saturating_sub(used) / 2;
        let mut left_width = 0;
        while start > 0 && left_width + widths[start - 1] <= left_target {
            start -= 1;
            left_width += widths[start];
            used += widths[start];
        }
        while end < graphemes.len() && used + widths[end] <= available {
            used += widths[end];
            end += 1;
        }
        while start > 0 && used + widths[start - 1] <= available {
            start -= 1;
            used += widths[start];
        }
        let mut visible = String::new();
        if start > 0 {
            visible.push('…');
        }
        for (_, grapheme) in &graphemes[start..end] {
            visible.push_str(grapheme);
        }
        if end < graphemes.len() {
            visible.push('…');
        }
        VisibleInputLine {
            text: visible,
            caret_column: Some(usize::from(start > 0) + widths[start..caret].iter().sum::<usize>()),
        }
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
        let mut indices: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                (self
                    .prompt
                    .as_ref()
                    .is_some_and(|prompt| prompt.completion.is_some())
                    || item.to_lowercase().contains(&query))
                .then_some(index)
            })
            .collect();
        if self.is_prompt()
            && let Some(exact) = indices
                .iter()
                .position(|index| self.items[*index] == self.query)
        {
            let index = indices.remove(exact);
            indices.insert(0, index);
        }
        indices
    }
}

impl crate::ToyotermApplication {
    pub(super) fn dispatch_prompt_completion(&mut self, now: Instant) -> Option<Instant> {
        if self.scripting.in_flight
            || !self.scripting.pending.is_empty()
            || self.ui.ime_preedit.is_some()
        {
            return None;
        }
        let selector = self.ui.selector.as_mut()?;
        if let Some((revision, query)) = selector.take_completion(now) {
            let id = selector.id;
            if let Err(error) =
                self.submit_script(toyoterm_script::ScriptInvocation::CompletePrompt {
                    id,
                    revision,
                    query,
                })
            {
                if let Some(selector) = self.ui.selector.as_mut() {
                    selector.finish_completion(id, revision, false);
                }
                tracing::warn!(target: "toyoterm::script", %error, "submit completion failed");
            }
            return None;
        }
        selector.completion_deadline()
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

    fn prompt(initial: &str, require_match: bool) -> SelectorOverlay {
        SelectorOverlay::prompt(
            9,
            InputPrompt {
                initial: initial.into(),
                completions: vec!["猫の本".into(), "猫の歌".into()],
                history: vec!["older".into(), "newer".into()],
                require_match,
                ..InputPrompt::default()
            },
        )
    }

    #[test]
    fn prompts_edit_unicode_at_point_and_delete_whole_graphemes() {
        let mut input = prompt("a👩‍💻e\u{301}猫", false);
        input.move_cursor(false);
        input.pop_query();
        assert_eq!(input.query, "a👩‍💻猫");
        input.move_cursor(false);
        input.delete_query();
        assert_eq!(input.query, "a猫");
        input.append_query("犬");
        assert_eq!(input.input_line(Some("変換")), "a犬変換▏猫");
        input.cursor_edge(false);
        input.append_query("先頭");
        assert_eq!(input.query, "先頭a犬猫");
        input.clear_query(false);
        assert_eq!(input.query, "先頭");
        input.clear_query(true);
        assert_eq!(input.result(), Some(String::new()));
    }

    #[test]
    fn prompts_accept_free_text_or_matching_candidates_and_complete_with_tab() {
        let mut input = prompt("猫", true);
        input.move_next();
        assert_eq!(input.result().as_deref(), Some("猫の歌"));
        input.complete();
        assert_eq!(input.query, "猫の歌");
        input.append_query("x");
        assert_eq!(input.result(), None);
        let input = prompt("unlisted", false);
        assert_eq!(input.result().as_deref(), Some("unlisted"));
        let mut exact = SelectorOverlay::prompt(
            10,
            InputPrompt {
                initial: "cat".into(),
                completions: vec!["cats".into(), "cat".into()],
                require_match: true,
                ..InputPrompt::default()
            },
        );
        assert_eq!(exact.result().as_deref(), Some("cat"));
        exact.move_next();
        assert_eq!(exact.result().as_deref(), Some("cats"));
        exact.complete();
        assert_eq!(exact.result().as_deref(), Some("cats"));
    }

    #[test]
    fn prompt_history_restores_the_draft_and_does_not_mutate_the_stored_entries() {
        let mut input = prompt("draft", false);
        input.recall_history(true);
        assert_eq!(input.query, "newer");
        input.recall_history(true);
        input.recall_history(true);
        assert_eq!(input.query, "older");
        input.recall_history(false);
        assert_eq!(input.query, "newer");
        input.recall_history(false);
        assert_eq!(input.query, "draft");
        input.recall_history(true);
        input.append_query(" edit");
        input.recall_history(true);
        assert_eq!(input.query, "newer");
        input.recall_history(false);
        assert_eq!(input.query, "newer edit");
    }

    #[test]
    fn prompt_input_is_bounded_and_multiline_paste_cannot_inject_controls() {
        let mut input = prompt("", false);
        input.append_query("bad\nline");
        assert_eq!(input.query, "");
        input.append_query(&"猫".repeat(1365));
        assert_eq!(input.query.len(), 4095);
        input.append_query("猫");
        assert_eq!(input.query.len(), 4095);
        input.append_query("x");
        assert_eq!(input.query.len(), 4096);
        input.pop_query();
        assert_eq!(input.query.len(), 4095);
    }

    #[test]
    fn insertion_that_joins_a_grapheme_keeps_point_on_its_boundary() {
        let mut input = prompt("\u{301}", false);
        input.cursor_edge(false);
        input.append_query("e");
        assert_eq!(input.input_line(None), "e\u{301}▏");
        input.pop_query();
        assert_eq!(input.query, "");
    }

    #[test]
    fn long_input_keeps_point_and_preedit_visible_without_splitting_graphemes() {
        let mut input = prompt(&"猫".repeat(60), false);
        input.cursor_edge(false);
        for _ in 0..30 {
            input.move_cursor(true);
        }
        let visible = input.visible_input_line(Some("変換"), 12).text;
        assert!(visible.contains("変換▏"));
        assert!(visible.starts_with('…') && visible.ends_with('…'));
        assert!(visible.width() <= 12);
    }

    #[test]
    fn visible_input_respects_display_columns_at_every_cursor_and_narrow_width() {
        for text in ["ascii input", "猫開発環境", "e\u{301}👩‍💻🇯🇵界", "\u{301}猫"]
        {
            let mut input = prompt(text, false);
            input.cursor_edge(false);
            let count = text.graphemes(true).count();
            for _ in 0..=count {
                for preedit in [None, Some("変換👩‍💻")] {
                    for columns in 0..24 {
                        let view = input.visible_input_line(preedit, columns);
                        let visible = view.text;
                        assert!(visible.width() <= columns, "{visible:?} exceeds {columns}");
                        if columns > 0 {
                            assert!(visible.contains('▏'), "caret lost: {visible:?}");
                            assert!(view.caret_column.is_some_and(|column| column < columns));
                        } else {
                            assert!(view.caret_column.is_none());
                        }
                        assert!(!visible.starts_with('\u{200d}') && !visible.ends_with('\u{200d}'));
                    }
                }
                input.move_cursor(true);
            }
        }
    }

    #[test]
    fn dynamic_completion_coalesces_edits_and_rejects_stale_results() {
        let mut input = SelectorOverlay::prompt(
            12,
            InputPrompt {
                dynamic: true,
                require_match: true,
                ..InputPrompt::default()
            },
        );
        let now = Instant::now() + Duration::from_secs(1);
        assert_eq!(input.take_completion(now), Some((0, String::new())));
        input.append_query("a");
        input.append_query("b");
        assert_eq!(input.take_completion(now), None);
        assert_eq!(input.result(), None);
        input.finish_completion(12, 0, true);
        input.update_completions(12, 0, vec!["old".into()]);
        assert!(input.items.is_empty());
        let (revision, query) = input.take_completion(now).unwrap();
        assert_eq!((revision, query.as_str()), (2, "ab"));
        input.update_completions(99, revision, vec!["wrong prompt".into()]);
        assert!(input.items.is_empty());
        // Dynamic providers may implement fuzzy matching; native lookup must
        // preserve results that do not contain the literal query.
        input.update_completions(12, revision, vec!["猫".into()]);
        assert_eq!(input.result().as_deref(), Some("猫"));
        input.move_cursor(false);
        assert_eq!(input.completion_deadline(), None);
        input.append_query("c");
        assert_eq!(input.result(), None);
    }

    #[test]
    fn failed_completion_retries_after_editing_and_history_invalidates_candidates() {
        let mut input = SelectorOverlay::prompt(
            13,
            InputPrompt {
                dynamic: true,
                history: vec!["older".into()],
                ..InputPrompt::default()
            },
        );
        let now = Instant::now() + Duration::from_secs(1);
        input.take_completion(now).unwrap();
        input.finish_completion(13, 0, false);
        assert!(input.completion_status().unwrap().contains("failed"));
        assert!(input.take_completion(now).is_none());
        input.append_query("new");
        assert_eq!(input.completion_status(), Some("Updating completions…"));
        let (revision, _) = input.take_completion(now).unwrap();
        input.update_completions(13, revision, vec!["newer".into()]);
        input.recall_history(true);
        assert!(input.items.is_empty());
        assert_eq!(input.take_completion(now).unwrap().1, "older");
    }
}
