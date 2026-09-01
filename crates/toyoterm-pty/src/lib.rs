use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use portable_pty::{Child, CommandBuilder, MasterPty, native_pty_system};
#[cfg(unix)]
use std::io::Write;

#[cfg(windows)]
mod windows;

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

#[cfg(unix)]
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

    #[cfg(unix)]
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

#[cfg(unix)]
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

    pub fn message(&self) -> &str {
        &self.message
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

#[cfg(unix)]
impl Pty for NativePty {
    fn spawn(&self, command: PtyCommand, size: PtySize) -> Result<Box<dyn PtySession>, PtyError> {
        tracing::debug!(
            target: "toyoterm::pty",
            columns = size.columns,
            rows = size.rows,
            "spawn PTY"
        );
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

        let process_id = child.process_id();
        tracing::info!(target: "toyoterm::pty", ?process_id, "PTY process started");
        Ok(Box::new(NativePtySession {
            master: pair.master,
            writer,
            child,
            reader_taken: false,
            completed: false,
        }))
    }
}

#[cfg(unix)]
struct NativePtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    reader_taken: bool,
    completed: bool,
}

#[cfg(unix)]
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

#[cfg(unix)]
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

    #[cfg(unix)]
    use toyoterm_terminal::{KeyModifiers, KeyPress, TerminalKey, TerminalMode, encode_key};

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
        let mut command = test_command(test_output_script());
        command.env("TOYOTERM_PTY_TEST", "1");
        let mut session = NativePty
            .spawn(command, PtySize::new(100, 30))
            .expect("spawn test command");
        assert!(session.process_id().is_some());

        let mut reader = session.take_reader().expect("take reader");
        let reader_thread = std::thread::spawn(move || {
            let mut output = String::new();
            reader
                .read_to_string(&mut output)
                .expect("read process output");
            output
        });
        let status = session.wait().expect("wait for test command");
        let output = reader_thread.join().expect("join PTY reader");

        assert!(status.code == 0, "unexpected status: {status:?}");
        assert!(output.contains("toyoterm-pty-ok"), "output was {output:?}");
    }

    #[cfg(windows)]
    #[test]
    fn default_shell_accepts_vt_input_and_exits() {
        let mut session = NativePty
            .spawn(PtyCommand::default_shell(), PtySize::new(80, 24))
            .expect("spawn default Windows shell");
        session
            .resize(PtySize::new(100, 30))
            .expect("resize ConPTY");
        let mut reader = session.take_reader().expect("take ConPTY reader");
        let (output_sender, output_receiver) = std::sync::mpsc::channel();
        let reader_thread = std::thread::spawn(move || {
            let mut output = String::new();
            let result = reader.read_to_string(&mut output).map(|_| output);
            output_sender.send(result).expect("send ConPTY output");
        });

        session
            .write(b"echo toyoterm-default-shell-ok\r\nexit\r\n")
            .expect("write VT input to default shell");
        // The GUI relies on reader EOF to close an exited pane; it does not
        // call wait first. The Windows backend must therefore finish output
        // autonomously when the root shell exits.
        let output = output_receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("ConPTY reader did not reach EOF after shell exit")
            .expect("read default shell output");
        let status = session.wait().expect("wait for default shell");
        reader_thread.join().expect("join ConPTY reader");

        assert_eq!(status.code, 0, "unexpected status: {status:?}");
        assert!(
            output.contains("toyoterm-default-shell-ok"),
            "output was {output:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn powershell_runs_inside_conpty() {
        let mut command = PtyCommand::new("powershell.exe");
        command.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Write-Output toyoterm-powershell-ok",
        ]);
        let mut session = NativePty
            .spawn(command, PtySize::new(80, 24))
            .expect("spawn PowerShell in ConPTY");
        let mut reader = session.take_reader().expect("take ConPTY reader");
        let reader_thread = std::thread::spawn(move || {
            let mut output = String::new();
            reader.read_to_string(&mut output).map(|_| output)
        });
        let status = session.wait().expect("wait for PowerShell");
        let output = reader_thread
            .join()
            .expect("join PowerShell reader")
            .expect("read PowerShell output");

        assert_eq!(status.code, 0, "unexpected status: {status:?}");
        assert!(
            output.contains("toyoterm-powershell-ok"),
            "output was {output:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn delivers_special_and_modified_keys_to_a_real_pty() {
        let presses = [
            KeyPress::new(TerminalKey::Text(" ".into()), KeyModifiers::default()),
            KeyPress::new(TerminalKey::Tab, KeyModifiers::default()),
            KeyPress::new(TerminalKey::Backspace, KeyModifiers::default()),
            KeyPress::new(TerminalKey::Enter, KeyModifiers::default()),
            KeyPress::new(TerminalKey::Function(5), KeyModifiers::default()),
            KeyPress::new(
                TerminalKey::Text("a".into()),
                KeyModifiers {
                    control: true,
                    ..KeyModifiers::default()
                },
            ),
            KeyPress::new(
                TerminalKey::Text("x".into()),
                KeyModifiers {
                    alt: true,
                    ..KeyModifiers::default()
                },
            ),
        ];
        let payload = presses
            .iter()
            .flat_map(|press| encode_key(press, TerminalMode::default()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(payload.len(), 12);

        let mut command = test_command(
            "stty raw -echo; printf READY; dd bs=1 count=12 2>/dev/null | od -An -tu1",
        );
        command.env("LC_ALL", "C");
        let mut session = NativePty
            .spawn(command, PtySize::new(80, 24))
            .expect("spawn raw PTY reader");
        let mut reader = session.take_reader().expect("take PTY reader");
        let mut ready = [0_u8; 5];
        reader
            .read_exact(&mut ready)
            .expect("wait for PTY readiness");
        assert_eq!(&ready, b"READY");

        session.write(&payload).expect("write encoded keys");
        let mut output = String::new();
        reader
            .read_to_string(&mut output)
            .expect("read captured key bytes");
        let status = session.wait().expect("wait for PTY key reader");
        assert_eq!(status.code, 0, "unexpected status: {status:?}");
        let captured = output
            .split_ascii_whitespace()
            .map(|byte| byte.parse::<u8>().expect("od emitted a byte"))
            .collect::<Vec<_>>();
        assert_eq!(captured, payload);
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
        command.args(["/D", "/C", script]);
        command
    }

    #[cfg(unix)]
    fn test_output_script() -> &'static str {
        "printf toyoterm-pty-ok"
    }

    #[cfg(windows)]
    fn test_output_script() -> &'static str {
        "echo toyoterm-pty-ok"
    }
}
