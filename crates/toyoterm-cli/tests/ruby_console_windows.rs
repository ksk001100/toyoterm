#![cfg(windows)]

use std::io::Read;
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
    let mut command = PtyCommand::new("powershell.exe");
    command.args(["-NoLogo", "-NoProfile"]);
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

    let executable = env!("CARGO_BIN_EXE_toyoterm").replace('\'', "''");
    let input = format!("& '{executable}' ruby console\r\n");
    session
        .write(input.as_bytes())
        .expect("start Ruby console in ConPTY");

    let mut output = String::new();
    receive_until(&output_receiver, &mut output, |text| {
        text.contains("toyoterm> ")
    });
    session
        .write(b"\r\n")
        .expect("submit an empty console line");
    receive_until(&output_receiver, &mut output, |text| {
        text.matches("toyoterm> ").count() >= 2
    });
    session.write(b"exit\r\n").expect("leave Ruby console");
    receive_until(&output_receiver, &mut output, |text| {
        text.matches("PS ").count() >= 2
    });
    session
        .write(b"echo TOYOTERM_SHELL_RECOVERED\r\nexit\r\n")
        .expect("exercise the recovered shell");
    receive_until(&output_receiver, &mut output, |text| {
        text.contains("TOYOTERM_SHELL_RECOVERED")
    });
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

fn receive_until(
    receiver: &std::sync::mpsc::Receiver<std::io::Result<Option<Vec<u8>>>>,
    output: &mut String,
    condition: impl Fn(&str) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !condition(output) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(remaining) {
            Ok(Ok(Some(chunk))) => output.push_str(&String::from_utf8_lossy(&chunk)),
            Ok(Ok(None)) if condition(output) => return,
            Ok(Ok(None)) => panic!("ConPTY reached EOF before the expected output:\n{output}"),
            Ok(Err(error)) => panic!("read ConPTY output: {error}\n{output}"),
            Err(error) => panic!("timed out waiting for ConPTY output: {error}\n{output}"),
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
