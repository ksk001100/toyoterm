//! Native core for toyoterm.
//!
//! Hot-path terminal work stays in Rust. Scripting and future IPC layers submit
//! [`Command`] values instead of reaching into mux internals directly.

pub mod api;
pub mod app;
pub mod input;
pub mod layout;
pub mod mux;
pub mod pty;
pub mod render;
pub mod script;
pub mod terminal;

pub use api::{
    Command, CommandResult, Event, NativeAction, PaneId, SplitDirection, TabId, WindowId,
    WorkspaceId,
};
pub use app::{CellMetrics, run_gui, run_gui_with_config_path};
pub use input::{
    BindingKey, KeyChord, KeyModifiers, KeyPress, KeypadKey, MouseWheelDirection, TerminalKey,
    encode_key, encode_mouse_wheel, encode_paste,
};
pub use layout::{
    PaneLayout, PanePlacement, PaneRect, SplitAxis, SplitBoundary, TabPlacement, TabStripLayout,
    WorkspacePlacement, WorkspaceStripLayout,
};
pub use mux::{Mux, MuxError, PaneNode};
pub use pty::{NativePty, Pty, PtyCommand, PtyError, PtyExitStatus, PtySession, PtySize};
pub use render::{
    GpuRenderer, PaneRenderData, RenderError, RenderOutcome, RenderStyle, TabRenderData,
    TextLayout, WorkspaceRenderData,
};
pub use script::{
    ColorConfig, ConfigManager, FontConfig, MrubyRuntime, ScriptError, ToyotermConfig,
    default_config_path,
};
pub use terminal::{
    AlacrittyTerminalBackend, CellAttributes, CellColor, CursorShape, CursorState,
    DEFAULT_SCROLLBACK_LINES, SelectionKind, SelectionSpan, TerminalBackend, TerminalCell,
    TerminalMode, TerminalSnapshot,
};
