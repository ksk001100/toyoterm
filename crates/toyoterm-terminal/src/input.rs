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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseWheelDirection {
    Up,
    Down,
}

impl KeyPress {
    pub fn new(key: TerminalKey, modifiers: KeyModifiers) -> Self {
        Self { key, modifiers }
    }
}

pub fn encode_key(press: &KeyPress, mode: TerminalMode) -> Option<Vec<u8>> {
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
}
