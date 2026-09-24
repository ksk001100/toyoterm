use toyoterm_terminal::{
    AlacrittyTerminalBackend, KeyEventKind, KeyModifiers, KeyPress, KeypadKey, TerminalBackend,
    TerminalKey, TerminalMode, encode_ime_commit, encode_key,
};

fn key(key: TerminalKey, flags: u8) -> (KeyPress, TerminalMode) {
    let mut backend = AlacrittyTerminalBackend::new(20, 4);
    backend.advance(format!("\x1b[={flags}u").as_bytes());
    (KeyPress::new(key, KeyModifiers::default()), backend.mode())
}

fn encoded(press: &KeyPress, mode: TerminalMode) -> Option<Vec<u8>> {
    encode_key(press, mode)
}

#[test]
fn kitty_keyboard_disabled_keeps_legacy_encoding() {
    let mode = TerminalMode::default();
    let cases: &[(TerminalKey, &[u8])] = &[
        (TerminalKey::Text("a".into()), b"a"),
        (TerminalKey::Enter, b"\r"),
        (TerminalKey::Backspace, b"\x7f"),
        (TerminalKey::Tab, b"\t"),
        (TerminalKey::Escape, b"\x1b"),
        (TerminalKey::ArrowUp, b"\x1b[A"),
        (TerminalKey::Home, b"\x1b[H"),
        (TerminalKey::End, b"\x1b[F"),
        (TerminalKey::PageUp, b"\x1b[5~"),
        (TerminalKey::PageDown, b"\x1b[6~"),
        (TerminalKey::Insert, b"\x1b[2~"),
        (TerminalKey::Delete, b"\x1b[3~"),
        (TerminalKey::Function(1), b"\x1bOP"),
        (TerminalKey::Function(12), b"\x1b[24~"),
        (TerminalKey::Keypad(KeypadKey::Digit(7)), b"7"),
    ];
    for (input, expected) in cases {
        assert_eq!(
            encoded(&KeyPress::new(input.clone(), KeyModifiers::default()), mode),
            Some(expected.to_vec()),
            "{input:?}"
        );
    }
    let mut control_a = KeyPress::new(TerminalKey::Text("a".into()), KeyModifiers::default());
    control_a.modifiers.control = true;
    assert_eq!(encoded(&control_a, mode), Some(vec![1]));
    let mut alt_x = KeyPress::new(TerminalKey::Text("x".into()), KeyModifiers::default());
    alt_x.modifiers.alt = true;
    assert_eq!(encoded(&alt_x, mode), Some(b"\x1bx".to_vec()));
    let mut shift_tab = KeyPress::new(TerminalKey::Tab, KeyModifiers::default());
    shift_tab.modifiers.shift = true;
    assert_eq!(encoded(&shift_tab, mode), Some(b"\x1b[Z".to_vec()));
    let app_mode = TerminalMode {
        application_cursor: true,
        application_keypad: true,
        ..mode
    };
    assert_eq!(
        encoded(
            &KeyPress::new(TerminalKey::ArrowUp, KeyModifiers::default()),
            app_mode
        ),
        Some(b"\x1bOA".to_vec())
    );
    assert_eq!(
        encoded(
            &KeyPress::new(
                TerminalKey::Keypad(KeypadKey::Digit(7)),
                KeyModifiers::default()
            ),
            app_mode
        ),
        Some(b"\x1bOw".to_vec())
    );
    let functions: [&[u8]; 12] = [
        b"\x1bOP",
        b"\x1bOQ",
        b"\x1bOR",
        b"\x1bOS",
        b"\x1b[15~",
        b"\x1b[17~",
        b"\x1b[18~",
        b"\x1b[19~",
        b"\x1b[20~",
        b"\x1b[21~",
        b"\x1b[23~",
        b"\x1b[24~",
    ];
    for (index, expected) in functions.into_iter().enumerate() {
        assert_eq!(
            encoded(
                &KeyPress::new(
                    TerminalKey::Function(index as u8 + 1),
                    KeyModifiers::default()
                ),
                mode
            ),
            Some(expected.to_vec())
        );
    }
}

#[test]
fn kitty_disambiguates_control_keys() {
    let (mut input, mode) = key(TerminalKey::Text("i".into()), 1);
    input.modifiers.control = true;
    assert_eq!(encoded(&input, mode), Some(b"\x1b[105;5u".to_vec()));
    assert_eq!(
        encoded(&KeyPress::new(TerminalKey::Tab, input.modifiers), mode),
        Some(b"\x1b[9;5u".to_vec())
    );
    assert_eq!(
        encoded(
            &KeyPress::new(TerminalKey::Escape, KeyModifiers::default()),
            mode
        ),
        Some(b"\x1b[27u".to_vec())
    );
    assert_eq!(
        encoded(
            &KeyPress::new(TerminalKey::Enter, KeyModifiers::default()),
            mode
        ),
        Some(b"\r".to_vec())
    );
}

