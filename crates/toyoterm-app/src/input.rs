use super::*;

pub(super) fn key_press(
    event: &KeyEvent,
    modifiers: ModifiersState,
    mode: crate::TerminalMode,
) -> Option<KeyPress> {
    let key = if mode.application_keypad
        && let Some(key) = keypad_key(event.physical_key)
    {
        TerminalKey::Keypad(key)
    } else {
        match &event.logical_key {
            Key::Named(named) => named_key(named)?,
            Key::Character(text) => {
                TerminalKey::Text(event.text.as_deref().unwrap_or(text.as_str()).to_owned())
            }
            _ => return None,
        }
    };
    Some(KeyPress::new(key, key_modifiers(modifiers)))
}

pub(super) fn keypad_key(physical_key: PhysicalKey) -> Option<KeypadKey> {
    Some(match physical_key {
        PhysicalKey::Code(KeyCode::Numpad0) => KeypadKey::Digit(0),
        PhysicalKey::Code(KeyCode::Numpad1) => KeypadKey::Digit(1),
        PhysicalKey::Code(KeyCode::Numpad2) => KeypadKey::Digit(2),
        PhysicalKey::Code(KeyCode::Numpad3) => KeypadKey::Digit(3),
        PhysicalKey::Code(KeyCode::Numpad4) => KeypadKey::Digit(4),
        PhysicalKey::Code(KeyCode::Numpad5) => KeypadKey::Digit(5),
        PhysicalKey::Code(KeyCode::Numpad6) => KeypadKey::Digit(6),
        PhysicalKey::Code(KeyCode::Numpad7) => KeypadKey::Digit(7),
        PhysicalKey::Code(KeyCode::Numpad8) => KeypadKey::Digit(8),
        PhysicalKey::Code(KeyCode::Numpad9) => KeypadKey::Digit(9),
        PhysicalKey::Code(KeyCode::NumpadAdd) => KeypadKey::Add,
        PhysicalKey::Code(KeyCode::NumpadSubtract) => KeypadKey::Subtract,
        PhysicalKey::Code(KeyCode::NumpadMultiply) => KeypadKey::Multiply,
        PhysicalKey::Code(KeyCode::NumpadDivide) => KeypadKey::Divide,
        PhysicalKey::Code(KeyCode::NumpadDecimal) => KeypadKey::Decimal,
        PhysicalKey::Code(KeyCode::NumpadComma) => KeypadKey::Comma,
        PhysicalKey::Code(KeyCode::NumpadEqual) => KeypadKey::Equal,
        PhysicalKey::Code(KeyCode::NumpadEnter) => KeypadKey::Enter,
        _ => return None,
    })
}

pub(super) fn should_handle_key_event(state: ElementState, _repeat: bool) -> bool {
    state == ElementState::Pressed
}

pub(super) fn clear_modifier_state(modifiers: &mut ModifiersState, alt_graph_active: &mut bool) {
    *modifiers = ModifiersState::empty();
    *alt_graph_active = false;
}

pub(super) fn effective_modifiers(
    mut modifiers: ModifiersState,
    alt_graph_active: bool,
) -> ModifiersState {
    if alt_graph_active {
        // AltGr is commonly exposed as the right Alt key together with a
        // synthetic Control modifier. Treat it as a text-layout modifier,
        // rather than emitting a control byte with an escape prefix.
        modifiers.remove(ModifiersState::CONTROL | ModifiersState::ALT);
    }
    modifiers
}

pub(super) fn keybinding_names(event: &KeyEvent, modifiers: ModifiersState) -> Vec<String> {
    let modifiers = key_modifiers(modifiers);
    let physical = match event.physical_key {
        PhysicalKey::Code(code) => Some(format!("{code:?}")),
        PhysicalKey::Unidentified(_) => None,
    };
    binding_candidates(physical, logical_binding_key(&event.logical_key), modifiers)
}

pub(super) fn binding_candidates(
    physical: Option<String>,
    logical: Option<String>,
    modifiers: KeyModifiers,
) -> Vec<String> {
    physical
        .map(|key| KeyChord::new(BindingKey::Physical(key), modifiers).canonical_name())
        .into_iter()
        .chain(
            logical.map(|key| KeyChord::new(BindingKey::Logical(key), modifiers).canonical_name()),
        )
        .collect()
}

#[cfg(test)]
pub(super) fn keybinding_name(logical_key: &Key, modifiers: ModifiersState) -> Option<String> {
    let key = logical_binding_key(logical_key)?;
    Some(KeyChord::new(BindingKey::Logical(key), key_modifiers(modifiers)).canonical_name())
}

pub(super) fn logical_binding_key(logical_key: &Key) -> Option<String> {
    Some(match logical_key {
        Key::Character(text) if !text.is_empty() => text.to_uppercase(),
        Key::Named(key) => keybinding_named_key(key)?.to_owned(),
        _ => return None,
    })
}

