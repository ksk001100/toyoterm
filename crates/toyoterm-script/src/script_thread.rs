use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RubyObjectModel {
    pub current_workspace: WorkspaceId,
    pub current_window: WindowId,
    pub current_tab: TabId,
    pub current_pane: PaneId,
    pub workspaces: Vec<RubyWorkspace>,
    pub windows: Vec<RubyWindow>,
    pub tabs: Vec<RubyTab>,
    pub panes: Vec<RubyPane>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RubyWorkspace {
    pub id: WorkspaceId,
    pub name: String,
    pub windows: Vec<WindowId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RubyWindow {
    pub id: WindowId,
    pub tabs: Vec<TabId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RubyTab {
    pub id: TabId,
    pub title: String,
    pub panes: Vec<PaneId>,
    pub zoomed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RubyPane {
    pub id: PaneId,
    pub title: String,
    pub cwd: Option<String>,
    pub pid: Option<u32>,
    pub command_running: bool,
    pub last_exit_status: Option<i32>,
    pub screen_text: String,
    pub zoomed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RubyEvent {
    pub name: &'static str,
    pub workspace: Option<WorkspaceId>,
    pub window: Option<WindowId>,
    pub tab: Option<TabId>,
    pub pane: Option<PaneId>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub exit_status: Option<i32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginMetadata {
    pub name: String,
    pub version: String,
    pub requires: String,
    pub path: PathBuf,
}

/// Immutable script registry mirrored on the main thread.  It contains no VM
/// state and is safe to use for native key resolution and palette rendering.
#[derive(Clone, Debug)]
pub struct ScriptSnapshot {
    pub config: ToyotermConfig,
    pub native_actions: HashMap<String, NativeAction>,
    pub keybindings: HashSet<String>,
    pub event_names: HashSet<String>,
    pub user_command_names: HashSet<String>,
    pub plugins: Vec<PluginMetadata>,
}

#[derive(Clone, Debug)]
pub struct ScriptContext {
    pub model: RubyObjectModel,
    pub handles: Vec<NativeHandle>,
    pub clipboard: Option<String>,
}

#[derive(Debug)]
pub enum ScriptInvocation {
    DrainStartup,
    KeyBinding { key: String, pane: PaneId },
    UserCommand { name: String, pane: PaneId },
    Event(RubyEvent),
    Eval(String),
    Reload,
    Bar { position: StatusBarPosition },
}

#[derive(Debug)]
pub struct ScriptRequest {
    pub id: u64,
    pub context: ScriptContext,
    pub invocation: ScriptInvocation,
}

#[derive(Debug)]
pub struct ScriptCompletion {
    pub id: u64,
    pub invocation: ScriptInvocation,
    pub result: Result<ScriptResult, ScriptError>,
}

#[derive(Debug)]
pub struct ScriptResult {
    pub value: Option<String>,
    pub bar: Option<Vec<BarItem>>,
    pub commands: Vec<NativeCommand>,
    pub snapshot: Option<ScriptSnapshot>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BarAlignment {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BarItem {
    pub alignment: BarAlignment,
    pub text: String,
}

#[derive(Debug)]
pub struct ScriptStartup {
    pub snapshot: ScriptSnapshot,
    pub config_error: Option<ScriptError>,
}

/// Channel endpoint for the single owner thread of the GUI's mruby VM.
///
/// `MrubyRuntime` is deliberately `!Send + !Sync`; it is constructed, used,
/// reloaded, and dropped inside the named worker.  Main/PTY/render code can
/// only exchange owned snapshots, invocations, and native commands with it.
pub struct ScriptThread {
    requests: mpsc::Sender<ScriptRequest>,
    worker: Option<JoinHandle<()>>,
}

impl ScriptThread {
    pub fn start(
        config_path: Option<PathBuf>,
        notify: impl Fn(ScriptCompletion) + Send + 'static,
    ) -> Result<(Self, ScriptStartup), ScriptError> {
        let (request_tx, request_rx) = mpsc::channel::<ScriptRequest>();
        let (startup_tx, startup_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("toyoterm-script".into())
            .spawn(move || {
                let startup = ConfigManager::load_startup_recovering(config_path.as_deref());
                let (mut manager, config_error) = match startup {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = startup_tx.send(Err(error));
                        return;
                    }
                };
                if startup_tx
                    .send(Ok(ScriptStartup {
                        snapshot: manager.snapshot(),
                        config_error,
                    }))
                    .is_err()
                {
                    return;
                }
                while let Ok(request) = request_rx.recv() {
                    let ScriptRequest {
                        id,
                        context,
                        invocation,
                    } = request;
                    let result = run_script_request(&mut manager, &context, &invocation);
                    notify(ScriptCompletion {
                        id,
                        invocation,
                        result,
                    });
                }
            })
            .map_err(|error| ScriptError::new("start script thread", error.to_string()))?;
        let startup = startup_rx.recv().map_err(|_| {
            ScriptError::new("start script thread", "worker exited during startup")
        })??;
        Ok((
            Self {
                requests: request_tx,
                worker: Some(worker),
            },
            startup,
        ))
    }

    pub fn submit(&self, request: ScriptRequest) -> Result<(), ScriptError> {
        self.requests
            .send(request)
            .map_err(|_| ScriptError::new("submit script request", "script thread has stopped"))
    }
}

impl Drop for ScriptThread {
    fn drop(&mut self) {
        // Closing the channel lets an idle worker drop the VM on its owner
        // thread. Do not join here: an unbounded Ruby callback must not hang
        // GUI shutdown. Dropping a JoinHandle detaches it until process exit.
        let (replacement, receiver) = mpsc::channel();
        drop(receiver);
        drop(std::mem::replace(&mut self.requests, replacement));
        drop(self.worker.take());
    }
}

impl RubyEvent {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            workspace: None,
            window: None,
            tab: None,
            pane: None,
            title: None,
            cwd: None,
            exit_status: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptError {
    operation: &'static str,
    message: String,
}

impl ScriptError {
    pub(super) fn new(operation: &'static str, message: impl Into<String>) -> Self {
        Self {
            operation,
            message: message.into(),
        }
    }

    pub fn operation(&self) -> &'static str {
        self.operation
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub(super) fn config_file(path: &Path, error: impl fmt::Display) -> Self {
        Self::new("load config", format!("{}: {error}", path.display()))
    }
}

impl fmt::Display for ScriptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.operation, self.message)
    }
}

impl std::error::Error for ScriptError {}
