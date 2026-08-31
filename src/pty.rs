use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use portable_pty::{Child, CommandBuilder, MasterPty, native_pty_system};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PtySize {
    pub columns: u16,
    pub rows: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
}

impl PtySize {
    pub const fn new(columns: u16, rows: u16) -> Self {
        Self {
            columns,
            rows,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

impl Default for PtySize {
    fn default() -> Self {
        Self::new(80, 24)
    }
}

impl From<PtySize> for portable_pty::PtySize {
    fn from(size: PtySize) -> Self {
        Self {
            rows: size.rows,
            cols: size.columns,
            pixel_width: size.pixel_width,
            pixel_height: size.pixel_height,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Program {
    DefaultShell,
    Executable(OsString),
}

/// A process description independent of the native PTY implementation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PtyCommand {
    program: Program,
    args: Vec<OsString>,
    cwd: Option<PathBuf>,
    environment: BTreeMap<OsString, Option<OsString>>,
}

impl PtyCommand {
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            program: Program::Executable(program.as_ref().to_owned()),
            args: Vec::new(),
            cwd: None,
            environment: BTreeMap::new(),
        }
    }

    pub fn default_shell() -> Self {
        Self {
            program: Program::DefaultShell,
            args: Vec::new(),
            cwd: None,
            environment: BTreeMap::new(),
        }
    }

    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        assert!(
            !matches!(self.program, Program::DefaultShell),
            "arguments cannot be added to the platform default shell"
        );
        self.args.push(arg.as_ref().to_owned());
        self
    }

    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        for arg in args {
            self.arg(arg);
        }
        self
    }

    pub fn cwd(&mut self, path: impl AsRef<Path>) -> &mut Self {
        self.cwd = Some(path.as_ref().to_owned());
        self
    }

    pub fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.environment
            .insert(key.as_ref().to_owned(), Some(value.as_ref().to_owned()));
        self
    }

    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.environment.insert(key.as_ref().to_owned(), None);
        self
    }

    fn into_builder(self) -> CommandBuilder {
        let mut builder = match self.program {
            Program::DefaultShell => CommandBuilder::new_default_prog(),
            Program::Executable(program) => {
                let mut builder = CommandBuilder::new(program);
                builder.args(self.args);
                builder
            }
        };
        if let Some(cwd) = self.cwd {
            builder.cwd(cwd);
        }
        for (key, value) in self.environment {
            match value {
                Some(value) => builder.env(key, value),
                None => builder.env_remove(key),
            }
        }
        builder
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PtyExitStatus {
    pub code: u32,
    pub signal: Option<String>,
}

impl From<portable_pty::ExitStatus> for PtyExitStatus {
    fn from(status: portable_pty::ExitStatus) -> Self {
        Self {
            code: status.exit_code(),
            signal: status.signal().map(str::to_owned),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct PtyError {
    operation: &'static str,
    message: String,
}

impl PtyError {
    fn new(operation: &'static str, error: impl fmt::Display) -> Self {
        Self {
            operation,
            message: error.to_string(),
        }
    }

    pub fn operation(&self) -> &'static str {
        self.operation
    }
}

impl fmt::Display for PtyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.operation, self.message)
    }
}

impl std::error::Error for PtyError {}

pub trait PtySession: Send {
    fn process_id(&self) -> Option<u32>;
    fn take_reader(&mut self) -> Result<Box<dyn Read + Send>, PtyError>;
    fn write(&mut self, data: &[u8]) -> Result<(), PtyError>;
    fn resize(&mut self, size: PtySize) -> Result<(), PtyError>;
    fn try_wait(&mut self) -> Result<Option<PtyExitStatus>, PtyError>;
    fn wait(&mut self) -> Result<PtyExitStatus, PtyError>;
    fn kill(&mut self) -> Result<(), PtyError>;
}

