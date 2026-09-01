# Local IPC design

toyoterm exposes one versioned local endpoint per GUI process. Unix builds use a Unix domain socket; Windows builds use a byte-mode named pipe. The listener thread only parses frames and forwards typed requests to the winit event loop. Mux mutation remains on the GUI thread.

## Instance selection

Each GUI writes an instance state file containing its ID, PID, transport, endpoint, protocol version, and authentication token. The default ID is the GUI PID, and an `active` file points to the most recently started GUI. Set `TOYOTERM_INSTANCE` for a stable explicit ID; clients use that ID instead of `active`.

Runtime state lives under `$TOYOTERM_RUNTIME_DIR` when set. Otherwise Unix uses `$XDG_RUNTIME_DIR/toyoterm` or `/tmp/toyoterm-<uid>`, and Windows uses `%LOCALAPPDATA%\toyoterm\runtime`.

## Protocol

Protocol version 1 uses a 32-bit big-endian frame length, a fixed magic value, a 16-bit version, and length-prefixed UTF-8 fields. Requests carry a request type, the per-instance token, and typed arguments. Responses carry the version, success/error status, and UTF-8 text. Frames and individual strings are limited to 1 MiB. Unknown versions, request types, invalid UTF-8, trailing data, and oversized frames are rejected.

Mutating CLI requests normalize to `NativeCommand`: pane text, split, and workspace activation use `NativeCommand::Mux`, while reload uses `NativeCommand::ReloadConfig`. Ruby, the command palette, and IPC therefore share the same validation and mux dispatch path.

## Security boundary

On Unix the runtime directory is mode `0700`, while state files and socket nodes are mode `0600`. Windows stores state below the current user's local application-data directory and uses a local named pipe. A fresh 256-bit token from the operating system random source authenticates every request, and token comparison does not short-circuit.

This is local same-user control, not a remote security boundary. A process that can read the user's runtime state can control the terminal, including sending text to a shell and evaluating Ruby through the console. The server binds no TCP port and the protocol has no network transport.
