#![cfg(windows)]

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use toyoterm_pty::{NativePty, Pty, PtyCommand, PtySize};

#[test]
fn windows_binaries_use_their_intended_subsystems() {
    assert_eq!(
        pe_subsystem(env!("CARGO_BIN_EXE_toyoterm")),
        3,
        "the interactive CLI must use the Windows console subsystem"
    );
    assert_eq!(
        pe_subsystem(env!("CARGO_BIN_EXE_toyoterm-gui")),
        2,
        "the GUI launcher must not create a console window"
    );
}

#[test]
fn ruby_console_keeps_control_of_conpty_and_returns_it_to_the_shell() {
    check_console_session(false);
}

#[test]
fn ruby_console_handles_inherited_raw_input_and_restores_it() {
    check_console_session(true);
}

#[test]
fn ruby_console_accepts_redirected_input() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_toyoterm"))
        .args(["ruby", "console"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start Ruby console with redirected input");
    child
        .stdin
        .take()
        .expect("take redirected stdin")
        .write_all(b"\nexit\n")
        .expect("submit an empty line and exit through a pipe");
    let output = child.wait_with_output().expect("wait for Ruby console");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "toyoterm> toyoterm> "
    );
}

fn check_console_session(raw_input: bool) {
    let mut command = PtyCommand::new("powershell.exe");
    // This test reads raw ConPTY output without emulating a terminal. Disable
    // PSReadLine's terminal negotiation and history so shell startup does not
    // depend on the runner's module version or user profile.
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NoExit",
        "-Command",
        "Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; \
         public class ConsoleMode { \
         [DllImport(\"kernel32.dll\")] public static extern IntPtr GetStdHandle(int n); \
         [DllImport(\"kernel32.dll\")] public static extern bool GetConsoleMode(IntPtr h, out uint m); \
         [DllImport(\"kernel32.dll\")] public static extern bool SetConsoleMode(IntPtr h, uint m); }'; \
         Remove-Module PSReadLine -ErrorAction SilentlyContinue; \
         function prompt { Write-Host 'TOYOTERM_SHELL_READY'; 'PS> ' }",
    ]);
    let mut session = NativePty
        .spawn(command, PtySize::new(100, 30))
        .expect("spawn PowerShell in ConPTY");
    let mut reader = session.take_reader().expect("take ConPTY reader");
    let (output_sender, output_receiver) = std::sync::mpsc::channel();
    let reader_thread = std::thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    let _ = output_sender.send(Ok(None));
                    break;
                }
                Ok(length) => {
                    if output_sender
                        .send(Ok(Some(buffer[..length].to_vec())))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) => {
                    let _ = output_sender.send(Err(error));
                    break;
                }
            }
        }
    });

    // Wait for the explicit prompt marker before sending interactive input.
    let mut output = String::new();
    receive_until(&output_receiver, &mut output, "shell startup", |text| {
        text.contains("TOYOTERM_SHELL_READY")
    });

    let executable = env!("CARGO_BIN_EXE_toyoterm").replace('\'', "''");
    let set_mode = if raw_input {
        // Simulate a shell that leaves character-at-a-time VT input enabled.
        // Enter must still submit a line in the child REPL.
        "[void][ConsoleMode]::SetConsoleMode($h, 0x200); "
    } else {
        ""
    };
    let input = format!(
        "$h = [ConsoleMode]::GetStdHandle(-10); {set_mode}\
         $before = 0; [void][ConsoleMode]::GetConsoleMode($h, [ref]$before); \
         & '{executable}' ruby console; \
         $after = 0; [void][ConsoleMode]::GetConsoleMode($h, [ref]$after); \
         if ($before -eq $after) {{ Write-Output ('TOYOTERM_MODE_' + 'RESTORED') }}\r"
    );
    session
        .write(&conpty_input(&input))
        .expect("start Ruby console in ConPTY");

    receive_until(
        &output_receiver,
        &mut output,
        "Ruby console startup",
        |text| text.contains("toyoterm> "),
    );
    session
        .write(&conpty_input("\r"))
        .expect("submit an empty console line");
    receive_until(
        &output_receiver,
        &mut output,
        "Ruby console empty-line prompt",
        |text| text.matches("toyoterm> ").count() >= 2,
    );
    session
        .write(&conpty_input("exit\r"))
        .expect("leave Ruby console");
    receive_until(
        &output_receiver,
        &mut output,
        "shell prompt recovery",
        |text| text.matches("TOYOTERM_SHELL_READY").count() >= 2,
    );
    assert!(
        output.contains("TOYOTERM_MODE_RESTORED"),
        "the Ruby console did not restore the inherited input mode:\n{output}"
    );
    session
        .write(&conpty_input(
            "Write-Output ('TOYOTERM_SHELL_' + 'RECOVERED')\rexit\r",
        ))
        .expect("exercise the recovered shell");
    receive_until(
        &output_receiver,
        &mut output,
        "shell command execution",
        |text| text.contains("TOYOTERM_SHELL_RECOVERED"),
    );
    let status = session.wait().expect("wait for PowerShell");
    reader_thread.join().expect("join ConPTY reader");

    assert_eq!(status.code, 0, "unexpected status: {status:?}\n{output}");
    assert!(
        output.matches("toyoterm> ").count() >= 2,
        "an empty line exited the Ruby console:\n{output}"
    );
    assert!(
        output.contains("TOYOTERM_SHELL_RECOVERED"),
        "the parent shell did not recover after leaving the console:\n{output}"
    );
}

fn conpty_input(text: &str) -> Vec<u8> {
    // A bare CR goes through ConPTY's VkKeyScanW-based key synthesis. Headless
    // Windows runners may lack the keyboard layout needed for that conversion.
    // Send Enter as explicit win32-input-mode key-down/key-up records instead:
    // CSI Vk;Sc;Uc;Kd;Cs;Rc _ (VK_RETURN=13, scan code=28, Unicode CR=13).
    // Printable text remains ordinary UTF-8; the protocol supports mixing both.
    const ENTER: &str = "\x1b[13;28;13;1;0;1_\x1b[13;28;13;0;0;1_";
    text.replace('\r', ENTER).into_bytes()
}

fn receive_until(
    receiver: &std::sync::mpsc::Receiver<std::io::Result<Option<Vec<u8>>>>,
    output: &mut String,
    expected: &str,
    condition: impl Fn(&str) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !condition(output) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(remaining) {
            Ok(Ok(Some(chunk))) => output.push_str(&String::from_utf8_lossy(&chunk)),
            Ok(Ok(None)) if condition(output) => return,
            Ok(Ok(None)) => panic!("ConPTY reached EOF waiting for {expected}:\n{output:?}"),
            Ok(Err(error)) => {
                panic!("read ConPTY output waiting for {expected}: {error}\n{output:?}")
            }
            Err(error) => panic!("timed out waiting for {expected}: {error}\n{output:?}"),
        }
    }
}

fn pe_subsystem(path: &str) -> u16 {
    let image = std::fs::read(path).expect("read Windows executable");
    let pe_offset =
        u32::from_le_bytes(image[0x3c..0x40].try_into().expect("read PE header offset")) as usize;
    assert_eq!(&image[pe_offset..pe_offset + 4], b"PE\0\0");
    let optional_header = pe_offset + 24;
    u16::from_le_bytes(
        image[optional_header + 68..optional_header + 70]
            .try_into()
            .expect("read PE subsystem"),
    )
}
