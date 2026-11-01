use super::*;

pub(super) const PROMPT_SOURCE: &str = r#"
  @extension_smoke_scope = Toyoterm.resource_scope(name: :extension_smoke) do
    Toyoterm.read_string(title: 'Extension UI smoke: input',
      initial: '日本語👩‍💻' * 30, completions: ['日本語👩‍💻' * 30]) {}
  end
"#;
const PANEL_SOURCE: &str = r#"
  @extension_smoke_scope.close
  Toyoterm.show_panel(title: 'Extension UI smoke: panel',
    lines: ['Read-only extension output', '日本語・結合文字 é・絵文字 👩‍💻'] +
      (1..40).map { |n| "Panel row #{n}" })
"#;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Prompt,
    Panel,
    Finished,
}

pub(super) struct ExtensionUiSmoke {
    phase: Phase,
    pub(super) deadline: Instant,
}

impl ExtensionUiSmoke {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            phase: Phase::Prompt,
            deadline: now + Duration::from_secs(30),
        }
    }

    pub(super) fn finished(&self) -> bool {
        self.phase == Phase::Finished
    }

    fn presented(&mut self, selector: Option<&SelectorOverlay>) -> Option<&'static str> {
        let selector = selector?;
        match self.phase {
            Phase::Prompt
                if selector.is_prompt() && selector.title == "Extension UI smoke: input" =>
            {
                self.phase = Phase::Panel;
                Some(PANEL_SOURCE)
            }
            Phase::Panel
                if selector.is_panel() && selector.title == "Extension UI smoke: panel" =>
            {
                self.phase = Phase::Finished;
                None
            }
            _ => None,
        }
    }
}

impl ToyotermApplication {
    pub(super) fn extension_ui_presented(&mut self, event_loop: &ActiveEventLoop) {
        let Some(smoke) = self.extension_ui_smoke.as_mut() else {
            return;
        };
        let next_source = smoke.presented(self.ui.selector.as_ref());
        if smoke.finished() {
            println!("extension UI smoke: input prompt and read-only panel presented");
            event_loop.exit();
        } else if let Some(source) = next_source
            && let Err(error) = self.submit_script(ScriptInvocation::Eval(source.into()))
        {
            self.fail(event_loop, error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_requires_both_matching_overlay_kinds_in_order() {
        let mut smoke = ExtensionUiSmoke::new(Instant::now());
        let prompt = SelectorOverlay::prompt(
            1,
            toyoterm_api::InputPrompt {
                title: "Extension UI smoke: input".into(),
                ..Default::default()
            },
        );
        let panel = SelectorOverlay::panel(2, "Extension UI smoke: panel".into(), vec![]);
        assert!(smoke.presented(None).is_none());
        assert!(smoke.presented(Some(&panel)).is_none());
        assert!(!smoke.finished());
        assert_eq!(smoke.presented(Some(&prompt)), Some(PANEL_SOURCE));
        assert!(smoke.presented(Some(&prompt)).is_none());
        assert!(!smoke.finished());
        assert!(smoke.presented(Some(&panel)).is_none());
        assert!(smoke.finished());
    }
}