#[test]
fn kitty_reports_press_repeat_and_release_events() {
    let (mut input, mode) = key(TerminalKey::ArrowUp, 2);
    assert_eq!(encoded(&input, mode), Some(b"\x1b[A".to_vec()));
    input.kind = KeyEventKind::Repeat;
    assert_eq!(encoded(&input, mode), Some(b"\x1b[1;1:2A".to_vec()));
    input.kind = KeyEventKind::Release;
    assert_eq!(encoded(&input, mode), Some(b"\x1b[1;1:3A".to_vec()));
    let (mut text, mode) = key(TerminalKey::Text("a".into()), 2);
    text.kind = KeyEventKind::Release;
    assert_eq!(encoded(&text, mode), None);
}

#[test]
fn kitty_reports_alternate_all_and_associated_text() {
    let (mut input, mode) = key(TerminalKey::Text("a".into()), 31);
    input.modifiers.shift = true;
    input.shifted_key = Some('A');
    input.base_layout_key = Some('a');
    input.associated_text = Some("A界".into());
    assert_eq!(
        encoded(&input, mode),
        Some(b"\x1b[97:65:97;2;65:30028u".to_vec())
    );
    let (text, mode) = key(TerminalKey::Text("a".into()), 8);
    assert_eq!(encoded(&text, mode), Some(b"\x1b[97u".to_vec()));
    let (text, mode) = key(TerminalKey::Text("a".into()), 4);
    assert_eq!(encoded(&text, mode), Some(b"a".to_vec()));
}

#[test]
fn kitty_combined_flags_and_modifier_key_events() {
    let (mut input, mode) = key(TerminalKey::Text("a".into()), 3);
    input.modifiers.control = true;
    input.kind = KeyEventKind::Repeat;
    assert_eq!(encoded(&input, mode), Some(b"\x1b[97;5:2u".to_vec()));
    let (mut input, mode) = key(TerminalKey::Text("a".into()), 10);
    input.kind = KeyEventKind::Release;
    assert_eq!(encoded(&input, mode), Some(b"\x1b[97;1:3u".to_vec()));
    let (mut modifier, mode) = key(
        TerminalKey::Modifier(toyoterm_terminal::ModifierKey::ShiftLeft),
        8,
    );
    modifier.modifiers.shift = true;
    assert_eq!(encoded(&modifier, mode), Some(b"\x1b[57441;2u".to_vec()));
}

#[test]
fn kitty_encodes_function_and_numpad_keys() {
    let (input, mode) = key(TerminalKey::Function(3), 8);
    assert_eq!(encoded(&input, mode), Some(b"\x1b[13~".to_vec()));
    let (input, mode) = key(TerminalKey::Keypad(KeypadKey::Digit(7)), 8);
    assert_eq!(encoded(&input, mode), Some(b"\x1b[57406u".to_vec()));
}

#[test]
fn kitty_keyboard_mode_push_pop_changes_encoding() {
    let mut backend = AlacrittyTerminalBackend::new(20, 4);
    let input = KeyPress::new(TerminalKey::Text("a".into()), KeyModifiers::default());
    assert_eq!(encoded(&input, backend.mode()), Some(b"a".to_vec()));
    backend.advance(b"\x1b[>8u");
    assert_eq!(encoded(&input, backend.mode()), Some(b"\x1b[97u".to_vec()));
    backend.advance(b"\x1b[<1u");
    assert_eq!(encoded(&input, backend.mode()), Some(b"a".to_vec()));
    backend.advance(b"\x1b[=1u\x1b[=2;2u");
    assert!(backend.mode().keyboard.disambiguate_escape_codes);
    assert!(backend.mode().keyboard.report_event_types);
    backend.advance(b"\x1b[=1;3u");
    assert!(!backend.mode().keyboard.disambiguate_escape_codes);
    assert!(backend.mode().keyboard.report_event_types);
    backend.advance(b"\x1b[<1u");
    assert_eq!(backend.mode().keyboard, Default::default());
}

#[test]
fn kitty_modes_follow_screen_and_reset_state() {
    let mut backend = AlacrittyTerminalBackend::new(20, 4);
    backend.advance(b"\x1b[=8u");
    assert!(backend.mode().keyboard.report_all_keys_as_escape_codes);
    backend.advance(b"\x1b[?1049h");
    assert!(!backend.mode().keyboard.report_all_keys_as_escape_codes);
    backend.advance(b"\x1b[=2u");
    assert!(backend.mode().keyboard.report_event_types);
    backend.advance(b"\x1b[?1049l");
    assert!(backend.mode().keyboard.report_all_keys_as_escape_codes);
    assert!(!backend.mode().keyboard.report_event_types);
    backend.advance(b"\x1bc");
    assert_eq!(backend.mode().keyboard, Default::default());
}

#[test]
fn kitty_ime_commit_uses_unknown_key_and_unicode_scalars() {
    let (_, mode) = key(TerminalKey::Text("a".into()), 24);
    assert_eq!(encode_ime_commit("日本", mode), b"\x1b[0;;26085:26412u");
    assert_eq!(
        encode_ime_commit("日本", TerminalMode::default()),
        "日本".as_bytes()
    );
}
