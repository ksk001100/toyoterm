use std::panic;

/// Installs a panic reporter while preserving Rust's standard panic output.
///
/// The binary catches unwinding panics at its outermost boundary. Application
/// and PTY guards are therefore dropped before a failure exit is returned.
pub fn install_panic_hook() {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        tracing::error!(
            target: "toyoterm::app",
            operation = "handle panic",
            panic = %panic_info,
            "fatal panic; shutting down terminal sessions"
        );
        default_hook(panic_info);
    }));
}
