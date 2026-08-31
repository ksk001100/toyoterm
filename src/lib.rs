//! Native core for toyoterm.
//!
//! Hot-path terminal work stays in Rust. Scripting and future IPC layers submit
//! [`Command`] values instead of reaching into mux internals directly.

pub mod api;
pub mod app;
pub mod input;
pub mod mux;
pub mod pty;
pub mod render;
pub mod terminal;

pub use api::{
    Command, CommandResult, Event, PaneId, SplitDirection, TabId, WindowId, WorkspaceId,
};
pub use app::{CellMetrics, run_gui};
pub use input::{
    KeyModifiers, KeyPress, MouseWheelDirection, TerminalKey, encode_key, encode_mouse_wheel,
};
pub use mux::{Mux, MuxError, PaneNode};
pub use pty::{NativePty, Pty, PtyCommand, PtyError, PtyExitStatus, PtySession, PtySize};
pub use render::{GpuRenderer, RenderError, RenderOutcome, TextLayout};
pub use terminal::{
    AlacrittyTerminalBackend, CursorShape, CursorState, DEFAULT_SCROLLBACK_LINES, TerminalBackend,
    TerminalMode, TerminalSnapshot,
};
