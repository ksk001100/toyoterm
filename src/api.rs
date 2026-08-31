use std::fmt;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub u64);

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

id_type!(WorkspaceId);
id_type!(WindowId);
id_type!(TabId);
id_type!(PaneId);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SplitDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    NewTab,
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
    SwitchWorkspace(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandResult {
    None,
    Workspace(WorkspaceId),
    Tab(TabId),
    Pane(PaneId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    WorkspaceChanged { workspace: WorkspaceId },
    TabCreated { tab: TabId },
    TabClosed { tab: TabId },
    PaneCreated { pane: PaneId },
    PaneClosed { pane: PaneId },
    PaneFocused { pane: PaneId },
    TextQueued { pane: PaneId, bytes: usize },
}
