use super::*;

pub(super) fn decode_native_action(
    action: &str,
    argument: &str,
) -> Result<NativeAction, ScriptError> {
    match action {
        "new_tab" => Ok(NativeAction::NewTab),
        "close_pane" => Ok(NativeAction::ClosePane),
        "close_tab" => Ok(NativeAction::CloseTab),
        "new_workspace" => Ok(NativeAction::NewWorkspace),
        "reload_config" => Ok(NativeAction::ReloadConfig),
        "search" => Ok(NativeAction::Search),
        "maximize_window" => Ok(NativeAction::MaximizeWindow),
        "toggle_maximize" => Ok(NativeAction::ToggleMaximize),
        "minimize_window" => Ok(NativeAction::MinimizeWindow),
        "toggle_fullscreen" => Ok(NativeAction::ToggleFullscreen),
        "next_tab" => Ok(NativeAction::NextTab),
        "previous_tab" => Ok(NativeAction::PreviousTab),
        "next_workspace" => Ok(NativeAction::NextWorkspace),
        "previous_workspace" => Ok(NativeAction::PreviousWorkspace),
        "copy_selection" => Ok(NativeAction::CopySelection),
        "paste_clipboard" => Ok(NativeAction::PasteClipboard),
        "start_visual_mode" => Ok(NativeAction::StartVisualMode),
        "toggle_visual_mode" => Ok(NativeAction::ToggleVisualMode),
        "start_visual_selection" => Ok(NativeAction::StartVisualSelection),
        "select_visual_selection" => Ok(NativeAction::SelectVisualSelection),
        "end_visual_selection" => Ok(NativeAction::EndVisualSelection),
        "move_visual_selection" => {
            parse_selection_motion(argument).map(NativeAction::MoveVisualSelection)
        }
        "yank_selection" => Ok(NativeAction::YankSelection),
        "user_command" if !argument.is_empty() => Ok(NativeAction::UserCommand(argument.into())),
        "user_command" => Err(ScriptError::new(
            "load key bindings",
            "user command name cannot be empty",
        )),
        "split" => parse_direction(argument).map(NativeAction::Split),
        "activate_pane" => parse_direction(argument).map(NativeAction::ActivatePane),
        other => Err(ScriptError::new(
            "load key bindings",
            format!("unsupported native action {other}"),
        )),
    }
}

fn parse_selection_motion(motion: &str) -> Result<toyoterm_api::SelectionMotion, ScriptError> {
    match motion.to_ascii_lowercase().as_str() {
        "left" => Ok(toyoterm_api::SelectionMotion::Left),
        "right" => Ok(toyoterm_api::SelectionMotion::Right),
        "up" => Ok(toyoterm_api::SelectionMotion::Up),
        "down" => Ok(toyoterm_api::SelectionMotion::Down),
        "line_start" => Ok(toyoterm_api::SelectionMotion::LineStart),
        "line_end" => Ok(toyoterm_api::SelectionMotion::LineEnd),
        _ => Err(ScriptError::new(
            "load key bindings",
            format!("invalid visual selection motion `{motion}`"),
        )),
    }
}

pub(super) fn parse_direction(direction: &str) -> Result<SplitDirection, ScriptError> {
    match direction.to_ascii_lowercase().as_str() {
        "left" => Ok(SplitDirection::Left),
        "right" => Ok(SplitDirection::Right),
        "up" => Ok(SplitDirection::Up),
        "down" => Ok(SplitDirection::Down),
        _ => Err(ScriptError::new(
            "load key bindings",
            format!("invalid pane direction `{direction}`"),
        )),
    }
}

pub(super) fn ruby_string_literal(value: &str) -> String {
    let mut literal = String::with_capacity(value.len() + 2);
    literal.push('"');
    for character in value.chars() {
        match character {
            '\\' => literal.push_str("\\\\"),
            '"' => literal.push_str("\\\""),
            '#' => literal.push_str("\\#"),
            '\n' => literal.push_str("\\n"),
            '\r' => literal.push_str("\\r"),
            '\t' => literal.push_str("\\t"),
            '\0' => literal.push_str("\\0"),
            character => literal.push(character),
        }
    }
    literal.push('"');
    literal
}

pub(super) fn parse_positive_f32(name: &str, value: &str) -> Result<f32, ScriptError> {
    let value = parse_f32(name, value)?;
    if value <= 0.0 {
        return Err(ScriptError::new(
            "validate config",
            format!("{name} must be positive"),
        ));
    }
    Ok(value)
}

pub(super) fn parse_nonnegative_f32(name: &str, value: &str) -> Result<f32, ScriptError> {
    let value = parse_f32(name, value)?;
    if value < 0.0 {
        return Err(ScriptError::new(
            "validate config",
            format!("{name} must not be negative"),
        ));
    }
    Ok(value)
}

pub(super) fn validate_color(name: &str, value: &str) -> Result<(), ScriptError> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(ScriptError::new(
            "validate config",
            format!("{name} color must be #RRGGBB"),
        ))
    }
}

pub(super) fn parse_f32(name: &str, value: &str) -> Result<f32, ScriptError> {
    let value = value
        .parse::<f32>()
        .map_err(|_| ScriptError::new("validate config", format!("{name} must be numeric")))?;
    if !value.is_finite() {
        return Err(ScriptError::new(
            "validate config",
            format!("{name} must be finite"),
        ));
    }
    Ok(value)
}