pub trait Pty: Send + Sync {
    fn spawn(&self, command: PtyCommand, size: PtySize) -> Result<Box<dyn PtySession>, PtyError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NativePty;

impl Pty for NativePty {
    fn spawn(&self, command: PtyCommand, size: PtySize) -> Result<Box<dyn PtySession>, PtyError> {
        if size.columns == 0 || size.rows == 0 {
            return Err(PtyError::new(
                "open PTY",
                "terminal dimensions must be non-zero",
            ));
        }

        let pair = native_pty_system()
            .openpty(size.into())
            .map_err(|error| PtyError::new("open PTY", error))?;
        let child = pair
            .slave
            .spawn_command(command.into_builder())
            .map_err(|error| PtyError::new("spawn PTY process", error))?;
        drop(pair.slave);
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| PtyError::new("open PTY writer", error))?;

        Ok(Box::new(NativePtySession {
            master: pair.master,
            writer,
            child,
            reader_taken: false,
            completed: false,
        }))
    }
}

struct NativePtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    reader_taken: bool,
    completed: bool,
}

impl PtySession for NativePtySession {
    fn process_id(&self) -> Option<u32> {
        self.child.process_id()
    }

    fn take_reader(&mut self) -> Result<Box<dyn Read + Send>, PtyError> {
        if self.reader_taken {
            return Err(PtyError::new("open PTY reader", "reader already taken"));
        }
        let reader = self
            .master
            .try_clone_reader()
            .map_err(|error| PtyError::new("open PTY reader", error))?;
        self.reader_taken = true;
        Ok(reader)
    }

    fn write(&mut self, data: &[u8]) -> Result<(), PtyError> {
        self.writer
            .write_all(data)
            .and_then(|()| self.writer.flush())
            .map_err(|error| PtyError::new("write PTY input", error))
    }

    fn resize(&mut self, size: PtySize) -> Result<(), PtyError> {
        if size.columns == 0 || size.rows == 0 {
            return Err(PtyError::new(
                "resize PTY",
                "terminal dimensions must be non-zero",
            ));
        }
        self.master
            .resize(size.into())
            .map_err(|error| PtyError::new("resize PTY", error))
    }

    fn try_wait(&mut self) -> Result<Option<PtyExitStatus>, PtyError> {
        let status = self
            .child
            .try_wait()
            .map_err(|error| PtyError::new("poll PTY process", error))?
            .map(PtyExitStatus::from);
        self.completed |= status.is_some();
        Ok(status)
    }

    fn wait(&mut self) -> Result<PtyExitStatus, PtyError> {
        let status = self
            .child
            .wait()
            .map(PtyExitStatus::from)
            .map_err(|error| PtyError::new("wait for PTY process", error))?;
        self.completed = true;
        Ok(status)
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        self.child
            .kill()
            .map_err(|error| PtyError::new("kill PTY process", error))
    }
}

impl Drop for NativePtySession {
    fn drop(&mut self) {
        if !self.completed {
            let _ = self.child.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_sized_terminals() {
        let result = NativePty.spawn(PtyCommand::default_shell(), PtySize::new(0, 24));
        let error = match result {
            Ok(_) => panic!("zero-width PTY was accepted"),
            Err(error) => error,
        };
        assert_eq!(error.operation(), "open PTY");
    }

    #[test]
    fn spawns_a_process_and_reads_its_output() {
        let mut command = test_command("printf toyoterm-pty-ok");
        command.env("TOYOTERM_PTY_TEST", "1");
        let mut session = NativePty
            .spawn(command, PtySize::new(100, 30))
            .expect("spawn test command");
        assert!(session.process_id().is_some());

        let mut output = String::new();
        session
            .take_reader()
            .expect("take reader")
            .read_to_string(&mut output)
            .expect("read process output");
        let status = session.wait().expect("wait for test command");

        assert!(status.code == 0, "unexpected status: {status:?}");
        assert!(output.contains("toyoterm-pty-ok"), "output was {output:?}");
    }

    #[cfg(unix)]
    fn test_command(script: &str) -> PtyCommand {
        let mut command = PtyCommand::new("/bin/sh");
        command.args(["-c", script]);
        command
    }

    #[cfg(windows)]
    fn test_command(script: &str) -> PtyCommand {
        let mut command = PtyCommand::new("cmd.exe");
        command.args(["/C", script]);
        command
    }
}
