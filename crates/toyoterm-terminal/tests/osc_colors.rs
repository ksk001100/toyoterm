use toyoterm_terminal::{AlacrittyTerminalBackend, TerminalBackend, TerminalEvent};

#[test]
fn malformed_hex_colors_are_ignored_at_every_pty_split() {
    // Exercise multi-byte characters crossing each component boundary for all
    // supported hex widths, including the combining mark found by sixel fuzzing.
    for digits in 1..=4 {
        for character in ['\u{0300}', '界', '\u{10eeee}'] {
            if character.len_utf8() > digits * 3 {
                continue;
            }
            for offset in 0..=digits * 3 - character.len_utf8() {
                let mut hex = "f".repeat(digits * 3);
                hex.replace_range(
                    offset..offset + character.len_utf8(),
                    &character.to_string(),
                );
                let sequence = format!(
                    "\x1b]13;#abcdef\x07\x1b]13;#{hex}\x1b\\\x1b]13;?\x07\x1b]13;#123456\x07\x1b]13;?\x07ok"
                );
                for split in 0..=sequence.len() {
                    let mut terminal = AlacrittyTerminalBackend::new(80, 24);
                    terminal.advance(&sequence.as_bytes()[..split]);
                    terminal.advance(&sequence.as_bytes()[split..]);
                    assert_eq!(
                        terminal.drain_events(),
                        vec![
                            TerminalEvent::PtyWrite("\x1b]13;rgb:abab/cdcd/efef\x1b\\".into()),
                            TerminalEvent::PtyWrite("\x1b]13;rgb:1212/3434/5656\x1b\\".into()),
                        ],
                        "hex {hex:?}, split {split}"
                    );
                    assert_eq!(terminal.snapshot().lines[0], "ok");
                }
            }
        }
    }
}

#[test]
fn hex_colors_preserve_all_supported_component_widths() {
    for (hex, reply) in [
        ("123", "1010/2020/3030"),
        ("aBcDeF", "abab/cdcd/efef"),
        ("123456789", "1212/4545/7878"),
        ("123456789abc", "1212/5656/9a9a"),
    ] {
        let mut terminal = AlacrittyTerminalBackend::new(80, 24);
        terminal.advance(format!("\x1b]13;#{hex}\x07\x1b]13;?\x07").as_bytes());
        assert_eq!(
            terminal.drain_events(),
            vec![TerminalEvent::PtyWrite(format!(
                "\x1b]13;rgb:{reply}\x1b\\"
            ))]
        );
    }
}
