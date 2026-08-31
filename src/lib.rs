//! Native core for toyoterm.
//!
//! Hot-path terminal work stays in Rust. Scripting and future IPC layers submit
//! [`Command`] values instead of reaching into mux internals directly.

pub mod api;
pub mod mux;
pub mod terminal;

pub use api::{
    Command, CommandResult, Event, PaneId, SplitDirection, TabId, WindowId, WorkspaceId,
};
pub use mux::{Mux, MuxError, PaneNode};
pub use terminal::{CursorShape, CursorState, TerminalBackend, TerminalMode, TerminalSnapshot};
