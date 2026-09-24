use crate::TerminalMode;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct KeyModifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub super_key: bool,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum BindingKey {
    Logical(String),
    Physical(String),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct KeyChord {
    pub key: BindingKey,
    pub modifiers: KeyModifiers,
}

impl KeyChord {
    pub fn new(key: BindingKey, modifiers: KeyModifiers) -> Self {
        Self { key, modifiers }
    }

    pub fn canonical_name(&self) -> String {
        let mut parts = Vec::with_capacity(5);
        if self.modifiers.control {
            parts.push("CTRL".to_owned());
        }
        if self.modifiers.shift {
            parts.push("SHIFT".to_owned());
        }
        if self.modifiers.alt {
            parts.push("ALT".to_owned());
        }
        if self.modifiers.super_key {
            parts.push("SUPER".to_owned());
        }
        parts.push(match &self.key {
            BindingKey::Logical(key) => key.to_uppercase(),
            BindingKey::Physical(key) => format!("PHYSICAL:{}", key.to_uppercase()),
        });
        parts.join("+")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalKey {
    Text(String),
    Enter,
    Backspace,
    Tab,
    Escape,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    Function(u8),
    Keypad(KeypadKey),
    Modifier(ModifierKey),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModifierKey {
    ShiftLeft,
    ShiftRight,
    ControlLeft,
    ControlRight,
    AltLeft,
    AltRight,
    SuperLeft,
    SuperRight,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum KeyEventKind {
    #[default]
    Press,
    Repeat,
    Release,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeypadKey {
    Digit(u8),
    Add,
    Subtract,
    Multiply,
    Divide,
    Decimal,
    Comma,
    Equal,
    Enter,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyPress {
    pub key: TerminalKey,
    pub modifiers: KeyModifiers,
    pub kind: KeyEventKind,
    /// Text supplied by the window system for this key event, excluding IME commits.
    pub associated_text: Option<String>,
    pub shifted_key: Option<char>,
    pub base_layout_key: Option<char>,
    pub unmodified_key: Option<char>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalMouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseEventKind {
    Press(TerminalMouseButton),
    Release(TerminalMouseButton),
    Drag(TerminalMouseButton),
    Move,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseWheelDirection {
    Up,
    Down,
}

impl KeyPress {
    pub fn new(key: TerminalKey, modifiers: KeyModifiers) -> Self {
        Self {
            key,
            modifiers,
            kind: KeyEventKind::Press,
            associated_text: None,
            shifted_key: None,
            base_layout_key: None,
            unmodified_key: None,
        }
    }
}

pub fn encode_key(press: &KeyPress, mode: TerminalMode) -> Option<Vec<u8>> {
    if press.kind == KeyEventKind::Release && !mode.keyboard.report_event_types {
        return None;
    }
    if mode.keyboard != crate::KeyboardProtocolMode::default() {
        return encode_kitty_key(press, mode);
    }
    encode_legacy_key(press, mode)
}

fn encode_legacy_key(press: &KeyPress, mode: TerminalMode) -> Option<Vec<u8>> {
    let bytes = match &press.key {
        TerminalKey::Text(text) => encode_text(text, press.modifiers.control)?,
        TerminalKey::Enter => vec![b'\r'],
        TerminalKey::Backspace => vec![0x7f],
        TerminalKey::Tab if press.modifiers.shift => b"\x1b[Z".to_vec(),
        TerminalKey::Tab => vec![b'\t'],
        TerminalKey::Escape => vec![0x1b],
        TerminalKey::ArrowUp => cursor_sequence(b'A', mode.application_cursor),
        TerminalKey::ArrowDown => cursor_sequence(b'B', mode.application_cursor),
        TerminalKey::ArrowRight => cursor_sequence(b'C', mode.application_cursor),
        TerminalKey::ArrowLeft => cursor_sequence(b'D', mode.application_cursor),
        TerminalKey::Home => b"\x1b[H".to_vec(),
        TerminalKey::End => b"\x1b[F".to_vec(),
        TerminalKey::PageUp => b"\x1b[5~".to_vec(),
        TerminalKey::PageDown => b"\x1b[6~".to_vec(),
        TerminalKey::Insert => b"\x1b[2~".to_vec(),
        TerminalKey::Delete => b"\x1b[3~".to_vec(),
        TerminalKey::Function(number) => function_sequence(*number)?.to_vec(),
        TerminalKey::Keypad(key) => keypad_sequence(*key, mode.application_keypad)?.to_vec(),
        TerminalKey::Modifier(_) => return None,
    };

    if press.modifiers.alt {
        let mut prefixed = Vec::with_capacity(bytes.len() + 1);
        prefixed.push(0x1b);
        prefixed.extend(bytes);
        Some(prefixed)
    } else {
        Some(bytes)
    }
}

// Kitty's CSI 1;modifier letter and CSI number;modifier ~ forms remain
// functional-key encodings; text, modifiers and keypad keys use CSI u.
fn encode_kitty_key(press: &KeyPress, mode: TerminalMode) -> Option<Vec<u8>> {
    let flags = mode.keyboard;
    let all = flags.report_all_keys_as_escape_codes;
    let event = press.kind;
    let modified = press.modifiers.control || press.modifiers.alt || press.modifiers.super_key;
    let escape_code = match &press.key {
        TerminalKey::Text(_) => all || (flags.disambiguate_escape_codes && modified),
        TerminalKey::Modifier(_) => all,
        TerminalKey::Escape => all || flags.disambiguate_escape_codes || flags.report_event_types,
        TerminalKey::Enter | TerminalKey::Tab | TerminalKey::Backspace => {
            all || ((flags.disambiguate_escape_codes || flags.report_event_types)
                && (press.modifiers.control || press.modifiers.alt || press.modifiers.super_key))
        }
        TerminalKey::Keypad(_) => {
            all || flags.disambiguate_escape_codes || flags.report_event_types
        }
        _ => all || flags.disambiguate_escape_codes || flags.report_event_types,
    };
    if !escape_code {
        return if event == KeyEventKind::Release {
            None
        } else {
            encode_legacy_key(press, mode)
        };
    }
    let (number, suffix) = kitty_key_code(press)?;
    let modifier = 1
        + u16::from(press.modifiers.shift)
        + (u16::from(press.modifiers.alt) << 1)
        + (u16::from(press.modifiers.control) << 2)
        + (u16::from(press.modifiers.super_key) << 3);
    let has_event = flags.report_event_types && event != KeyEventKind::Press;
    let mut sequence = if !matches!(suffix, 'u' | '~') && modifier == 1 && !has_event {
        "\x1b[".to_owned()
    } else {
        format!("\x1b[{number}")
    };
    if suffix == 'u' && flags.report_alternate_keys {
        let mut has_shifted = false;
        if press.modifiers.shift
            && let Some(shifted) = press.shifted_key
        {
            sequence.push(':');
            sequence.push_str(&(shifted as u32).to_string());
            has_shifted = true;
        }
        if let Some(base) = press.base_layout_key {
            if !has_shifted {
                sequence.push(':');
            }
            sequence.push(':');
            sequence.push_str(&(base as u32).to_string());
        }
    }
    if modifier != 1 || has_event {
        sequence.push(';');
        sequence.push_str(&modifier.to_string());
    }
    if has_event {
        sequence.push(':');
        sequence.push(if event == KeyEventKind::Repeat {
            '2'
        } else {
            '3'
        });
    }
    if suffix == 'u'
        && all
        && flags.report_associated_text
        && event != KeyEventKind::Release
        && let Some(text) = press
            .associated_text
            .as_deref()
            .filter(|text| !text.is_empty())
    {
        let codepoints: Vec<_> = text
            .chars()
            .filter(|ch| !ch.is_control())
            .map(|ch| (ch as u32).to_string())
            .collect();
        if !codepoints.is_empty() {
            sequence.push(';');
            if modifier == 1 && !has_event {
                sequence.push(';');
            }
            sequence.push_str(&codepoints.join(":"));
        }
    }
    sequence.push(suffix);
    Some(sequence.into_bytes())
}

fn kitty_key_code(press: &KeyPress) -> Option<(u32, char)> {
    Some(match &press.key {
        TerminalKey::Text(text) => {
            let mut chars = text.chars();
            let first = chars.next()?;
            (
                press.unmodified_key.map(u32::from).unwrap_or_else(|| {
                    if chars.next().is_some() {
                        0
                    } else {
                        first as u32
                    }
                }),
                'u',
            )
        }
        TerminalKey::Escape => (27, 'u'),
        TerminalKey::Enter => (13, 'u'),
        TerminalKey::Tab => (9, 'u'),
        TerminalKey::Backspace => (127, 'u'),
        TerminalKey::Insert => (2, '~'),
        TerminalKey::Delete => (3, '~'),
        TerminalKey::PageUp => (5, '~'),
        TerminalKey::PageDown => (6, '~'),
        TerminalKey::ArrowUp => (1, 'A'),
        TerminalKey::ArrowDown => (1, 'B'),
        TerminalKey::ArrowRight => (1, 'C'),
        TerminalKey::ArrowLeft => (1, 'D'),
        TerminalKey::Home => (1, 'H'),
        TerminalKey::End => (1, 'F'),
        TerminalKey::Function(1) => (1, 'P'),
        TerminalKey::Function(2) => (1, 'Q'),
        TerminalKey::Function(3) => (13, '~'),
        TerminalKey::Function(4) => (1, 'S'),
        TerminalKey::Function(5) => (15, '~'),
        TerminalKey::Function(6) => (17, '~'),
        TerminalKey::Function(7) => (18, '~'),
        TerminalKey::Function(8) => (19, '~'),
        TerminalKey::Function(9) => (20, '~'),
        TerminalKey::Function(10) => (21, '~'),
        TerminalKey::Function(11) => (23, '~'),
        TerminalKey::Function(12) => (24, '~'),
        TerminalKey::Function(_) => return None,
        TerminalKey::Keypad(key) => (
            match key {
                KeypadKey::Digit(n @ 0..=9) => 57399 + u32::from(*n),
                KeypadKey::Digit(_) => return None,
                KeypadKey::Decimal => 57409,
                KeypadKey::Divide => 57410,
                KeypadKey::Multiply => 57411,
                KeypadKey::Subtract => 57412,
                KeypadKey::Add => 57413,
                KeypadKey::Enter => 57414,
                KeypadKey::Equal => 57415,
                KeypadKey::Comma => 57416,
            },
            'u',
        ),
        TerminalKey::Modifier(key) => (
            match key {
                ModifierKey::ShiftLeft => 57441,
                ModifierKey::ControlLeft => 57442,
                ModifierKey::AltLeft => 57443,
                ModifierKey::SuperLeft => 57444,
                ModifierKey::ShiftRight => 57447,
                ModifierKey::ControlRight => 57448,
                ModifierKey::AltRight => 57449,
                ModifierKey::SuperRight => 57450,
            },
            'u',
        ),
    })
}

pub fn encode_mouse_wheel(
    direction: MouseWheelDirection,
    column: u16,
    row: u16,
    modifiers: KeyModifiers,
    sgr_mouse: bool,
) -> Vec<u8> {
    let mut code = match direction {
        MouseWheelDirection::Up => 64,
        MouseWheelDirection::Down => 65,
    };
    if modifiers.shift {
        code += 4;
    }
    if modifiers.alt {
        code += 8;
    }
    if modifiers.control {
        code += 16;
    }

    let column = column.saturating_add(1);
    let row = row.saturating_add(1);
    if sgr_mouse {
        format!("\x1b[<{code};{column};{row}M").into_bytes()
    } else {
        vec![
            0x1b,
            b'[',
            b'M',
            (code + 32) as u8,
            column.min(223) as u8 + 32,
            row.min(223) as u8 + 32,
        ]
    }
}

pub fn encode_mouse_event(
    kind: MouseEventKind,
    column: u16,
    row: u16,
    modifiers: KeyModifiers,
    sgr_mouse: bool,
) -> Vec<u8> {
    let mut modifier_bits = 0;
    if modifiers.shift {
        modifier_bits += 4;
    }
    if modifiers.alt {
        modifier_bits += 8;
    }
    if modifiers.control {
        modifier_bits += 16;
    }

    let column = column.saturating_add(1);
    let row = row.saturating_add(1);

    if sgr_mouse {
        let (code, trailer) = match kind {
            MouseEventKind::Press(button) => (mouse_button_code(button) + modifier_bits, 'M'),
            MouseEventKind::Release(button) => (mouse_button_code(button) + modifier_bits, 'm'),
            MouseEventKind::Drag(button) => (mouse_button_code(button) + 32 + modifier_bits, 'M'),
            MouseEventKind::Move => (3 + 32 + modifier_bits, 'M'),
        };
        format!("\x1b[<{code};{column};{row}{trailer}").into_bytes()
    } else {
        let code = match kind {
            MouseEventKind::Press(button) => mouse_button_code(button) + modifier_bits,
            MouseEventKind::Release(_) => 3 + modifier_bits,
            MouseEventKind::Drag(button) => mouse_button_code(button) + 32 + modifier_bits,
            MouseEventKind::Move => 3 + 32 + modifier_bits,
        };
        vec![
            0x1b,
            b'[',
            b'M',
            (code + 32) as u8,
            column.min(223) as u8 + 32,
            row.min(223) as u8 + 32,
        ]
    }
}

fn mouse_button_code(button: TerminalMouseButton) -> u32 {
    match button {
        TerminalMouseButton::Left => 0,
        TerminalMouseButton::Middle => 1,
        TerminalMouseButton::Right => 2,
    }
}

pub fn encode_paste(text: &str, mode: TerminalMode) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    if mode.bracketed_paste {
        let mut bytes = Vec::with_capacity(normalized.len() + 12);
        bytes.extend_from_slice(b"\x1b[200~");
        bytes.extend_from_slice(normalized.as_bytes());
        bytes.extend_from_slice(b"\x1b[201~");
        bytes
    } else {
        normalized.replace('\n', "\r").into_bytes()
    }
}

/// Encode text committed by an IME, for which no key identity is available.
pub fn encode_ime_commit(text: &str, mode: TerminalMode) -> Vec<u8> {
    if mode.keyboard.report_all_keys_as_escape_codes && mode.keyboard.report_associated_text {
        let codepoints: Vec<_> = text
            .chars()
            .filter(|ch| !ch.is_control())
            .map(|ch| (ch as u32).to_string())
            .collect();
        if !codepoints.is_empty() {
            return format!("\x1b[0;;{}u", codepoints.join(":")).into_bytes();
        }
        return Vec::new();
    }
    text.as_bytes().to_vec()
}

fn encode_text(text: &str, control: bool) -> Option<Vec<u8>> {
    if !control {
        return Some(text.as_bytes().to_vec());
    }

    let mut characters = text.chars();
    let character = characters.next()?;
    if characters.next().is_some() {
        return None;
    }
    let control = match character {
        'a'..='z' => character as u8 - b'a' + 1,
        'A'..='Z' => character as u8 - b'A' + 1,
        ' ' | '@' | '`' => 0,
        '[' | '{' => 27,
        '\\' | '|' => 28,
        ']' | '}' => 29,
        '^' | '~' => 30,
        '_' => 31,
        '?' => 127,
        _ => return None,
    };
    Some(vec![control])
}

fn cursor_sequence(final_byte: u8, application_cursor: bool) -> Vec<u8> {
    vec![
        0x1b,
        if application_cursor { b'O' } else { b'[' },
        final_byte,
    ]
}

fn function_sequence(number: u8) -> Option<&'static [u8]> {
    Some(match number {
        1 => b"\x1bOP",
        2 => b"\x1bOQ",
        3 => b"\x1bOR",
        4 => b"\x1bOS",
        5 => b"\x1b[15~",
        6 => b"\x1b[17~",
        7 => b"\x1b[18~",
        8 => b"\x1b[19~",
        9 => b"\x1b[20~",
        10 => b"\x1b[21~",
        11 => b"\x1b[23~",
        12 => b"\x1b[24~",
        _ => return None,
    })
}

fn keypad_sequence(key: KeypadKey, application_keypad: bool) -> Option<&'static [u8]> {
    if !application_keypad {
        return Some(match key {
            KeypadKey::Digit(0) => b"0",
            KeypadKey::Digit(1) => b"1",
            KeypadKey::Digit(2) => b"2",
            KeypadKey::Digit(3) => b"3",
            KeypadKey::Digit(4) => b"4",
            KeypadKey::Digit(5) => b"5",
            KeypadKey::Digit(6) => b"6",
            KeypadKey::Digit(7) => b"7",
            KeypadKey::Digit(8) => b"8",
            KeypadKey::Digit(9) => b"9",
            KeypadKey::Digit(_) => return None,
            KeypadKey::Add => b"+",
            KeypadKey::Subtract => b"-",
            KeypadKey::Multiply => b"*",
            KeypadKey::Divide => b"/",
            KeypadKey::Decimal => b".",
            KeypadKey::Comma => b",",
            KeypadKey::Equal => b"=",
            KeypadKey::Enter => b"\r",
        });
    }

    Some(match key {
        KeypadKey::Digit(0) => b"\x1bOp",
        KeypadKey::Digit(1) => b"\x1bOq",
        KeypadKey::Digit(2) => b"\x1bOr",
        KeypadKey::Digit(3) => b"\x1bOs",
        KeypadKey::Digit(4) => b"\x1bOt",
        KeypadKey::Digit(5) => b"\x1bOu",
        KeypadKey::Digit(6) => b"\x1bOv",
        KeypadKey::Digit(7) => b"\x1bOw",
        KeypadKey::Digit(8) => b"\x1bOx",
        KeypadKey::Digit(9) => b"\x1bOy",
        KeypadKey::Digit(_) => return None,
        KeypadKey::Add => b"\x1bOk",
        KeypadKey::Subtract => b"\x1bOm",
        KeypadKey::Multiply => b"\x1bOj",
        KeypadKey::Divide => b"\x1bOo",
        KeypadKey::Decimal => b"\x1bOn",
        KeypadKey::Comma => b"\x1bOl",
        KeypadKey::Equal => b"\x1bOX",
        KeypadKey::Enter => b"\x1bOM",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: TerminalKey) -> KeyPress {
        KeyPress::new(key, KeyModifiers::default())
    }

    #[test]
    fn encodes_text_and_control_keys() {
        assert_eq!(
            encode_key(
                &press(TerminalKey::Text("hello".into())),
                TerminalMode::default()
            ),
            Some(b"hello".to_vec())
        );
        let control_c = KeyPress::new(
            TerminalKey::Text("c".into()),
            KeyModifiers {
                control: true,
                ..KeyModifiers::default()
            },
        );
        assert_eq!(
            encode_key(&control_c, TerminalMode::default()),
            Some(vec![3])
        );
    }

    #[test]
    fn physical_and_logical_key_chords_have_distinct_names() {
        let modifiers = KeyModifiers {
            control: true,
            shift: true,
            ..KeyModifiers::default()
        };
        assert_eq!(
            KeyChord::new(BindingKey::Logical("h".into()), modifiers).canonical_name(),
            "CTRL+SHIFT+H"
        );
        assert_eq!(
            KeyChord::new(BindingKey::Physical("KeyH".into()), modifiers).canonical_name(),
            "CTRL+SHIFT+PHYSICAL:KEYH"
        );
    }

    #[test]
    fn alt_prefixes_encoded_input() {
        let alt_x = KeyPress::new(
            TerminalKey::Text("x".into()),
            KeyModifiers {
                alt: true,
                ..KeyModifiers::default()
            },
        );
        assert_eq!(
            encode_key(&alt_x, TerminalMode::default()),
            Some(b"\x1bx".to_vec())
        );
    }

    #[test]
    fn honors_application_cursor_mode() {
        assert_eq!(
            encode_key(&press(TerminalKey::ArrowUp), TerminalMode::default()),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            encode_key(
                &press(TerminalKey::ArrowUp),
                TerminalMode {
                    application_cursor: true,
                    ..TerminalMode::default()
                },
            ),
            Some(b"\x1bOA".to_vec())
        );
    }

    #[test]
    fn encodes_shift_tab_and_function_keys() {
        let shift_tab = KeyPress::new(
            TerminalKey::Tab,
            KeyModifiers {
                shift: true,
                ..KeyModifiers::default()
            },
        );
        assert_eq!(
            encode_key(&shift_tab, TerminalMode::default()),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(
            encode_key(&press(TerminalKey::Function(12)), TerminalMode::default()),
            Some(b"\x1b[24~".to_vec())
        );
        assert_eq!(
            encode_key(&press(TerminalKey::Function(13)), TerminalMode::default()),
            None
        );
    }

    #[test]
    fn encodes_numeric_keypad_in_normal_and_application_modes() {
        let key = press(TerminalKey::Keypad(KeypadKey::Digit(7)));
        assert_eq!(
            encode_key(&key, TerminalMode::default()),
            Some(b"7".to_vec())
        );
        assert_eq!(
            encode_key(
                &key,
                TerminalMode {
                    application_keypad: true,
                    ..TerminalMode::default()
                },
            ),
            Some(b"\x1bOw".to_vec())
        );

        let enter = press(TerminalKey::Keypad(KeypadKey::Enter));
        assert_eq!(
            encode_key(
                &enter,
                TerminalMode {
                    application_keypad: true,
                    ..TerminalMode::default()
                },
            ),
            Some(b"\x1bOM".to_vec())
        );
    }

    #[test]
    fn encodes_sgr_mouse_wheel_with_cell_coordinates() {
        let bytes = encode_mouse_wheel(
            MouseWheelDirection::Up,
            4,
            2,
            KeyModifiers {
                control: true,
                ..KeyModifiers::default()
            },
            true,
        );
        assert_eq!(bytes, b"\x1b[<80;5;3M");
    }

    #[test]
    fn encodes_legacy_mouse_wheel() {
        let bytes = encode_mouse_wheel(
            MouseWheelDirection::Down,
            0,
            0,
            KeyModifiers::default(),
            false,
        );
        assert_eq!(bytes, [0x1b, b'[', b'M', 97, 33, 33]);
    }

    #[test]
    fn normalizes_pasted_newlines_for_the_terminal() {
        assert_eq!(
            encode_paste("one\r\ntwo\n", TerminalMode::default()),
            b"one\rtwo\r"
        );
    }

    #[test]
    fn wraps_bracketed_paste_without_converting_newlines() {
        let mode = TerminalMode {
            bracketed_paste: true,
            ..TerminalMode::default()
        };
        assert_eq!(
            encode_paste("one\r\ntwo", mode),
            b"\x1b[200~one\ntwo\x1b[201~"
        );
    }

    #[test]
    fn encodes_sgr_mouse_events() {
        // Left press
        assert_eq!(
            encode_mouse_event(
                MouseEventKind::Press(TerminalMouseButton::Left),
                10,
                5,
                KeyModifiers::default(),
                true,
            ),
            b"\x1b[<0;11;6M"
        );

        // Left release
        assert_eq!(
            encode_mouse_event(
                MouseEventKind::Release(TerminalMouseButton::Left),
                10,
                5,
                KeyModifiers::default(),
                true,
            ),
            b"\x1b[<0;11;6m"
        );

        // Middle press with shift
        assert_eq!(
            encode_mouse_event(
                MouseEventKind::Press(TerminalMouseButton::Middle),
                0,
                0,
                KeyModifiers {
                    shift: true,
                    ..KeyModifiers::default()
                },
                true,
            ),
            b"\x1b[<5;1;1M"
        );

        // Right release with ctrl and alt
        assert_eq!(
            encode_mouse_event(
                MouseEventKind::Release(TerminalMouseButton::Right),
                2,
                3,
                KeyModifiers {
                    control: true,
                    alt: true,
                    ..KeyModifiers::default()
                },
                true,
            ),
            b"\x1b[<26;3;4m"
        );

        // Left drag (motion with button held)
        assert_eq!(
            encode_mouse_event(
                MouseEventKind::Drag(TerminalMouseButton::Left),
                15,
                20,
                KeyModifiers::default(),
                true,
            ),
            b"\x1b[<32;16;21M"
        );

        // Right drag with shift
        assert_eq!(
            encode_mouse_event(
                MouseEventKind::Drag(TerminalMouseButton::Right),
                15,
                20,
                KeyModifiers {
                    shift: true,
                    ..KeyModifiers::default()
                },
                true,
            ),
            b"\x1b[<38;16;21M"
        );

        // Move without buttons held (code 35)
        assert_eq!(
            encode_mouse_event(MouseEventKind::Move, 7, 8, KeyModifiers::default(), true,),
            b"\x1b[<35;8;9M"
        );
    }

    #[test]
    fn encodes_legacy_mouse_events() {
        // Left press: code 0 + 32 = 32 (' ')
        assert_eq!(
            encode_mouse_event(
                MouseEventKind::Press(TerminalMouseButton::Left),
                0,
                0,
                KeyModifiers::default(),
                false,
            ),
            [0x1b, b'[', b'M', 32, 33, 33]
        );

        // Release: always code 3 + 32 = 35 ('#')
        assert_eq!(
            encode_mouse_event(
                MouseEventKind::Release(TerminalMouseButton::Right),
                0,
                0,
                KeyModifiers::default(),
                false,
            ),
            [0x1b, b'[', b'M', 35, 33, 33]
        );

        // Left drag: code 32 + 32 = 64 ('@')
        assert_eq!(
            encode_mouse_event(
                MouseEventKind::Drag(TerminalMouseButton::Left),
                1,
                2,
                KeyModifiers::default(),
                false,
            ),
            [0x1b, b'[', b'M', 64, 34, 35]
        );

        // Move: code 35 + 32 = 67 ('C')
        assert_eq!(
            encode_mouse_event(MouseEventKind::Move, 1, 2, KeyModifiers::default(), false,),
            [0x1b, b'[', b'M', 67, 34, 35]
        );
    }
}
