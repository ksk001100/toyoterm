use std::fmt;

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
    PaneClosed,
    PaneFocused,
    TitleChanged,
    CwdChanged,
    PromptStarted,
    CommandLineStarted,
    CommandStarted,
    CommandFinished,
    Bell,
}

impl ScriptEventKind {
    pub const ALL: [Self; 17] = [
        Self::AppStarted,
        Self::ConfigReloaded,
        Self::WorkspaceChanged,
        Self::WindowCreated,
        Self::WindowClosed,
        Self::TabCreated,
        Self::TabClosed,
        Self::PaneCreated,
        Self::PaneClosed,
        Self::PaneFocused,
        Self::TitleChanged,
        Self::CwdChanged,
        Self::PromptStarted,
        Self::CommandLineStarted,
        Self::CommandStarted,
        Self::CommandFinished,
        Self::Bell,
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
            Self::PaneClosed => "pane_closed",
            Self::PaneFocused => "pane_focused",
            Self::TitleChanged => "title_changed",
            Self::CwdChanged => "cwd_changed",
            Self::PromptStarted => "prompt_started",
            Self::CommandLineStarted => "command_line_started",
            Self::CommandStarted => "command_started",
            Self::CommandFinished => "command_finished",
            Self::Bell => "bell",
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
pub enum NativeCommand {
    Mux(Command),
    InvokeAction {
        action: NativeAction,
        context: ActionContext,
    },
    CreateWindowWithLaunch {
        workspace: WorkspaceId,
        launch: PaneLaunchSpec,
    },
    NewTabWithLaunch {
        window: WindowId,
        launch: PaneLaunchSpec,
    },
    SplitWithLaunch {
        pane: PaneId,
        direction: SplitDirection,
        launch: PaneLaunchSpec,
    },
    ClipboardWrite(String),
    SetPaneBadge {
        pane: PaneId,
        badge: Option<String>,
    },
    SearchPane {
        pane: PaneId,
        query: String,
        direction: PaneSearchDirection,
    },
    OpenSelector {
        id: u64,
        title: String,
        items: Vec<String>,
    },
    ReloadConfig,
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
