use std::ffi::{CStr, CString, c_char, c_void};
use std::fmt;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

const CONFIG_DSL: &str = r##"
module Toyoterm
  class FontConfig
    attr_accessor :family, :size

    def initialize
      @family = "monospace"
      @size = 14.0
    end
  end

  class ColorConfig
    attr_accessor :background, :foreground, :cursor, :selection

    def initialize
      @background = "#090b0e"
      @foreground = "#dce1e8"
      @cursor = "#f5f7fa"
      @selection = "#375891"
    end
  end

  class WindowConfig
    attr_accessor :opacity

    def initialize
      @opacity = 1.0
    end
  end

  class Config
    attr_accessor :default_shell, :scrollback_lines

    def initialize
      @font = FontConfig.new
      @colors = ColorConfig.new
      @window = WindowConfig.new
      @default_shell = nil
      @scrollback_lines = 10_000
    end

    def font(&block)
      block ? block.call(@font) : @font
    end

    def colors(&block)
      block ? block.call(@colors) : @colors
    end

    def window(&block)
      block ? block.call(@window) : @window
    end
  end

  @config = Config.new

  def self.configure(&block)
    block.call(@config)
  end

  def self.__config
    @config
  end
end
"##;

unsafe extern "C" {
    fn toyoterm_mruby_open() -> *mut c_void;
    fn toyoterm_mruby_close(state: *mut c_void);
    fn toyoterm_mruby_eval(
        state: *mut c_void,
        source: *const c_char,
        output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_string_free(string: *mut c_char);
}

#[derive(Clone, Debug, PartialEq)]
pub struct FontConfig {
    pub family: String,
    pub size: f32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColorConfig {
    pub background: String,
    pub foreground: String,
    pub cursor: String,
    pub selection: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToyotermConfig {
    pub font: FontConfig,
    pub colors: ColorConfig,
    pub window_opacity: f32,
    pub default_shell: Option<String>,
    pub scrollback_lines: usize,
}

impl Default for ToyotermConfig {
    fn default() -> Self {
        Self {
            font: FontConfig {
                family: "monospace".into(),
                size: 14.0,
            },
            colors: ColorConfig {
                background: "#090b0e".into(),
                foreground: "#dce1e8".into(),
                cursor: "#f5f7fa".into(),
                selection: "#375891".into(),
            },
            window_opacity: 1.0,
            default_shell: None,
            scrollback_lines: 10_000,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptError {
    operation: &'static str,
    message: String,
}

impl ScriptError {
    fn new(operation: &'static str, message: impl Into<String>) -> Self {
        Self {
            operation,
            message: message.into(),
        }
    }

    pub fn operation(&self) -> &'static str {
        self.operation
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ScriptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.operation, self.message)
    }
}

impl std::error::Error for ScriptError {}

/// A single-threaded owner for one embedded mruby VM.
pub struct MrubyRuntime {
    state: NonNull<c_void>,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl MrubyRuntime {
    pub fn new() -> Result<Self, ScriptError> {
        // SAFETY: The returned state is exclusively owned by this wrapper and closed in Drop.
        let state = NonNull::new(unsafe { toyoterm_mruby_open() })
            .ok_or_else(|| ScriptError::new("initialize mruby", "mrb_open failed"))?;
        Ok(Self {
            state,
            not_send_or_sync: PhantomData,
        })
    }

    pub fn eval(&mut self, source: &str) -> Result<String, ScriptError> {
        let source = CString::new(source)
            .map_err(|_| ScriptError::new("evaluate mruby", "source contains a NUL byte"))?;
        let mut output = std::ptr::null_mut();
        // SAFETY: `state` is live, `source` is NUL terminated, and the shim initializes `output`.
        let status =
            unsafe { toyoterm_mruby_eval(self.state.as_ptr(), source.as_ptr(), &mut output) };
        let output = NonNull::new(output)
            .ok_or_else(|| ScriptError::new("evaluate mruby", "failed to allocate result"))?;
        // SAFETY: The shim returns a NUL-terminated allocation which remains live until freed below.
        let text = unsafe { CStr::from_ptr(output.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: `output` was allocated by the shim and has not been freed yet.
        unsafe { toyoterm_mruby_string_free(output.as_ptr()) };

        match status {
            0 => Ok(text),
            1 => Err(ScriptError::new("evaluate mruby", text)),
            _ => Err(ScriptError::new(
                "evaluate mruby",
                "mruby evaluation failed",
            )),
        }
    }
}

impl Drop for MrubyRuntime {
    fn drop(&mut self) {
        // SAFETY: This is the only owner, and Drop runs exactly once.
        unsafe { toyoterm_mruby_close(self.state.as_ptr()) };
    }
}

pub struct ConfigManager {
    runtime: MrubyRuntime,
    config: ToyotermConfig,
}

impl ConfigManager {
    pub fn new() -> Result<Self, ScriptError> {
        let (runtime, config) = load_config("")?;
        Ok(Self { runtime, config })
    }

    pub fn config(&self) -> &ToyotermConfig {
        &self.config
    }

    /// Evaluate config in a fresh VM and swap it in only after complete validation.
    pub fn reload(&mut self, source: &str) -> Result<&ToyotermConfig, ScriptError> {
        let (runtime, config) = load_config(source)?;
        self.runtime = runtime;
        self.config = config;
        Ok(&self.config)
    }

    pub fn eval(&mut self, source: &str) -> Result<String, ScriptError> {
        self.runtime.eval(source)
    }
}

fn load_config(source: &str) -> Result<(MrubyRuntime, ToyotermConfig), ScriptError> {
    let mut runtime = MrubyRuntime::new()?;
    runtime.eval(CONFIG_DSL)?;
    runtime.eval(source)?;

    let defaults = ToyotermConfig::default();
    let family = runtime.eval("Toyoterm.__config.font.family")?;
    let font_size = parse_positive_f32("font size", &runtime.eval("Toyoterm.__config.font.size")?)?;
    let opacity = parse_f32(
        "window opacity",
        &runtime.eval("Toyoterm.__config.window.opacity")?,
    )?;
    if !(0.0..=1.0).contains(&opacity) {
        return Err(ScriptError::new(
            "validate config",
            "window opacity must be between 0 and 1",
        ));
    }
    let scrollback_lines = runtime
        .eval("Toyoterm.__config.scrollback_lines")?
        .parse::<usize>()
        .map_err(|_| ScriptError::new("validate config", "scrollback_lines must be an integer"))?;
    let default_shell = runtime.eval("Toyoterm.__config.default_shell")?;

    let config = ToyotermConfig {
        font: FontConfig {
            family,
            size: font_size,
        },
        colors: ColorConfig {
            background: runtime.eval("Toyoterm.__config.colors.background")?,
            foreground: runtime.eval("Toyoterm.__config.colors.foreground")?,
            cursor: runtime.eval("Toyoterm.__config.colors.cursor")?,
            selection: runtime.eval("Toyoterm.__config.colors.selection")?,
        },
        window_opacity: opacity,
        default_shell: if default_shell.is_empty() {
            defaults.default_shell
        } else {
            Some(default_shell)
        },
        scrollback_lines,
    };
    Ok((runtime, config))
}

fn parse_positive_f32(name: &str, value: &str) -> Result<f32, ScriptError> {
    let value = parse_f32(name, value)?;
    if value <= 0.0 {
        return Err(ScriptError::new(
            "validate config",
            format!("{name} must be positive"),
        ));
    }
    Ok(value)
}

fn parse_f32(name: &str, value: &str) -> Result<f32, ScriptError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_ruby_in_a_persistent_vm() {
        let mut runtime = MrubyRuntime::new().unwrap();
        assert_eq!(runtime.eval("$value = 6 * 7").unwrap(), "42");
        assert_eq!(runtime.eval("$value + 1").unwrap(), "43");
    }

    #[test]
    fn loads_the_configuration_dsl() {
        let mut manager = ConfigManager::new().unwrap();
        let config = manager
            .reload(
                r##"
                Toyoterm.configure do |config|
                  config.font do |font|
                    font.family = "JetBrains Mono"
                    font.size = 16
                  end
                  config.colors.background = "#111111"
                  config.window.opacity = 0.92
                  config.default_shell = "/bin/zsh"
                  config.scrollback_lines = 50_000
                end
                "##,
            )
            .unwrap();

        assert_eq!(config.font.family, "JetBrains Mono");
        assert_eq!(config.font.size, 16.0);
        assert_eq!(config.colors.background, "#111111");
        assert_eq!(config.window_opacity, 0.92);
        assert_eq!(config.default_shell.as_deref(), Some("/bin/zsh"));
        assert_eq!(config.scrollback_lines, 50_000);
    }

    #[test]
    fn failed_reload_preserves_the_previous_runtime_and_config() {
        let mut manager = ConfigManager::new().unwrap();
        manager
            .reload("Toyoterm.configure { |config| config.font.size = 18 }")
            .unwrap();

        let error = manager.reload("Toyoterm.configure {").unwrap_err();
        assert_eq!(error.operation(), "evaluate mruby");
        assert_eq!(manager.config().font.size, 18.0);
        assert_eq!(manager.eval("Toyoterm.__config.font.size").unwrap(), "18");
    }
}
