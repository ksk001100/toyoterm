use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::fmt;

use toyoterm_api::{
    Command, CommandResult, Event, NativeHandle, PaneId, SplitDirection, TabId, WindowId,
    WorkspaceId,
};

#[derive(Clone, Debug, PartialEq)]
pub enum PaneNode {
    Leaf(PaneId),
    Split {
        direction: SplitDirection,
        ratio: f32,
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

impl PaneNode {
    pub fn panes(&self) -> Vec<PaneId> {
        let mut panes = Vec::new();
        self.collect_panes(&mut panes);
        panes
    }

    fn collect_panes(&self, panes: &mut Vec<PaneId>) {
        match self {
            Self::Leaf(pane) => panes.push(*pane),
            Self::Split { first, second, .. } => {
                first.collect_panes(panes);
                second.collect_panes(panes);
            }
        }
    }

    fn split(&mut self, target: PaneId, new_pane: PaneId, direction: SplitDirection) -> bool {
        match self {
            Self::Leaf(pane) if *pane == target => {
                let target = Self::Leaf(target);
                let new = Self::Leaf(new_pane);
                let (first, second) = match direction {
                    SplitDirection::Left | SplitDirection::Up => (new, target),
                    SplitDirection::Right | SplitDirection::Down => (target, new),
                };
                *self = Self::Split {
                    direction,
                    ratio: 0.5,
                    first: Box::new(first),
                    second: Box::new(second),
                };
                true
            }
            Self::Leaf(_) => false,
            Self::Split { first, second, .. } => {
                first.split(target, new_pane, direction)
                    || second.split(target, new_pane, direction)
            }
        }
    }

    fn remove(self, target: PaneId) -> (Option<Self>, bool) {
        match self {
            Self::Leaf(pane) if pane == target => (None, true),
            leaf @ Self::Leaf(_) => (Some(leaf), false),
            Self::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let (first, removed) = first.remove(target);
                if removed {
                    return match first {
                        Some(first) => (
                            Some(Self::Split {
                                direction,
                                ratio,
                                first: Box::new(first),
                                second,
                            }),
                            true,
                        ),
                        None => (Some(*second), true),
                    };
                }

                let first = Box::new(first.expect("an unchanged child cannot disappear"));
                let (second, removed) = second.remove(target);
                match (second, removed) {
                    (Some(second), true) => (
                        Some(Self::Split {
                            direction,
                            ratio,
                            first,
                            second: Box::new(second),
                        }),
                        true,
                    ),
                    (None, true) => (Some(*first), true),
                    (Some(second), false) => (
                        Some(Self::Split {
                            direction,
                            ratio,
                            first,
                            second: Box::new(second),
                        }),
                        false,
                    ),
                    (None, false) => unreachable!("an unchanged child cannot disappear"),
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
struct Pane {
    tab: TabId,
    pending_input: Vec<u8>,
}

#[derive(Clone, Debug)]
struct Tab {
    window: WindowId,
    root: PaneNode,
    active_pane: PaneId,
}

#[derive(Clone, Debug)]
struct Window {
    workspace: WorkspaceId,
    tabs: Vec<TabId>,
    active_tab: TabId,
}

#[derive(Clone, Debug)]
struct Workspace {
    name: String,
    windows: Vec<WindowId>,
    active_window: WindowId,
}

#[derive(Debug, Eq, PartialEq)]
pub enum MuxError {
    UnknownWorkspace(WorkspaceId),
    UnknownWindow(WindowId),
    UnknownPane(PaneId),
    UnknownTab(TabId),
    CannotCloseLastWindow(WindowId),
    CannotCloseLastPane(PaneId),
    CannotCloseLastTab(TabId),
}

impl fmt::Display for MuxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownWorkspace(id) => write!(formatter, "unknown workspace {id}"),
            Self::UnknownWindow(id) => write!(formatter, "unknown window {id}"),
            Self::UnknownPane(id) => write!(formatter, "unknown pane {id}"),
            Self::UnknownTab(id) => write!(formatter, "unknown tab {id}"),
            Self::CannotCloseLastWindow(id) => {
                write!(formatter, "cannot close the last window {id}")
            }
            Self::CannotCloseLastPane(id) => write!(formatter, "cannot close the last pane {id}"),
            Self::CannotCloseLastTab(id) => write!(formatter, "cannot close the last tab {id}"),
        }
    }
}

impl Error for MuxError {}

/// In-memory mux state. All external control planes operate through `dispatch`.
pub struct Mux {
    next_id: u64,
    workspaces: HashMap<WorkspaceId, Workspace>,
    workspace_names: HashMap<String, WorkspaceId>,
    windows: HashMap<WindowId, Window>,
    tabs: HashMap<TabId, Tab>,
    panes: HashMap<PaneId, Pane>,
    current_workspace: WorkspaceId,
    events: VecDeque<Event>,
}

impl Default for Mux {
    fn default() -> Self {
        Self::new()
    }
}

impl Mux {
    pub fn new() -> Self {
        let mut mux = Self {
            next_id: 1,
            workspaces: HashMap::new(),
            workspace_names: HashMap::new(),
            windows: HashMap::new(),
            tabs: HashMap::new(),
            panes: HashMap::new(),
            current_workspace: WorkspaceId(0),
            events: VecDeque::new(),
        };
        let workspace = mux.create_workspace("Workspace 1".to_owned());
        mux.current_workspace = workspace;
        mux.events.clear();
        mux
    }

    pub fn current_workspace(&self) -> WorkspaceId {
        self.current_workspace
    }

    pub fn workspaces(&self) -> Vec<WorkspaceId> {
        let mut workspaces = self.workspaces.keys().copied().collect::<Vec<_>>();
        workspaces.sort_unstable();
        workspaces
    }

    pub fn workspace_name(&self, workspace: WorkspaceId) -> Option<&str> {
        self.workspaces
            .get(&workspace)
            .map(|workspace| workspace.name.as_str())
    }

    pub fn workspace_windows(&self, workspace: WorkspaceId) -> Option<&[WindowId]> {
        self.workspaces
            .get(&workspace)
            .map(|workspace| workspace.windows.as_slice())
    }

    pub fn current_window(&self) -> Option<WindowId> {
        self.workspaces
            .get(&self.current_workspace)
            .map(|workspace| workspace.active_window)
    }

    pub fn current_tab(&self) -> Option<TabId> {
        self.current_window()
            .and_then(|window| self.windows.get(&window))
            .map(|window| window.active_tab)
    }

    pub fn current_pane(&self) -> Option<PaneId> {
        self.current_tab()
            .and_then(|tab| self.tabs.get(&tab))
            .map(|tab| tab.active_pane)
    }

    pub fn pane_tree(&self, tab: TabId) -> Option<&PaneNode> {
        self.tabs.get(&tab).map(|tab| &tab.root)
    }

    pub fn tabs(&self, window: WindowId) -> Option<&[TabId]> {
        self.windows
            .get(&window)
            .map(|window| window.tabs.as_slice())
    }

    /// Returns the tab's one-based position within its window.
    pub fn tab_number(&self, tab: TabId) -> Option<usize> {
        let window = self.tabs.get(&tab)?.window;
        self.windows[&window]
            .tabs
            .iter()
            .position(|candidate| *candidate == tab)
            .map(|index| index + 1)
    }

    pub fn tab_panes(&self, tab: TabId) -> Option<Vec<PaneId>> {
        self.tabs.get(&tab).map(|tab| tab.root.panes())
    }

    pub fn pane_ids(&self) -> impl Iterator<Item = PaneId> + '_ {
        self.panes.keys().copied()
    }

    pub fn native_handles(&self) -> Vec<NativeHandle> {
        self.workspaces
            .keys()
            .copied()
            .map(NativeHandle::from)
            .chain(self.windows.keys().copied().map(NativeHandle::from))
            .chain(self.tabs.keys().copied().map(NativeHandle::from))
            .chain(self.panes.keys().copied().map(NativeHandle::from))
            .collect()
    }

    pub fn pending_input(&self, pane: PaneId) -> Option<&[u8]> {
        self.panes
            .get(&pane)
            .map(|pane| pane.pending_input.as_slice())
    }

    /// Removes input queued for a pane so an application/PTY worker can write it.
    pub fn take_pending_input(&mut self, pane: PaneId) -> Result<Vec<u8>, MuxError> {
        let pane = self
            .panes
            .get_mut(&pane)
            .ok_or(MuxError::UnknownPane(pane))?;
        Ok(std::mem::take(&mut pane.pending_input))
    }

    pub fn drain_events(&mut self) -> impl Iterator<Item = Event> + '_ {
        self.events.drain(..)
    }

    pub fn dispatch(&mut self, command: Command) -> Result<CommandResult, MuxError> {
        tracing::debug!(target: "toyoterm::mux", ?command, "dispatch command");
        match command {
            Command::NewTab => {
                let window = self.current_window().expect("mux has an active window");
                self.new_tab(window).map(CommandResult::Tab)
            }
            Command::NewTabIn(window) => self.new_tab(window).map(CommandResult::Tab),
            Command::CreateWindow(workspace) => {
                self.create_window(workspace).map(CommandResult::Window)
            }
            Command::ActivateWindow(window) => {
                self.activate_window(window)?;
                Ok(CommandResult::Window(window))
            }
            Command::CloseWindow(window) => {
                self.close_window(window)?;
                Ok(CommandResult::None)
            }
            Command::ActivateTab(tab) => {
                self.activate_tab(tab)?;
                Ok(CommandResult::Tab(tab))
            }
            Command::CloseTab(tab) => {
                self.close_tab(tab)?;
                Ok(CommandResult::None)
            }
            Command::Split { pane, direction } => {
                self.split_pane(pane, direction).map(CommandResult::Pane)
            }
            Command::ClosePane(pane) => {
                self.close_pane(pane)?;
                Ok(CommandResult::None)
            }
            Command::ActivatePane(pane) => {
                self.activate_pane(pane)?;
                Ok(CommandResult::Pane(pane))
            }
            Command::SendText { pane, text } => {
                let state = self
                    .panes
                    .get_mut(&pane)
                    .ok_or(MuxError::UnknownPane(pane))?;
                state.pending_input.extend_from_slice(text.as_bytes());
                self.events.push_back(Event::TextQueued {
                    pane,
                    bytes: text.len(),
                });
                Ok(CommandResult::None)
            }
            Command::ActivateWorkspace(workspace) => {
                self.activate_workspace(workspace)?;
                Ok(CommandResult::Workspace(workspace))
            }
            Command::SwitchWorkspace(name) => {
                let workspace = match self.workspace_names.get(&name) {
                    Some(workspace) => *workspace,
                    None => self.create_workspace(name),
                };
                self.current_workspace = workspace;
                self.events.push_back(Event::WorkspaceChanged { workspace });
                let pane = self.current_pane().expect("workspace has an active pane");
                self.events.push_back(Event::PaneFocused { pane });
                Ok(CommandResult::Workspace(workspace))
            }
        }
    }

    /// Removes a pane whose child process has exited, collapsing empty mux
    /// containers on the way up. Returns `true` when this is the final pane,
    /// allowing the GUI to terminate instead of leaving an unusable mux.
    pub fn close_exited_pane(&mut self, pane: PaneId) -> Result<bool, MuxError> {
        let tab = self
            .panes
            .get(&pane)
            .ok_or(MuxError::UnknownPane(pane))?
            .tab;
        if !matches!(self.tabs[&tab].root, PaneNode::Leaf(_)) {
            self.close_pane(pane)?;
            return Ok(false);
        }

        let window = self.tabs[&tab].window;
        if self.windows[&window].tabs.len() > 1 {
            self.close_tab(tab)?;
            return Ok(false);
        }

        if self.panes.len() == 1 {
            return Ok(true);
        }

        let was_current = self.current_pane() == Some(pane);
        let workspace = self.windows[&window].workspace;
        self.panes.remove(&pane);
        self.tabs.remove(&tab);
        self.windows.remove(&window);
        self.events.push_back(Event::PaneClosed { pane });
        self.events.push_back(Event::TabClosed { tab });
        self.events.push_back(Event::WindowClosed { window });

        let remove_workspace = self.workspaces[&workspace].windows.len() == 1;
        if remove_workspace {
            let removed = self
                .workspaces
                .remove(&workspace)
                .expect("pane workspace exists");
            self.workspace_names.remove(&removed.name);
            if self.current_workspace == workspace {
                self.current_workspace = *self
                    .workspaces
                    .keys()
                    .min()
                    .expect("a non-final pane retains a workspace");
                self.events.push_back(Event::WorkspaceChanged {
                    workspace: self.current_workspace,
                });
            }
        } else {
            let state = self
                .workspaces
                .get_mut(&workspace)
                .expect("pane workspace exists");
            state.windows.retain(|candidate| *candidate != window);
            if state.active_window == window {
                state.active_window = state.windows[0];
            }
        }

        if was_current {
            let pane = self
                .current_pane()
                .expect("another workspace retains a pane");
            self.events.push_back(Event::PaneFocused { pane });
        }

        Ok(false)
    }

    pub fn summary(&self) -> String {
        let workspace = &self.workspaces[&self.current_workspace];
        let window_count = workspace.windows.len();
        let tab_count: usize = workspace
            .windows
            .iter()
            .map(|id| self.windows[id].tabs.len())
            .sum();
        let pane_count: usize = workspace
            .windows
            .iter()
            .flat_map(|id| self.windows[id].tabs.iter())
            .map(|id| self.tabs[id].root.panes().len())
            .sum();
        format!(
            "workspace={} windows={} tabs={} panes={}",
            workspace.name, window_count, tab_count, pane_count
        )
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn create_workspace(&mut self, name: String) -> WorkspaceId {
        let workspace = WorkspaceId(self.allocate_id());
        let window = WindowId(self.allocate_id());
        let tab = TabId(self.allocate_id());
        let pane = PaneId(self.allocate_id());

        self.panes.insert(
            pane,
            Pane {
                tab,
                pending_input: Vec::new(),
            },
        );
        self.tabs.insert(
            tab,
            Tab {
                window,
                root: PaneNode::Leaf(pane),
                active_pane: pane,
            },
        );
        self.windows.insert(
            window,
            Window {
                workspace,
                tabs: vec![tab],
                active_tab: tab,
            },
        );
        self.workspaces.insert(
            workspace,
            Workspace {
                name: name.clone(),
                windows: vec![window],
                active_window: window,
            },
        );
        self.workspace_names.insert(name, workspace);
        workspace
    }

    fn create_window(&mut self, workspace: WorkspaceId) -> Result<WindowId, MuxError> {
        if !self.workspaces.contains_key(&workspace) {
            return Err(MuxError::UnknownWorkspace(workspace));
        }
        let window = WindowId(self.allocate_id());
        let tab = TabId(self.allocate_id());
        let pane = PaneId(self.allocate_id());
        self.panes.insert(
            pane,
            Pane {
                tab,
                pending_input: Vec::new(),
            },
        );
        self.tabs.insert(
            tab,
            Tab {
                window,
                root: PaneNode::Leaf(pane),
                active_pane: pane,
            },
        );
        self.windows.insert(
            window,
            Window {
                workspace,
                tabs: vec![tab],
                active_tab: tab,
            },
        );
        let state = self
            .workspaces
            .get_mut(&workspace)
            .expect("validated workspace exists");
        state.windows.push(window);
        state.active_window = window;
        self.current_workspace = workspace;
        self.events.push_back(Event::WindowCreated { window });
        self.events.push_back(Event::TabCreated { tab });
        self.events.push_back(Event::PaneCreated { pane });
        self.events.push_back(Event::PaneFocused { pane });
        Ok(window)
    }

    fn activate_window(&mut self, window: WindowId) -> Result<(), MuxError> {
        let workspace = self
            .windows
            .get(&window)
            .ok_or(MuxError::UnknownWindow(window))?
            .workspace;
        self.workspaces
            .get_mut(&workspace)
            .expect("window workspace exists")
            .active_window = window;
        self.current_workspace = workspace;
        self.events.push_back(Event::WorkspaceChanged { workspace });
        let pane = self.current_pane().expect("window has an active pane");
        self.events.push_back(Event::PaneFocused { pane });
        Ok(())
    }

    fn close_window(&mut self, window: WindowId) -> Result<(), MuxError> {
        let was_current = self.current_window() == Some(window);
        let state = self
            .windows
            .get(&window)
            .ok_or(MuxError::UnknownWindow(window))?
            .clone();
        if self.workspaces[&state.workspace].windows.len() == 1 {
            return Err(MuxError::CannotCloseLastWindow(window));
        }
        for tab in &state.tabs {
            let panes = self.tabs[tab].root.panes();
            for pane in panes {
                self.panes.remove(&pane);
                self.events.push_back(Event::PaneClosed { pane });
            }
            self.tabs.remove(tab);
            self.events.push_back(Event::TabClosed { tab: *tab });
        }
        self.windows.remove(&window);
        let workspace = self
            .workspaces
            .get_mut(&state.workspace)
            .expect("window workspace exists");
        workspace.windows.retain(|candidate| *candidate != window);
        if workspace.active_window == window {
            workspace.active_window = workspace.windows[0];
        }
        self.events.push_back(Event::WindowClosed { window });
        if was_current {
            let pane = self
                .current_pane()
                .expect("workspace retains an active pane");
            self.events.push_back(Event::PaneFocused { pane });
        }
        Ok(())
    }

    fn new_tab(&mut self, window: WindowId) -> Result<TabId, MuxError> {
        if !self.windows.contains_key(&window) {
            return Err(MuxError::UnknownWindow(window));
        }
        let is_current_window = self.current_window() == Some(window);
        let tab = TabId(self.allocate_id());
        let pane = PaneId(self.allocate_id());
        self.panes.insert(
            pane,
            Pane {
                tab,
                pending_input: Vec::new(),
            },
        );
        self.tabs.insert(
            tab,
            Tab {
                window,
                root: PaneNode::Leaf(pane),
                active_pane: pane,
            },
        );
        let state = self.windows.get_mut(&window).expect("active window exists");
        state.tabs.push(tab);
        state.active_tab = tab;
        self.events.push_back(Event::TabCreated { tab });
        self.events.push_back(Event::PaneCreated { pane });
        if is_current_window {
            self.events.push_back(Event::PaneFocused { pane });
        }
        Ok(tab)
    }

    fn split_pane(&mut self, pane: PaneId, direction: SplitDirection) -> Result<PaneId, MuxError> {
        let tab = self
            .panes
            .get(&pane)
            .ok_or(MuxError::UnknownPane(pane))?
            .tab;
        let new_pane = PaneId(self.allocate_id());
        let state = self
            .tabs
            .get_mut(&tab)
            .expect("pane refers to an existing tab");
        let split = state.root.split(pane, new_pane, direction);
        debug_assert!(split, "pane must occur in its tab tree");
        state.active_pane = new_pane;
        self.panes.insert(
            new_pane,
            Pane {
                tab,
                pending_input: Vec::new(),
            },
        );
        self.focus_tab(tab);
        self.events.push_back(Event::PaneCreated { pane: new_pane });
        self.events.push_back(Event::PaneFocused { pane: new_pane });
        Ok(new_pane)
    }

    fn activate_pane(&mut self, pane: PaneId) -> Result<(), MuxError> {
        let tab = self
            .panes
            .get(&pane)
            .ok_or(MuxError::UnknownPane(pane))?
            .tab;
        self.tabs
            .get_mut(&tab)
            .expect("pane tab exists")
            .active_pane = pane;
        self.focus_tab(tab);
        self.events.push_back(Event::PaneFocused { pane });
        Ok(())
    }

    fn activate_tab(&mut self, tab: TabId) -> Result<(), MuxError> {
        if !self.tabs.contains_key(&tab) {
            return Err(MuxError::UnknownTab(tab));
        }
        self.focus_tab(tab);
        let pane = self.tabs[&tab].active_pane;
        self.events.push_back(Event::PaneFocused { pane });
        Ok(())
    }

    fn activate_workspace(&mut self, workspace: WorkspaceId) -> Result<(), MuxError> {
        if !self.workspaces.contains_key(&workspace) {
            return Err(MuxError::UnknownWorkspace(workspace));
        }
        self.current_workspace = workspace;
        self.events.push_back(Event::WorkspaceChanged { workspace });
        let pane = self.current_pane().expect("workspace has an active pane");
        self.events.push_back(Event::PaneFocused { pane });
        Ok(())
    }

    fn focus_tab(&mut self, tab: TabId) {
        let window = self.tabs[&tab].window;
        self.windows
            .get_mut(&window)
            .expect("tab window exists")
            .active_tab = tab;
        let workspace = self.windows[&window].workspace;
        self.workspaces
            .get_mut(&workspace)
            .expect("window workspace exists")
            .active_window = window;
        self.current_workspace = workspace;
    }

    fn close_pane(&mut self, pane: PaneId) -> Result<(), MuxError> {
        let tab = self
            .panes
            .get(&pane)
            .ok_or(MuxError::UnknownPane(pane))?
            .tab;
        let state = self.tabs.get_mut(&tab).expect("pane tab exists");
        if matches!(state.root, PaneNode::Leaf(_)) {
            return Err(MuxError::CannotCloseLastPane(pane));
        }
        let old_root = std::mem::replace(&mut state.root, PaneNode::Leaf(pane));
        let (new_root, removed) = old_root.remove(pane);
        debug_assert!(removed);
        state.root = new_root.expect("a split retains at least one pane");
        self.panes.remove(&pane);
        if state.active_pane == pane {
            state.active_pane = state.root.panes()[0];
            self.events.push_back(Event::PaneFocused {
                pane: state.active_pane,
            });
        }
        self.events.push_back(Event::PaneClosed { pane });
        Ok(())
    }

    fn close_tab(&mut self, tab: TabId) -> Result<(), MuxError> {
        let was_current = self.current_tab() == Some(tab);
        let tab_state = self.tabs.get(&tab).ok_or(MuxError::UnknownTab(tab))?;
        let window = tab_state.window;
        if self.windows[&window].tabs.len() == 1 {
            return Err(MuxError::CannotCloseLastTab(tab));
        }
        let panes = tab_state.root.panes();
        for pane in panes {
            self.panes.remove(&pane);
            self.events.push_back(Event::PaneClosed { pane });
        }
        self.tabs.remove(&tab);
        let window = self.windows.get_mut(&window).expect("tab window exists");
        window.tabs.retain(|candidate| *candidate != tab);
        if window.active_tab == tab {
            window.active_tab = window.tabs[0];
        }
        self.events.push_back(Event::TabClosed { tab });
        if was_current {
            let pane = self.current_pane().expect("window retains an active pane");
            self.events.push_back(Event::PaneFocused { pane });
        }
        Ok(())
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

        fn index(&mut self, len: usize) -> usize {
            self.next() as usize % len
        }
    }

    #[test]
    fn starts_with_a_usable_hierarchy() {
        let mux = Mux::new();
        assert!(mux.current_window().is_some());
        assert!(mux.current_tab().is_some());
        assert!(mux.current_pane().is_some());
        assert_eq!(
            mux.summary(),
            "workspace=Workspace 1 windows=1 tabs=1 panes=1"
        );
    }

    #[test]
    fn tab_numbers_follow_their_position_within_the_window() {
        let mut mux = Mux::new();
        let first = mux.current_tab().unwrap();
        let CommandResult::Tab(second) = mux.dispatch(Command::NewTab).unwrap() else {
            panic!("new tab did not return a tab");
        };

        assert_eq!(mux.tab_number(first), Some(1));
        assert_eq!(mux.tab_number(second), Some(2));

        mux.dispatch(Command::CloseTab(first)).unwrap();
        assert_eq!(mux.tab_number(first), None);
        assert_eq!(mux.tab_number(second), Some(1));
    }

    #[test]
    fn split_order_respects_direction() {
        let mut mux = Mux::new();
        let original = mux.current_pane().unwrap();
        let CommandResult::Pane(new) = mux
            .dispatch(Command::Split {
                pane: original,
                direction: SplitDirection::Left,
            })
            .unwrap()
        else {
            panic!("split did not return a pane");
        };
        let tree = mux.pane_tree(mux.current_tab().unwrap()).unwrap();
        assert_eq!(tree.panes(), vec![new, original]);
        assert_eq!(mux.current_pane(), Some(new));
    }

    #[test]
    fn closing_a_pane_collapses_the_split() {
        let mut mux = Mux::new();
        let original = mux.current_pane().unwrap();
        let CommandResult::Pane(new) = mux
            .dispatch(Command::Split {
                pane: original,
                direction: SplitDirection::Right,
            })
            .unwrap()
        else {
            panic!("split did not return a pane");
        };
        mux.dispatch(Command::ClosePane(new)).unwrap();
        assert_eq!(
            mux.pane_tree(mux.current_tab().unwrap()).unwrap().panes(),
            vec![original]
        );
        assert_eq!(mux.current_pane(), Some(original));
    }

    #[test]
    fn generated_split_and_close_sequences_preserve_mux_invariants() {
        let mut cases = Cases(0xdec0_de01_1234_5678);

        for case in 0..128 {
            let mut mux = Mux::new();
            let tab = mux.current_tab().unwrap();
            let mut panes = vec![mux.current_pane().unwrap()];

            for _ in 0..16 {
                let target = panes[cases.index(panes.len())];
                let direction = match cases.index(4) {
                    0 => SplitDirection::Left,
                    1 => SplitDirection::Right,
                    2 => SplitDirection::Up,
                    _ => SplitDirection::Down,
                };
                let CommandResult::Pane(created) = mux
                    .dispatch(Command::Split {
                        pane: target,
                        direction,
                    })
                    .unwrap()
                else {
                    panic!("split did not return a pane");
                };
                panes.push(created);
            }

            while panes.len() > 1 {
                let removed = panes.swap_remove(cases.index(panes.len()));
                mux.dispatch(Command::ClosePane(removed)).unwrap();

                let tree_panes = mux.pane_tree(tab).unwrap().panes();
                assert_eq!(tree_panes.len(), panes.len(), "case {case}");
                assert!(
                    panes.iter().all(|pane| tree_panes.contains(pane)),
                    "case {case}"
                );
                assert!(!mux.pane_ids().any(|pane| pane == removed), "case {case}");
                assert!(panes.contains(&mux.current_pane().unwrap()), "case {case}");
            }

            assert_eq!(mux.pane_tree(tab).unwrap(), &PaneNode::Leaf(panes[0]));
        }
    }

    #[test]
    fn send_text_is_queued_without_a_script_callback() {
        let mut mux = Mux::new();
        let pane = mux.current_pane().unwrap();
        mux.dispatch(Command::SendText {
            pane,
            text: "echo hello\n".into(),
        })
        .unwrap();
        assert_eq!(mux.pending_input(pane), Some(&b"echo hello\n"[..]));
        assert_eq!(
            mux.take_pending_input(pane).unwrap(),
            b"echo hello\n".to_vec()
        );
        assert_eq!(mux.pending_input(pane), Some(&[][..]));
        assert_eq!(
            mux.drain_events().collect::<Vec<_>>(),
            vec![Event::TextQueued { pane, bytes: 11 }]
        );
    }

    #[test]
    fn switching_to_a_new_workspace_creates_a_complete_hierarchy() {
        let mut mux = Mux::new();
        let original = mux.current_workspace();
        let CommandResult::Workspace(created) = mux
            .dispatch(Command::SwitchWorkspace("backend".into()))
            .unwrap()
        else {
            panic!("switch did not return a workspace");
        };
        assert_ne!(created, original);
        assert_eq!(mux.summary(), "workspace=backend windows=1 tabs=1 panes=1");

        let result = mux
            .dispatch(Command::SwitchWorkspace("Workspace 1".into()))
            .unwrap();
        assert_eq!(result, CommandResult::Workspace(original));
    }

    #[test]
    fn refuses_to_remove_last_pane_or_tab() {
        let mut mux = Mux::new();
        let pane = mux.current_pane().unwrap();
        let tab = mux.current_tab().unwrap();
        assert_eq!(
            mux.dispatch(Command::ClosePane(pane)),
            Err(MuxError::CannotCloseLastPane(pane))
        );
        assert_eq!(
            mux.dispatch(Command::CloseTab(tab)),
            Err(MuxError::CannotCloseLastTab(tab))
        );
    }

    #[test]
    fn exited_panes_collapse_their_empty_containers() {
        let mut mux = Mux::new();
        let first_pane = mux.current_pane().unwrap();
        let first_tab = mux.current_tab().unwrap();
        mux.dispatch(Command::NewTab).unwrap();
        let second_pane = mux.current_pane().unwrap();

        assert!(!mux.close_exited_pane(second_pane).unwrap());
        assert_eq!(mux.current_tab(), Some(first_tab));
        assert_eq!(mux.current_pane(), Some(first_pane));
        assert_eq!(mux.pane_ids().collect::<Vec<_>>(), vec![first_pane]);
    }

    #[test]
    fn exited_pane_removes_an_empty_workspace() {
        let mut mux = Mux::new();
        let default_workspace = mux.current_workspace();
        let default_pane = mux.current_pane().unwrap();
        mux.dispatch(Command::SwitchWorkspace("temporary".into()))
            .unwrap();
        let temporary_pane = mux.current_pane().unwrap();

        assert!(!mux.close_exited_pane(temporary_pane).unwrap());
        assert_eq!(mux.current_workspace(), default_workspace);
        assert_eq!(mux.current_pane(), Some(default_pane));
        assert_eq!(mux.workspaces(), vec![default_workspace]);
    }

    #[test]
    fn final_exited_pane_requests_application_exit() {
        let mut mux = Mux::new();
        let pane = mux.current_pane().unwrap();

        assert!(mux.close_exited_pane(pane).unwrap());
        assert_eq!(mux.current_pane(), Some(pane));
    }

    #[test]
    fn tabs_keep_their_active_pane_when_switching() {
        let mut mux = Mux::new();
        let first_tab = mux.current_tab().unwrap();
        let first_pane = mux.current_pane().unwrap();
        let CommandResult::Pane(first_split) = mux
            .dispatch(Command::Split {
                pane: first_pane,
                direction: SplitDirection::Right,
            })
            .unwrap()
        else {
            panic!("split did not return a pane");
        };
        let CommandResult::Tab(second_tab) = mux.dispatch(Command::NewTab).unwrap() else {
            panic!("new tab did not return a tab");
        };

        mux.dispatch(Command::ActivateTab(first_tab)).unwrap();
        assert_eq!(mux.current_pane(), Some(first_split));
        mux.dispatch(Command::ActivateTab(second_tab)).unwrap();
        assert_ne!(mux.current_pane(), Some(first_split));
    }

    #[test]
    fn workspaces_restore_their_active_tab_and_pane() {
        let mut mux = Mux::new();
        let first_workspace = mux.current_workspace();
        let first_tab = mux.current_tab().unwrap();
        let first_pane = mux.current_pane().unwrap();
        let CommandResult::Pane(first_split) = mux
            .dispatch(Command::Split {
                pane: first_pane,
                direction: SplitDirection::Down,
            })
            .unwrap()
        else {
            panic!("split did not return a pane");
        };

        let CommandResult::Workspace(second_workspace) = mux
            .dispatch(Command::SwitchWorkspace("backend".into()))
            .unwrap()
        else {
            panic!("switch did not return a workspace");
        };
        let second_tab = mux.current_tab().unwrap();

        mux.dispatch(Command::ActivateWorkspace(first_workspace))
            .unwrap();
        assert_eq!(mux.current_tab(), Some(first_tab));
        assert_eq!(mux.current_pane(), Some(first_split));

        mux.dispatch(Command::ActivateWorkspace(second_workspace))
            .unwrap();
        assert_eq!(mux.current_tab(), Some(second_tab));
        assert_ne!(mux.current_pane(), Some(first_split));
    }

    #[test]
    fn generated_workspace_transitions_restore_each_active_hierarchy() {
        let mut mux = Mux::new();
        let mut cases = Cases(0xa11c_e55e_cafe_babe);
        let mut expected = vec![(
            mux.current_workspace(),
            mux.current_tab().unwrap(),
            mux.current_pane().unwrap(),
        )];

        for index in 0..24 {
            let CommandResult::Workspace(workspace) = mux
                .dispatch(Command::SwitchWorkspace(format!("generated-{index}")))
                .unwrap()
            else {
                panic!("switch did not return a workspace");
            };
            let original = mux.current_pane().unwrap();
            let direction = match cases.index(4) {
                0 => SplitDirection::Left,
                1 => SplitDirection::Right,
                2 => SplitDirection::Up,
                _ => SplitDirection::Down,
            };
            let CommandResult::Pane(active) = mux
                .dispatch(Command::Split {
                    pane: original,
                    direction,
                })
                .unwrap()
            else {
                panic!("split did not return a pane");
            };
            expected.push((workspace, mux.current_tab().unwrap(), active));
        }

        for case in 0..512 {
            let (workspace, tab, pane) = expected[cases.index(expected.len())];
            mux.dispatch(Command::ActivateWorkspace(workspace)).unwrap();
            assert_eq!(mux.current_workspace(), workspace, "case {case}");
            assert_eq!(mux.current_tab(), Some(tab), "case {case}");
            assert_eq!(mux.current_pane(), Some(pane), "case {case}");
        }
    }

    #[test]
    fn tab_commands_cover_create_activate_and_close_lifecycle() {
        let mut mux = Mux::new();
        let first = mux.current_tab().unwrap();
        let CommandResult::Tab(second) = mux.dispatch(Command::NewTab).unwrap() else {
            panic!("new tab did not return a tab");
        };
        assert_eq!(mux.current_tab(), Some(second));

        mux.dispatch(Command::ActivateTab(first)).unwrap();
        assert_eq!(mux.current_tab(), Some(first));
        mux.dispatch(Command::CloseTab(second)).unwrap();
        assert_eq!(mux.current_tab(), Some(first));

        let CommandResult::Tab(third) = mux.dispatch(Command::NewTab).unwrap() else {
            panic!("new tab did not return a tab");
        };
        mux.dispatch(Command::CloseTab(third)).unwrap();
        assert_eq!(mux.current_tab(), Some(first));
        assert_eq!(mux.tabs(mux.current_window().unwrap()), Some(&[first][..]));
    }

    #[test]
    fn lifecycle_events_preserve_command_order() {
        let mut mux = Mux::new();
        let first_pane = mux.current_pane().unwrap();
        let CommandResult::Tab(tab) = mux.dispatch(Command::NewTab).unwrap() else {
            panic!("new tab did not return a tab");
        };
        let created_pane = mux.current_pane().unwrap();
        assert_eq!(
            mux.drain_events().collect::<Vec<_>>(),
            vec![
                Event::TabCreated { tab },
                Event::PaneCreated { pane: created_pane },
                Event::PaneFocused { pane: created_pane },
            ]
        );

        mux.dispatch(Command::CloseTab(tab)).unwrap();
        assert_eq!(
            mux.drain_events().collect::<Vec<_>>(),
            vec![
                Event::PaneClosed { pane: created_pane },
                Event::TabClosed { tab },
                Event::PaneFocused { pane: first_pane },
            ]
        );
    }

    #[test]
    fn window_commands_cover_create_activate_tab_and_close_lifecycle() {
        let mut mux = Mux::new();
        let workspace = mux.current_workspace();
        let first = mux.current_window().unwrap();
        assert_eq!(
            mux.dispatch(Command::CloseWindow(first)),
            Err(MuxError::CannotCloseLastWindow(first))
        );

        let CommandResult::Window(second) = mux.dispatch(Command::CreateWindow(workspace)).unwrap()
        else {
            panic!("create window did not return a window");
        };
        assert_eq!(mux.current_window(), Some(second));
        assert_eq!(mux.workspace_windows(workspace), Some(&[first, second][..]));

        let CommandResult::Tab(second_tab) = mux.dispatch(Command::NewTabIn(second)).unwrap()
        else {
            panic!("new tab did not return a tab");
        };
        assert_eq!(mux.current_tab(), Some(second_tab));
        mux.dispatch(Command::ActivateWindow(first)).unwrap();
        assert_eq!(mux.current_window(), Some(first));

        mux.dispatch(Command::CloseWindow(second)).unwrap();
        assert_eq!(mux.workspace_windows(workspace), Some(&[first][..]));
        assert!(!mux.native_handles().contains(&NativeHandle::from(second)));
    }

    #[test]
    fn native_handle_snapshot_drops_closed_ids() {
        let mut mux = Mux::new();
        let first = mux.current_pane().unwrap();
        let CommandResult::Pane(second) = mux
            .dispatch(Command::Split {
                pane: first,
                direction: SplitDirection::Right,
            })
            .unwrap()
        else {
            panic!("split did not return a pane");
        };
        assert!(mux.native_handles().contains(&NativeHandle::from(second)));

        mux.dispatch(Command::ClosePane(second)).unwrap();
        assert!(!mux.native_handles().contains(&NativeHandle::from(second)));
        assert!(mux.native_handles().contains(&NativeHandle::from(first)));
    }
}
