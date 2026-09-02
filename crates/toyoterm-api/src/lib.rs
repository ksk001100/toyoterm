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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeAction {
    NewTab,
    ClosePane,
    ReloadConfig,
    CommandPalette,
    MaximizeWindow,
    ToggleMaximize,
    MinimizeWindow,
    ToggleFullscreen,
    UserCommand(String),
    Split(SplitDirection),
    ActivatePane(SplitDirection),
}

#[derive(Clone, Debug, PartialEq)]
pub enum NativeCommand {
    Mux(Command),
    ClipboardWrite(String),
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
}
