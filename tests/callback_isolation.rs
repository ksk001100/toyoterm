use std::io::Read;

use toyoterm::{Command, ConfigManager, Mux, NativePty, Pty, PtyCommand, PtySize};

#[test]
fn ruby_callback_exception_does_not_terminate_the_pty_child() {
    let mut session = NativePty
        .spawn(waiting_command(), PtySize::new(80, 24))
        .expect("spawn waiting PTY child");
    let mut reader = session.take_reader().expect("take PTY reader");

    let mut config = ConfigManager::new().expect("initialize mruby");
    let mut mux = Mux::new();
    let pane = mux.current_pane().expect("active pane");
    config
        .reload(
            r#"
                Toyoterm.configure do |config|
                  config.bind "CTRL+E" do |context|
                    context.pane.send_text("must not reach the PTY\n")
                    raise "intentional callback failure"
                  end
                end
                "#,
        )
        .expect("load failing callback");

    let error = config
        .trigger_keybinding("CTRL+E", pane)
        .expect_err("callback should fail");
    assert!(error.message().contains("intentional callback failure"));
    assert_eq!(
        session.try_wait().expect("poll PTY child"),
        None,
        "PTY child exited because the Ruby callback failed"
    );

    mux.dispatch(Command::SendText {
        pane,
        text: "terminal-still-alive\n".into(),
    })
    .expect("queue native input after callback failure");
    session
        .write(
            &mux.take_pending_input(pane)
                .expect("take native input after callback failure"),
        )
        .expect("write to PTY after callback failure");
    let mut output = String::new();
    reader
        .read_to_string(&mut output)
        .expect("read PTY response after callback failure");
    let status = session.wait().expect("wait for PTY child");

    assert_eq!(status.code, 0, "unexpected PTY status: {status:?}");
    assert!(
        output.contains("AFTER:terminal-still-alive"),
        "unexpected PTY output: {output:?}"
    );
    assert!(!output.contains("must not reach the PTY"));
}

#[cfg(unix)]
fn waiting_command() -> PtyCommand {
    let mut command = PtyCommand::new("/bin/sh");
    command.args([
        "-c",
        "stty -echo; IFS= read -r line; printf 'AFTER:%s' \"$line\"",
    ]);
    command
}

#[cfg(windows)]
fn waiting_command() -> PtyCommand {
    let mut command = PtyCommand::new("powershell.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "$line = [Console]::ReadLine(); [Console]::Write('AFTER:' + $line)",
    ]);
    command
}
