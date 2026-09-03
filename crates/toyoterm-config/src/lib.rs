use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub struct FontConfig {
    pub family: String,
    pub fallback: Vec<String>,
    pub size: f32,
    pub weight: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeaderConfig {
    pub key: String,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColorConfig {
    pub background: String,
    pub foreground: String,
    pub cursor: String,
    pub selection: String,
    /// The configurable ANSI colors (indexes 0 through 15). Indexes 16 through
    /// 255 use the standard xterm color cube and grayscale ramp.
    pub ansi: Vec<String>,
    pub tab_bar: String,
    pub tab_active: String,
    pub tab_inactive: String,
    pub workspace_bar: String,
    pub status_bar: String,
    pub pane_border: String,
    pub search_match: String,
    pub search_match_active: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UiConfig {
    pub padding_x: f32,
    pub padding_y: f32,
    pub line_height: f32,
    pub tab_bar: bool,
    pub tab_bar_height: f32,
    pub tab_width: f32,
    pub workspace_bar: bool,
    pub workspace_bar_height: f32,
    pub workspace_width: f32,
    pub status_bar_height: f32,
    pub pane_divider_width: f32,
    pub active_pane_border_width: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowConfig {
    pub opacity: f32,
    pub width: f32,
    pub height: f32,
    pub min_width: f32,
    pub min_height: f32,
    pub decorations: bool,
    pub resizable: bool,
    pub always_on_top: bool,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BehaviorConfig {
    pub scroll_lines: f32,
    pub copy_on_select: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToyotermConfig {
    pub font: FontConfig,
    pub colors: ColorConfig,
    pub ui: UiConfig,
    pub window: WindowConfig,
    pub behavior: BehaviorConfig,
    pub default_shell: Option<String>,
    pub scrollback_lines: usize,
    pub leader: Option<LeaderConfig>,
    pub status_interval: Option<Duration>,
}

impl Default for ToyotermConfig {
    fn default() -> Self {
        Self {
            font: FontConfig {
                family: "monospace".into(),
                fallback: Vec::new(),
                size: 14.0,
                weight: 400,
            },
            colors: ColorConfig {
                background: "#090b0e".into(),
                foreground: "#dce1e8".into(),
                cursor: "#f5f7fa".into(),
                selection: "#375891".into(),
                ansi: default_ansi_colors(),
                tab_bar: "#11151b".into(),
                tab_active: "#18243a".into(),
                tab_inactive: "#15191f".into(),
                workspace_bar: "#0d1014".into(),
                status_bar: "#101419".into(),
                pane_border: "#375891".into(),
                search_match: "#c4972f".into(),
                search_match_active: "#ffbe3a".into(),
            },
            ui: UiConfig {
                padding_x: 8.0,
                padding_y: 8.0,
                line_height: 1.2857143,
                tab_bar: true,
                tab_bar_height: 30.0,
                tab_width: 160.0,
                workspace_bar: true,
                workspace_bar_height: 24.0,
                workspace_width: 160.0,
                status_bar_height: 24.0,
                pane_divider_width: 2.0,
                active_pane_border_width: 2.0,
            },
            window: WindowConfig {
                opacity: 1.0,
                width: 960.0,
                height: 600.0,
                min_width: 320.0,
                min_height: 180.0,
                decorations: true,
                resizable: true,
                always_on_top: false,
                title: "toyoterm".into(),
            },
            behavior: BehaviorConfig {
                scroll_lines: 3.0,
                copy_on_select: false,
            },
            default_shell: None,
            scrollback_lines: 10_000,
            leader: None,
            status_interval: None,
        }
    }
}

pub fn default_ansi_colors() -> Vec<String> {
    [
        "#000000", "#cd0000", "#00cd00", "#cdcd00", "#0000ee", "#cd00cd", "#00cdcd", "#e5e5e5",
        "#7f7f7f", "#ff0000", "#00ff00", "#ffff00", "#5c5cff", "#ff00ff", "#00ffff", "#ffffff",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

pub fn default_config_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        default_config_path_windows(
            std::env::var_os("APPDATA").as_deref(),
            std::env::var_os("USERPROFILE").as_deref(),
        )
    }
    #[cfg(not(windows))]
    {
        default_config_path_unix(
            std::env::var_os("XDG_CONFIG_HOME").as_deref(),
            std::env::var_os("HOME").as_deref(),
        )
    }
}

pub fn default_plugin_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        default_plugin_dir_windows(
            std::env::var_os("APPDATA").as_deref(),
            std::env::var_os("USERPROFILE").as_deref(),
        )
    }
    #[cfg(not(windows))]
    {
        default_plugin_dir_unix(
            std::env::var_os("XDG_CONFIG_HOME").as_deref(),
            std::env::var_os("HOME").as_deref(),
        )
    }
}

pub fn candidate_config_paths() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        candidate_config_paths_windows(
            std::env::var_os("APPDATA").as_deref(),
            std::env::var_os("USERPROFILE").as_deref(),
        )
    }
    #[cfg(not(windows))]
    {
        candidate_config_paths_unix(
            std::env::var_os("XDG_CONFIG_HOME").as_deref(),
            std::env::var_os("HOME").as_deref(),
        )
    }
}

pub fn home_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    home_directory_from_env(home)
}

#[cfg(any(windows, test))]
pub(crate) fn candidate_config_paths_windows(
    appdata: Option<&std::ffi::OsStr>,
    userprofile: Option<&std::ffi::OsStr>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(appdata) = appdata.filter(|s| !s.is_empty()) {
        candidates.push(PathBuf::from(appdata).join("toyoterm").join("config.rb"));
    }
    if let Some(userprofile) = userprofile.filter(|s| !s.is_empty()) {
        candidates.push(
            PathBuf::from(userprofile)
                .join(".config")
                .join("toyoterm")
                .join("config.rb"),
        );
    }
    candidates
}

#[cfg(any(windows, test))]
pub(crate) fn default_config_path_windows(
    appdata: Option<&std::ffi::OsStr>,
    userprofile: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    let candidates = candidate_config_paths_windows(appdata, userprofile);
    for path in &candidates {
        if path.exists() {
            return Some(path.clone());
        }
    }
    candidates.into_iter().next()
}

#[cfg(any(windows, test))]
pub(crate) fn default_plugin_dir_windows(
    appdata: Option<&std::ffi::OsStr>,
    userprofile: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(appdata) = appdata.filter(|s| !s.is_empty()) {
        candidates.push(PathBuf::from(appdata).join("toyoterm").join("plugins"));
    }
    if let Some(userprofile) = userprofile.filter(|s| !s.is_empty()) {
        candidates.push(
            PathBuf::from(userprofile)
                .join(".config")
                .join("toyoterm")
                .join("plugins"),
        );
    }
    for path in &candidates {
        if path.exists() {
            return Some(path.clone());
        }
    }
    candidates.into_iter().next()
}

#[cfg(any(not(windows), test))]
pub(crate) fn candidate_config_paths_unix(
    xdg_config_home: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(xdg) = xdg_config_home.filter(|s| !s.is_empty()) {
        candidates.push(PathBuf::from(xdg).join("toyoterm").join("config.rb"));
    } else if let Some(home) = home.filter(|s| !s.is_empty()) {
        candidates.push(
            PathBuf::from(home)
                .join(".config")
                .join("toyoterm")
                .join("config.rb"),
        );
    }
    candidates
}

#[cfg(any(not(windows), test))]
pub(crate) fn default_config_path_unix(
    xdg_config_home: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    candidate_config_paths_unix(xdg_config_home, home)
        .into_iter()
        .next()
}

#[cfg(any(not(windows), test))]
pub(crate) fn default_plugin_dir_unix(
    xdg_config_home: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    if let Some(xdg) = xdg_config_home.filter(|s| !s.is_empty()) {
        Some(PathBuf::from(xdg).join("toyoterm").join("plugins"))
    } else {
        home.filter(|s| !s.is_empty()).map(|home| {
            PathBuf::from(home)
                .join(".config")
                .join("toyoterm")
                .join("plugins")
        })
    }
}

fn home_directory_from_env(home: Option<OsString>) -> Option<PathBuf> {
    home.filter(|path| !path.is_empty()).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_documented_values() {
        let config = ToyotermConfig::default();

        assert_eq!(
            config.font,
            FontConfig {
                family: "monospace".into(),
                fallback: Vec::new(),
                size: 14.0,
                weight: 400,
            }
        );
        assert_eq!(config.colors.background, "#090b0e");
        assert_eq!(config.colors.foreground, "#dce1e8");
        assert_eq!(config.colors.cursor, "#f5f7fa");
        assert_eq!(config.colors.selection, "#375891");
        assert_eq!(config.colors.ansi, default_ansi_colors());
        assert_eq!(config.window.opacity, 1.0);
        assert_eq!(config.default_shell, None);
        assert_eq!(config.scrollback_lines, 10_000);
        assert_eq!(config.leader, None);
        assert_eq!(config.status_interval, None);
    }

    #[test]
    fn default_ansi_palette_has_all_sixteen_slots_in_order() {
        assert_eq!(
            default_ansi_colors(),
            [
                "#000000", "#cd0000", "#00cd00", "#cdcd00", "#0000ee", "#cd00cd", "#00cdcd",
                "#e5e5e5", "#7f7f7f", "#ff0000", "#00ff00", "#ffff00", "#5c5cff", "#ff00ff",
                "#00ffff", "#ffffff",
            ]
        );
    }

    #[test]
    fn unix_config_path_prefers_xdg_config_home() {
        let xdg = std::ffi::OsStr::new("/custom/xdg");
        let home = std::ffi::OsStr::new("/home/user");
        assert_eq!(
            default_config_path_unix(Some(xdg), Some(home)),
            Some(PathBuf::from("/custom/xdg/toyoterm/config.rb"))
        );
        assert_eq!(
            default_plugin_dir_unix(Some(xdg), Some(home)),
            Some(PathBuf::from("/custom/xdg/toyoterm/plugins"))
        );
    }

    #[test]
    fn unix_config_path_falls_back_to_home_config_when_xdg_empty_or_unset() {
        let home = std::ffi::OsStr::new("/home/user");
        let empty_xdg = std::ffi::OsStr::new("");
        assert_eq!(
            default_config_path_unix(None, Some(home)),
            Some(PathBuf::from("/home/user/.config/toyoterm/config.rb"))
        );
        assert_eq!(
            default_config_path_unix(Some(empty_xdg), Some(home)),
            Some(PathBuf::from("/home/user/.config/toyoterm/config.rb"))
        );
        assert_eq!(
            default_plugin_dir_unix(None, Some(home)),
            Some(PathBuf::from("/home/user/.config/toyoterm/plugins"))
        );
    }

    #[test]
    fn windows_config_candidates_list_appdata_then_userprofile() {
        let appdata = std::ffi::OsStr::new(r"C:\Users\toyo\AppData\Roaming");
        let userprofile = std::ffi::OsStr::new(r"C:\Users\toyo");
        let candidates = candidate_config_paths_windows(Some(appdata), Some(userprofile));
        assert_eq!(
            candidates,
            vec![
                PathBuf::from(r"C:\Users\toyo\AppData\Roaming\toyoterm\config.rb"),
                PathBuf::from(r"C:\Users\toyo\.config\toyoterm\config.rb"),
            ]
        );
        // If neither exists on disk, default is first candidate (APPDATA)
        assert_eq!(
            default_config_path_windows(Some(appdata), Some(userprofile)),
            Some(PathBuf::from(
                r"C:\Users\toyo\AppData\Roaming\toyoterm\config.rb"
            ))
        );
    }

    #[test]
    fn windows_config_path_falls_back_to_userprofile_when_appdata_unset() {
        let userprofile = std::ffi::OsStr::new(r"C:\Users\toyo");
        assert_eq!(
            default_config_path_windows(None, Some(userprofile)),
            Some(PathBuf::from(r"C:\Users\toyo\.config\toyoterm\config.rb"))
        );
        assert_eq!(
            default_plugin_dir_windows(None, Some(userprofile)),
            Some(PathBuf::from(r"C:\Users\toyo\.config\toyoterm\plugins"))
        );
    }

    #[test]
    fn missing_or_empty_home_directory_is_ignored() {
        assert_eq!(home_directory_from_env(None), None);
        assert_eq!(home_directory_from_env(Some(OsString::new())), None);
        assert_eq!(
            home_directory_from_env(Some(OsString::from("relative-home"))),
            Some(PathBuf::from("relative-home"))
        );
    }
}
