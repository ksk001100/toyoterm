use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::Path;

use conpty_oxide::blocking::{
    Child as ConPtyChild, Command as ConPtyCommand, OwnedReadHalf, OwnedWriteHalf,
};
use conpty_oxide::{PtyController, SessionOptions, Size as ConPtySize};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

use super::{NativePty, Program, Pty, PtyCommand, PtyError, PtyExitStatus, PtySession, PtySize};

struct JobHandle(HANDLE);

// SAFETY: HANDLE is a Win32 kernel object handle and is safe to transfer across threads.
unsafe impl Send for JobHandle {}

impl JobHandle {
    fn create_kill_on_close() -> Result<Self, PtyError> {
        // SAFETY: Creating an unnamed Job Object with default security attributes.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(PtyError::new(
                "create job object",
                std::io::Error::last_os_error(),
            ));
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        // SAFETY: Job handle is valid and info matches the requested information class.
        let res = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if res == 0 {
            let error = std::io::Error::last_os_error();
            // SAFETY: Closing the job object handle on error.
            unsafe { CloseHandle(job) };
            return Err(PtyError::new("configure job object", error));
        }

        Ok(Self(job))
    }

    fn assign_process(&self, process_id: u32) -> Result<(), PtyError> {
        // SAFETY: Opening the target process with required rights to assign to the job object.
        let process = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, process_id) };
        if process.is_null() {
            return Err(PtyError::new(
                "open process for job object",
                std::io::Error::last_os_error(),
            ));
        }

        // SAFETY: Assigning the open process handle to the job object handle.
        let res = unsafe { AssignProcessToJobObject(self.0, process) };
        let assign_err = if res == 0 {
            Some(std::io::Error::last_os_error())
        } else {
            None
        };

        // SAFETY: Closing the process handle after assignment.
        unsafe { CloseHandle(process) };

        if let Some(err) = assign_err {
            return Err(PtyError::new("assign process to job object", err));
        }
        Ok(())
    }

    fn terminate(&self, exit_code: u32) -> Result<(), PtyError> {
        // SAFETY: Terminating all processes in the job object.
        let res = unsafe { TerminateJobObject(self.0, exit_code) };
        if res == 0 {
            Err(PtyError::new(
                "terminate job object",
                std::io::Error::last_os_error(),
            ))
        } else {
            Ok(())
        }
    }
}

impl Drop for JobHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: Closing the valid Job Object handle.
            unsafe { CloseHandle(self.0) };
        }
    }
}

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
        let job = match JobHandle::create_kill_on_close() {
            Ok(job) => {
                if let Err(error) = job.assign_process(process_id) {
                    tracing::warn!(target: "toyoterm::pty", %error, "failed to assign process to job object");
                }
                Some(job)
            }
            Err(error) => {
                tracing::warn!(target: "toyoterm::pty", %error, "failed to create job object");
                None
            }
        };
        let parts = session.into_parts();
        tracing::info!(target: "toyoterm::pty", process_id, "ConPTY process started");
        Ok(Box::new(WindowsPtySession {
            output: Some(parts.output),
            input: Some(parts.input),
            child: parts.child,
            controller: parts.controller,
            job,
            completed: false,
        }))
    }
}

fn command_builder(command: PtyCommand) -> ConPtyCommand {
    let program = match command.program {
        Program::DefaultShell => default_shell_program(),
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
    job: Option<JobHandle>,
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
        if let Some(job) = &self.job {
            let _ = job.terminate(1);
        }
        self.child
            .kill()
            .map_err(|error| PtyError::new("kill PTY process", error))
    }
}

impl Drop for WindowsPtySession {
    fn drop(&mut self) {
        if !self.completed {
            if let Some(job) = &self.job {
                let _ = job.terminate(1);
            }
            let _ = self.child.kill();
        }
    }
}

pub(crate) fn default_shell_program() -> OsString {
    find_executable_in_path("pwsh.exe")
        .or_else(|| find_executable_in_path("powershell.exe"))
        .or_else(|| {
            std::env::var_os("SystemRoot").and_then(|root| {
                let candidate =
                    Path::new(&root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
                if candidate.is_file() {
                    Some(candidate.into_os_string())
                } else {
                    None
                }
            })
        })
        .or_else(|| std::env::var_os("ComSpec").filter(|program| !program.is_empty()))
        .unwrap_or_else(|| OsString::from("cmd.exe"))
}

fn find_executable_in_path(executable_name: &str) -> Option<OsString> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(executable_name);
        if candidate.is_file() {
            return Some(candidate.into_os_string());
        }
    }
    None
}
