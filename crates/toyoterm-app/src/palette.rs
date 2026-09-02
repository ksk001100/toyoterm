use toyoterm_api::SplitDirection;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PaletteAction {
    ReloadConfig,
    NewTab,
    MaximizeWindow,
    ToggleMaximize,
    MinimizeWindow,
    ToggleFullscreen,
    Split(SplitDirection),
    ClosePane,
    SwitchWorkspace(String),
    RubyConsole,
    UserCommand(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaletteItem {
    pub label: String,
    pub action: PaletteAction,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommandPalette {
    open: bool,
    ruby_console: bool,
    query: String,
    console_output: Vec<String>,
    selected: usize,
}

impl CommandPalette {
    pub fn open(&mut self) {
        self.open = true;
        self.ruby_console = false;
        self.query.clear();
        self.selected = 0;
    }

    pub fn open_console(&mut self) {
        self.open = true;
        self.ruby_console = true;
        self.query.clear();
        self.selected = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.selected = 0;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }
    pub fn is_console(&self) -> bool {
        self.open && self.ruby_console
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn selected(&self) -> usize {
        self.selected
    }
    pub fn console_output(&self) -> &[String] {
        &self.console_output
    }

    pub fn insert(&mut self, text: &str) {
        self.query.push_str(text);
        self.selected = 0;
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.selected = 0;
    }

    pub fn move_selection(&mut self, delta: isize, item_count: usize) {
        if item_count == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(item_count as isize) as usize;
    }

    pub fn take_input(&mut self) -> String {
        self.selected = 0;
        std::mem::take(&mut self.query)
    }

    pub fn push_console_result(&mut self, input: &str, result: Result<&str, &str>) {
        self.console_output.push(format!("> {input}"));
        self.console_output.push(match result {
            Ok(value) => format!("=> {value}"),
            Err(error) => format!("! {error}"),
        });
        if self.console_output.len() > 12 {
            let remove = self.console_output.len() - 12;
            self.console_output.drain(..remove);
        }
    }
}

pub fn filter_items(items: &[PaletteItem], query: &str) -> Vec<PaletteItem> {
    let query = query.to_lowercase();
    let mut matches = items
        .iter()
        .filter_map(|item| {
            fuzzy_score(&item.label.to_lowercase(), &query).map(|score| (score, item.clone()))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|(a_score, a), (b_score, b)| {
        b_score.cmp(a_score).then_with(|| a.label.cmp(&b.label))
    });
    matches.into_iter().map(|(_, item)| item).collect()
}

fn fuzzy_score(candidate: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let mut score = 0;
    let mut search_from = 0;
    let mut previous = None;
    for needle in query.chars() {
        let offset = candidate[search_from..].find(needle)?;
        let index = search_from + offset;
        score += if previous == Some(index.saturating_sub(1)) {
            8
        } else {
            2
        };
        if index == 0 || candidate.as_bytes().get(index.wrapping_sub(1)) == Some(&b' ') {
            score += 4;
        }
        previous = Some(index);
        search_from = index + needle.len_utf8();
    }
    Some(score - candidate.len() as i32 / 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_filter_prefers_consecutive_and_word_prefix_matches() {
        let items = vec![
            PaletteItem {
                label: "Close Pane".into(),
                action: PaletteAction::ClosePane,
            },
            PaletteItem {
                label: "Command Palette".into(),
                action: PaletteAction::RubyConsole,
            },
        ];
        let filtered = filter_items(&items, "cp");
        assert_eq!(filtered[0].label, "Close Pane");
    }

    #[test]
    fn selection_wraps_and_query_edits_reset_it() {
        let mut palette = CommandPalette::default();
        palette.open();
        palette.move_selection(-1, 3);
        assert_eq!(palette.selected(), 2);
        palette.insert("x");
        assert_eq!(palette.selected(), 0);
        palette.backspace();
        assert_eq!(palette.query(), "");
    }
}
