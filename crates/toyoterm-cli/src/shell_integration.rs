pub const BASH: &str = include_str!("../../../shell-integration/toyoterm.bash");
pub const ZSH: &str = include_str!("../../../shell-integration/toyoterm.zsh");
pub const FISH: &str = include_str!("../../../shell-integration/toyoterm.fish");
pub const POWERSHELL: &str = include_str!("../../../shell-integration/toyoterm.ps1");

pub fn script(shell: &str) -> Option<&'static str> {
    match shell.to_ascii_lowercase().as_str() {
        "bash" => Some(BASH),
        "zsh" => Some(ZSH),
        "fish" => Some(FISH),
        "powershell" | "pwsh" => Some(POWERSHELL),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_supported_shell_names() {
        assert_eq!(script("BASH"), Some(BASH));
        assert_eq!(script("pwsh"), Some(POWERSHELL));
        assert_eq!(script("powershell"), Some(POWERSHELL));
        assert_eq!(script("nu"), None);
    }

    #[test]
    fn every_script_emits_cwd_and_command_markers() {
        for source in [BASH, ZSH, FISH, POWERSHELL] {
            assert!(source.contains("]7;file://"));
            assert!(source.contains("]133;C"));
            assert!(source.contains("]133;D;"));
        }
    }
}