pub(super) fn keybinding_named_key(key: &NamedKey) -> Option<&'static str> {
    Some(match key {
        NamedKey::Enter => "ENTER",
        NamedKey::Backspace => "BACKSPACE",
        NamedKey::Tab => "TAB",
        NamedKey::Escape => "ESCAPE",
        NamedKey::Space => "SPACE",
        NamedKey::ArrowUp => "UP",
        NamedKey::ArrowDown => "DOWN",
        NamedKey::ArrowLeft => "LEFT",
        NamedKey::ArrowRight => "RIGHT",
        NamedKey::Home => "HOME",
        NamedKey::End => "END",
        NamedKey::PageUp => "PAGEUP",
        NamedKey::PageDown => "PAGEDOWN",
        NamedKey::Insert => "INSERT",
        NamedKey::Delete => "DELETE",
        NamedKey::F1 => "F1",
        NamedKey::F2 => "F2",
        NamedKey::F3 => "F3",
        NamedKey::F4 => "F4",
        NamedKey::F5 => "F5",
        NamedKey::F6 => "F6",
        NamedKey::F7 => "F7",
        NamedKey::F8 => "F8",
        NamedKey::F9 => "F9",
        NamedKey::F10 => "F10",
        NamedKey::F11 => "F11",
        NamedKey::F12 => "F12",
        _ => return None,
    })
}

pub(super) fn key_modifiers(modifiers: ModifiersState) -> KeyModifiers {
    KeyModifiers {
        shift: modifiers.shift_key(),
        control: modifiers.control_key(),
        alt: modifiers.alt_key(),
        super_key: modifiers.super_key(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ShortcutPlatform {
    MacOs,
    LinuxOrWindows,
}

pub(super) fn current_shortcut_platform() -> ShortcutPlatform {
    if cfg!(target_os = "macos") {
        ShortcutPlatform::MacOs
    } else {
        ShortcutPlatform::LinuxOrWindows
    }
}

pub(super) fn has_link_modifier(modifiers: ModifiersState, platform: ShortcutPlatform) -> bool {
    match platform {
        ShortcutPlatform::MacOs => modifiers.super_key(),
        ShortcutPlatform::LinuxOrWindows => modifiers.control_key(),
    }
}

pub(super) fn hyperlink_at(
    snapshot: &toyoterm_terminal::TerminalSnapshot,
    column: u16,
    row: u16,
) -> Option<String> {
    snapshot
        .cells
        .get(usize::from(row))?
        .iter()
        .find(|cell| {
            let end = cell.column.saturating_add(u16::from(cell.width.max(1)));
            (cell.column..end).contains(&column)
        })?
        .hyperlink
        .clone()
}

pub(super) fn validate_allowed_url(url: &str) -> Result<(), String> {
    if url.len() > 2_048 || url.chars().any(char::is_control) {
        return Err("URL is invalid or too long".to_owned());
    }
    let Some((scheme, _)) = url.split_once(':') else {
        return Err("URL has no scheme".to_owned());
    };
    if !matches!(
        scheme.to_ascii_lowercase().as_str(),
        "http" | "https" | "mailto"
    ) {
        return Err(format!("URL scheme {scheme:?} is not allowed"));
    }
    Ok(())
}

pub(super) fn open_allowed_url(url: &str) -> Result<(), String> {
    validate_allowed_url(url)?;

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(url);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("rundll32.exe");
        command.args(["url.dll,FileProtocolHandler", url]);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(url);
        command
    };

    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("launch URL handler: {error}"))
}

pub(super) fn named_key(key: &NamedKey) -> Option<TerminalKey> {
    Some(match key {
        NamedKey::Enter => TerminalKey::Enter,
        NamedKey::Backspace => TerminalKey::Backspace,
        NamedKey::Tab => TerminalKey::Tab,
        NamedKey::Escape => TerminalKey::Escape,
        NamedKey::Space => TerminalKey::Text(" ".into()),
        NamedKey::ArrowUp => TerminalKey::ArrowUp,
        NamedKey::ArrowDown => TerminalKey::ArrowDown,
        NamedKey::ArrowLeft => TerminalKey::ArrowLeft,
        NamedKey::ArrowRight => TerminalKey::ArrowRight,
        NamedKey::Home => TerminalKey::Home,
        NamedKey::End => TerminalKey::End,
        NamedKey::PageUp => TerminalKey::PageUp,
        NamedKey::PageDown => TerminalKey::PageDown,
        NamedKey::Insert => TerminalKey::Insert,
        NamedKey::Delete => TerminalKey::Delete,
        NamedKey::F1 => TerminalKey::Function(1),
        NamedKey::F2 => TerminalKey::Function(2),
        NamedKey::F3 => TerminalKey::Function(3),
        NamedKey::F4 => TerminalKey::Function(4),
        NamedKey::F5 => TerminalKey::Function(5),
        NamedKey::F6 => TerminalKey::Function(6),
        NamedKey::F7 => TerminalKey::Function(7),
        NamedKey::F8 => TerminalKey::Function(8),
        NamedKey::F9 => TerminalKey::Function(9),
        NamedKey::F10 => TerminalKey::Function(10),
        NamedKey::F11 => TerminalKey::Function(11),
        NamedKey::F12 => TerminalKey::Function(12),
        _ => return None,
    })
}
