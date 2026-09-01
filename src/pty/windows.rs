use std::ffi::OsString;
use std::io::{Read, Write};

use conpty_oxide::blocking::{
    Child as ConPtyChild, Command as ConPtyCommand, OwnedReadHalf, OwnedWriteHalf,
};
use conpty_oxide::{PtyController, SessionOptions, Size as ConPtySize};

use super::{NativePty, Program, Pty, PtyCommand, PtyError, PtyExitStatus, PtySession, PtySize};

impl Pty for NativePty {
    fn spawn(&self, command: PtyCommand, size: PtySize) -> Result<Box<dyn PtySession>, PtyError> {
        tracing::debug!(
            target: "toyoterm::pty",
            columns = size.columns,
            rows = size.rows,
            "spawn ConPTY"
        );
        let conpty_size = ConPtySize::try_new(size.columns, size.rows)
            .map_err(|error| PtyError::new("open PTY", error))?;
        let session = command_builder(command)
            .spawn_with(SessionOptions::new().size(conpty_size))
            .map_err(|error| PtyError::new("spawn PTY process", error))?;
        let process_id = session.id();
        let parts = session.into_parts();
        tracing::info!(target: "toyoterm::pty", process_id, "ConPTY process started");
        Ok(Box::new(WindowsPtySession {
            output: Some(parts.output),
            input: Some(parts.input),
            child: parts.child,
            controller: parts.controller,
            completed: false,
        }))
    }
}

fn command_builder(command: PtyCommand) -> ConPtyCommand {
    let program = match command.program {
        Program::DefaultShell => std::env::var_os("ComSpec")
            .filter(|program| !program.is_empty())
            .unwrap_or_else(|| OsString::from("cmd.exe")),
        Program::Executable(program) => program,
    };
    let mut builder = ConPtyCommand::new(program);
    builder.args(command.args);
    if let Some(cwd) = command.cwd {
        builder.current_dir(cwd);
    }
    for (key, value) in command.environment {
        match value {
            Some(value) => builder.env(key, value),
            None => builder.env_remove(key),
        };
    }
    builder
}

struct WindowsPtySession {
    output: Option<OwnedReadHalf>,
    input: Option<OwnedWriteHalf>,
    child: ConPtyChild,
    controller: PtyController,
    completed: bool,
}

impl PtySession for WindowsPtySession {
    fn process_id(&self) -> Option<u32> {
        Some(self.child.id())
    }

    fn take_reader(&mut self) -> Result<Box<dyn Read + Send>, PtyError> {
        self.output
            .take()
            .map(|reader| Box::new(reader) as Box<dyn Read + Send>)
            .ok_or_else(|| PtyError::new("open PTY reader", "reader already taken"))
    }

    fn write(&mut self, data: &[u8]) -> Result<(), PtyError> {
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| PtyError::new("write PTY input", "PTY is already closed"))?;
        input
            .write_all(data)
            .and_then(|()| input.flush())
            .map_err(|error| PtyError::new("write PTY input", error))
    }

    fn resize(&mut self, size: PtySize) -> Result<(), PtyError> {
        let size = ConPtySize::try_new(size.columns, size.rows)
            .map_err(|error| PtyError::new("resize PTY", error))?;
        self.controller
            .resize(size)
            .map_err(|error| PtyError::new("resize PTY", error))
    }

    fn try_wait(&mut self) -> Result<Option<PtyExitStatus>, PtyError> {
        let status = self
            .child
            .try_wait()
            .map_err(|error| PtyError::new("poll PTY process", error))?
            .map(|status| PtyExitStatus {
                code: status.code(),
                signal: None,
            });
        self.completed |= status.is_some();
        Ok(status)
    }

    fn wait(&mut self) -> Result<PtyExitStatus, PtyError> {
        let status = self
            .child
            .wait()
            .map_err(|error| PtyError::new("wait for PTY process", error))?;
        self.completed = true;
        Ok(PtyExitStatus {
            code: status.code(),
            signal: None,
        })
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        self.child
            .kill()
            .map_err(|error| PtyError::new("kill PTY process", error))
    }
}

impl Drop for WindowsPtySession {
    fn drop(&mut self) {
        if !self.completed {
            let _ = self.child.kill();
        }
    }
}
