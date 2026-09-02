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
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToyotermConfig {
    pub font: FontConfig,
    pub colors: ColorConfig,
    pub window_opacity: f32,
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
            },
            window_opacity: 1.0,
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
    home_directory().map(|home| home.join(".config").join("toyoterm").join("config.rb"))
}

pub fn home_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    home.filter(|path| !path.is_empty()).map(PathBuf::from)
}
