//! Native core for toyoterm.
//!
//! Hot-path terminal work stays in Rust. Scripting and future IPC layers submit
//! [`Command`] values instead of reaching into mux internals directly.

pub mod api;
pub mod app;
pub mod input;
pub mod ipc;
pub mod layout;
pub mod lifecycle;
pub mod logging;
pub mod mux;
pub mod palette;
pub mod pty;
pub mod render;
pub mod script;
pub mod shell_integration;
pub mod terminal;

pub use api::{
    Command, CommandResult, Event, HandleKind, NativeAction, NativeCommand, NativeHandle, NativeId,
    PaneId, SplitDirection, TabId, WindowId, WorkspaceId,
};
pub use app::{CellMetrics, run_gui, run_gui_smoke_test, run_gui_with_config_path};
pub use input::{
    BindingKey, KeyChord, KeyModifiers, KeyPress, KeypadKey, MouseWheelDirection, TerminalKey,
    encode_key, encode_mouse_wheel, encode_paste,
};
pub use ipc::{IpcRequest, IpcResponse, IpcServer, eval_remote, request_remote, run_console};
pub use layout::{
    ConfigErrorLayout, PaneLayout, PanePlacement, PaneRect, SplitAxis, SplitBoundary, TabPlacement,
    TabStripLayout, WorkspacePlacement, WorkspaceStripLayout,
};
pub use lifecycle::install_panic_hook;
pub use logging::init_logging;
pub use mux::{Mux, MuxError, PaneNode};
pub use palette::{CommandPalette, PaletteAction, PaletteItem, filter_items};
pub use pty::{NativePty, Pty, PtyCommand, PtyError, PtyExitStatus, PtySession, PtySize};
pub use render::{
    ConfigErrorRenderData, GpuRenderer, PaletteRenderData, PaneRenderData, RenderError,
    RenderOutcome, RenderStyle, StatusBarRenderData, TabRenderData, TextLayout,
    WorkspaceRenderData,
};
pub use script::{
    ColorConfig, ConfigManager, FontConfig, LeaderConfig, MrubyRuntime, ScriptError,
    ToyotermConfig, default_config_path,
};
pub use terminal::{
    AlacrittyTerminalBackend, CellAttributes, CellColor, CursorShape, CursorState,
    DEFAULT_SCROLLBACK_LINES, SelectionKind, SelectionSpan, TerminalBackend, TerminalCell,
    TerminalEvent, TerminalMode, TerminalSnapshot,
};
