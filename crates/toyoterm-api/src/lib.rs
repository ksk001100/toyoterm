use std::fmt;

mod image_pixels;
pub use image_pixels::ImagePixels;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HandleKind {
    Workspace,
    Window,
    Tab,
    Pane,
}

pub trait NativeId: Copy {
    const KIND: HandleKind;

    fn from_raw(raw: u64) -> Self;
    fn raw(self) -> u64;

    fn handle(self) -> NativeHandle {
        NativeHandle {
            kind: Self::KIND,
            id: self.raw(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct NativeHandle {
    kind: HandleKind,
    id: u64,
}

impl NativeHandle {
    pub const fn new(kind: HandleKind, id: u64) -> Self {
        Self { kind, id }
    }

    pub const fn kind(self) -> HandleKind {
        self.kind
    }

    pub const fn id(self) -> u64 {
        self.id
    }

    pub fn downcast<T: NativeId>(self) -> Option<T> {
        (self.kind == T::KIND).then(|| T::from_raw(self.id))
    }
}

macro_rules! id_type {
    ($name:ident, $kind:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub u64);

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl NativeId for $name {
            const KIND: HandleKind = HandleKind::$kind;

            fn from_raw(raw: u64) -> Self {
                Self(raw)
            }

            fn raw(self) -> u64 {
                self.0
            }
        }

        impl From<$name> for NativeHandle {
            fn from(id: $name) -> Self {
                id.handle()
            }
        }
    };
}

id_type!(WorkspaceId, Workspace);
id_type!(WindowId, Window);
id_type!(TabId, Tab);
id_type!(PaneId, Pane);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SplitDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionMotion {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
    WordForward,
    WordBackward,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaneSearchDirection {
    Next,
    Previous,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ScriptEventKind {
    AppStarted,
    ConfigReloaded,
    WorkspaceChanged,
    WindowCreated,
    WindowClosed,
    TabCreated,
    TabClosed,
    PaneCreated,
    PaneReady,
    PaneResized,
    PaneClosed,
    PaneFocused,
    TitleChanged,
    CwdChanged,
    PromptStarted,
    CommandLineStarted,
    CommandStarted,
    CommandFinished,
    Bell,
    WindowResized,
}

impl ScriptEventKind {
    pub const ALL: [Self; 20] = [
        Self::AppStarted,
        Self::ConfigReloaded,
        Self::WorkspaceChanged,
        Self::WindowCreated,
        Self::WindowClosed,
        Self::TabCreated,
        Self::TabClosed,
        Self::PaneCreated,
        Self::PaneReady,
        Self::PaneResized,
        Self::PaneClosed,
        Self::PaneFocused,
        Self::TitleChanged,
        Self::CwdChanged,
        Self::PromptStarted,
        Self::CommandLineStarted,
        Self::CommandStarted,
        Self::CommandFinished,
        Self::Bell,
        Self::WindowResized,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AppStarted => "app_started",
            Self::ConfigReloaded => "config_reloaded",
            Self::WorkspaceChanged => "workspace_changed",
            Self::WindowCreated => "window_created",
            Self::WindowClosed => "window_closed",
            Self::TabCreated => "tab_created",
            Self::TabClosed => "tab_closed",
            Self::PaneCreated => "pane_created",
            Self::PaneReady => "pane_ready",
            Self::PaneResized => "pane_resized",
            Self::PaneClosed => "pane_closed",
            Self::PaneFocused => "pane_focused",
            Self::TitleChanged => "title_changed",
            Self::CwdChanged => "cwd_changed",
            Self::PromptStarted => "prompt_started",
            Self::CommandLineStarted => "command_line_started",
            Self::CommandStarted => "command_started",
            Self::CommandFinished => "command_finished",
            Self::Bell => "bell",
            Self::WindowResized => "window_resized",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeAction {
    NewTab,
    ClosePane,
    CloseTab,
    NewWorkspace,
    ReloadConfig,
    Search,
    MaximizeWindow,
    ToggleMaximize,
    MinimizeWindow,
    ToggleFullscreen,
    NextTab,
    PreviousTab,
    NextWorkspace,
    PreviousWorkspace,
    NextPrompt,
    PreviousPrompt,
    NextMark,
    PreviousMark,
    SelectNextCommandOutput,
    SelectPreviousCommandOutput,
    SelectLastCommandOutput,
    CopySelection,
    PasteClipboard,
    StartVisualMode,
    ToggleVisualMode,
    StartVisualSelection,
    SelectVisualSelection,
    EndVisualSelection,
    MoveVisualSelection(SelectionMotion),
    YankSelection,
    UserCommand(String),
    Split(SplitDirection),
    ActivatePane(SplitDirection),
    ToggleZoom,
}

impl NativeAction {
    /// Canonical operation name shared with the configuration DSL.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::NewTab => "new_tab",
            Self::ClosePane => "close_pane",
            Self::CloseTab => "close_tab",
            Self::NewWorkspace => "new_workspace",
            Self::ReloadConfig => "reload_config",
            Self::Search => "search",
            Self::MaximizeWindow => "maximize_window",
            Self::ToggleMaximize => "toggle_maximize",
            Self::MinimizeWindow => "minimize_window",
            Self::ToggleFullscreen => "toggle_fullscreen",
            Self::NextTab => "next_tab",
            Self::PreviousTab => "previous_tab",
            Self::NextWorkspace => "next_workspace",
            Self::PreviousWorkspace => "previous_workspace",
            Self::NextPrompt => "next_prompt",
            Self::PreviousPrompt => "previous_prompt",
            Self::NextMark => "next_mark",
            Self::PreviousMark => "previous_mark",
            Self::SelectNextCommandOutput => "select_next_command_output",
            Self::SelectPreviousCommandOutput => "select_previous_command_output",
            Self::SelectLastCommandOutput => "select_last_command_output",
            Self::CopySelection => "copy_selection",
            Self::PasteClipboard => "paste_clipboard",
            Self::StartVisualMode => "start_visual_mode",
            Self::ToggleVisualMode => "toggle_visual_mode",
            Self::StartVisualSelection => "start_visual_selection",
            Self::SelectVisualSelection => "select_visual_selection",
            Self::EndVisualSelection => "end_visual_selection",
            Self::MoveVisualSelection(_) => "move_visual_selection",
            Self::YankSelection => "yank_selection",
            Self::UserCommand(_) => "user_command",
            Self::Split(_) => "split",
            Self::ActivatePane(_) => "activate_pane",
            Self::ToggleZoom => "toggle_zoom",
        }
    }

    pub fn argument(&self) -> Option<&str> {
        match self {
            Self::Split(direction) | Self::ActivatePane(direction) => Some(match direction {
                SplitDirection::Left => "left",
                SplitDirection::Right => "right",
                SplitDirection::Up => "up",
                SplitDirection::Down => "down",
            }),
            Self::MoveVisualSelection(motion) => Some(match motion {
                SelectionMotion::Left => "left",
                SelectionMotion::Right => "right",
                SelectionMotion::Up => "up",
                SelectionMotion::Down => "down",
                SelectionMotion::LineStart => "line_start",
                SelectionMotion::LineEnd => "line_end",
                SelectionMotion::WordForward => "word_forward",
                SelectionMotion::WordBackward => "word_backward",
            }),
            Self::UserCommand(name) => Some(name),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionContext {
    pub workspace: WorkspaceId,
    pub window: WindowId,
    pub tab: TabId,
    pub pane: PaneId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaneLaunchSpec {
    pub program: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub environment: Vec<(String, Option<String>)>,
}

#[derive(Clone, Debug, PartialEq)]
/// Transport envelope that routes an already-decoded native command to its
/// owning application domain.
///
/// Unlike [`NativeAction`], command variants carry the concrete IDs and data
/// needed for execution where the operation requires them. Application ingress
/// resolves actions before domain handlers mutate native state.
pub enum NativeCommand {
    Mux(Command),
    Action(ActionCommand),
    Pane(PaneCommand),
    Window(WindowCommand),
    Ui(UiCommand),
    Clipboard(ClipboardCommand),
    Script(ScriptCommand),
    Config(ConfigCommand),
}

#[derive(Clone, Debug, PartialEq)]
/// A user-operation request captured with the hierarchy that was active when
/// it was produced. The application validates or replaces that context at the
/// ingress boundary before dispatching domain commands.
pub enum ActionCommand {
    Invoke {
        action: NativeAction,
        context: ActionContext,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaneCommand {
    SplitWithLaunch {
        pane: PaneId,
        direction: SplitDirection,
        launch: PaneLaunchSpec,
    },
    Search {
        pane: PaneId,
        query: String,
        direction: PaneSearchDirection,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum WindowCommand {
    CreateWithLaunch {
        workspace: WorkspaceId,
        launch: PaneLaunchSpec,
    },
    NewTabWithLaunch {
        window: WindowId,
        launch: PaneLaunchSpec,
    },
    Maximize,
    ToggleMaximize,
    Minimize,
    ToggleFullscreen,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputPrompt {
    pub title: String,
    pub initial: String,
    pub completions: Vec<String>,
    pub history: Vec<String>,
    pub require_match: bool,
    pub dynamic: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum UiCommand {
    OpenPanel {
        id: u64,
        title: String,
        lines: Vec<String>,
    },
    UpdatePanel {
        id: u64,
        title: Option<String>,
        lines: Vec<String>,
    },
    OpenPrompt {
        id: u64,
        prompt: InputPrompt,
    },
    UpdatePromptCompletions {
        id: u64,
        revision: u64,
        items: Vec<String>,
    },
    ClosePrompt {
        id: u64,
    },
    OpenSelector {
        id: u64,
        title: String,
        items: Vec<String>,
    },
    SetPaneBadge {
        pane: PaneId,
        badge: Option<String>,
    },
    OpenSearch {
        pane: PaneId,
    },
    NavigatePrompt {
        pane: PaneId,
        direction: PaneSearchDirection,
    },
    NavigateMark {
        pane: PaneId,
        direction: PaneSearchDirection,
    },
    SelectCommandOutput {
        pane: PaneId,
        direction: PaneSearchDirection,
    },
    SelectLastCommandOutput {
        pane: PaneId,
    },
    StartVisualMode {
        pane: PaneId,
    },
    ToggleVisualMode {
        pane: PaneId,
    },
    StartVisualSelection {
        pane: PaneId,
    },
    SelectVisualSelection {
        pane: PaneId,
    },
    EndVisualSelection {
        pane: PaneId,
    },
    MoveVisualSelection {
        pane: PaneId,
        motion: SelectionMotion,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClipboardCommand {
    Write(String),
    CopySelection { pane: PaneId },
    Paste { pane: PaneId },
    YankSelection { pane: PaneId },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ScriptCommand {
    InvokeUserCommand { name: String, pane: PaneId },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigCommand {
    Reload,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    NewTab,
    NewTabIn(WindowId),
    CreateWindow(WorkspaceId),
    ActivateWindow(WindowId),
    CloseWindow(WindowId),
    ActivateTab(TabId),
    CloseTab(TabId),
    Split {
        pane: PaneId,
        direction: SplitDirection,
    },
    ClosePane(PaneId),
    ActivatePane(PaneId),
    SendText {
        pane: PaneId,
        text: String,
    },
    ActivateWorkspace(WorkspaceId),
    SwitchWorkspace(String),
    ToggleZoom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandResult {
    None,
    Workspace(WorkspaceId),
    Window(WindowId),
    Tab(TabId),
    Pane(PaneId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    WorkspaceChanged { workspace: WorkspaceId },
    WindowCreated { window: WindowId },
    WindowClosed { window: WindowId },
    TabCreated { tab: TabId },
    TabClosed { tab: TabId },
    PaneCreated { pane: PaneId },
    PaneClosed { pane: PaneId },
    PaneFocused { pane: PaneId },
    TextQueued { pane: PaneId, bytes: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_handles_preserve_id_type() {
        let pane = PaneId(7).handle();
        assert_eq!(pane.kind(), HandleKind::Pane);
        assert_eq!(pane.id(), 7);
        assert_eq!(pane.downcast::<PaneId>(), Some(PaneId(7)));
        assert_eq!(pane.downcast::<TabId>(), None);
        assert_ne!(pane, TabId(7).handle());
    }

    #[test]
    fn script_event_names_are_unique() {
        let names = ScriptEventKind::ALL.map(ScriptEventKind::as_str);
        let unique = names.into_iter().collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), ScriptEventKind::ALL.len());
    }
}
