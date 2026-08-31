use std::collections::HashSet;
use std::ffi::{CStr, CString, c_char, c_void};
use std::fmt;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::rc::Rc;

use crate::{Command, PaneId};

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

  class KeyBindingContext
    attr_reader :pane

    def initialize(pane)
      @pane = pane
    end
  end

  class Event
    attr_reader :name, :pane

    def initialize(name, pane)
      @name = name
      @pane = pane
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
      @bindings = {}
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

    def bind(key, &block)
      raise ArgumentError, "key binding requires a block" unless block
      key = key.to_s.upcase
      raise ArgumentError, "key binding cannot be empty" if key.empty?
      @bindings[key] = block
    end

    def __binding_count
      @bindings.length
    end

    def __binding_key(index)
      @bindings.keys[index]
    end

    def __trigger_binding(key, pane)
      callback = @bindings[key.to_s.upcase]
      return false unless callback
      checkpoint = Toyoterm.__command_checkpoint
      begin
        callback.call(KeyBindingContext.new(pane))
      rescue => error
        Toyoterm.__rollback_commands(checkpoint)
        raise error
      end
      true
    end
  end

  class Pane
    attr_reader :id

    def initialize(id)
      @id = id
    end

    def send_text(text)
      text = text.to_s
      raise ArgumentError, "text contains a NUL byte" if text.index("\0")
      Toyoterm.__queue_command(:send_text, @id, text)
      self
    end
  end

  @config = Config.new
  @current_pane = Pane.new(0)
  @commands = []
  @current_command = nil
  @reload_requested = false
  @event_handlers = {}

  def self.configure(&block)
    block.call(@config)
  end

  def self.__config
    @config
  end

  def self.current_pane
    @current_pane
  end

  def self.reload_config
    @reload_requested = true
    nil
  end

  def self.on(name, &block)
    raise ArgumentError, "event handler requires a block" unless block
    name = name.to_s
    raise ArgumentError, "event name cannot be empty" if name.empty?
    (@event_handlers[name] ||= []) << block
    block
  end

  def self.__event_count
    @event_handlers.length
  end

  def self.__event_name(index)
    @event_handlers.keys[index]
  end

  def self.__emit_event(name, pane)
    handlers = @event_handlers[name.to_s]
    return false unless handlers
    checkpoint = __command_checkpoint
    begin
      event = Event.new(name.to_sym, pane)
      handlers.each { |handler| handler.call(event) }
    rescue => error
      __rollback_commands(checkpoint)
      raise error
    end
    true
  end

  def self.__take_reload_request
    requested = @reload_requested
    @reload_requested = false
    requested
  end

  def self.__set_current_pane(id)
    @current_pane = Pane.new(id)
  end

  def self.__queue_command(type, pane_id, payload)
    @commands << [type, pane_id, payload]
  end

  def self.__command_checkpoint
    @commands.length
  end

  def self.__rollback_commands(checkpoint)
    @commands.pop while @commands.length > checkpoint
  end

  def self.__next_command
    @current_command = @commands.shift
    @current_command ? @current_command[0].to_s : ""
  end

  def self.__current_command_pane
    @current_command[1]
  end

  def self.__current_command_payload
    @current_command[2]
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

    fn config_file(path: &Path, error: impl fmt::Display) -> Self {
        Self::new("load config", format!("{}: {error}", path.display()))
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
    keybindings: HashSet<String>,
    event_names: HashSet<String>,
    source_path: Option<PathBuf>,
}

impl ConfigManager {
    pub fn new() -> Result<Self, ScriptError> {
        let (runtime, config, keybindings, event_names) = load_config("")?;
        Ok(Self {
            runtime,
            config,
            keybindings,
            event_names,
            source_path: None,
        })
    }

    pub fn config(&self) -> &ToyotermConfig {
        &self.config
    }

    pub fn load_startup(explicit_path: Option<&Path>) -> Result<Self, ScriptError> {
        let env_path = std::env::var_os("TOYOTERM_CONFIG_FILE").filter(|path| !path.is_empty());
        let home = home_directory();
        let mut manager = Self::new()?;
        let Some(path) = resolve_config_path(explicit_path, env_path.as_deref(), home.as_deref())
        else {
            return Ok(manager);
        };
        let required = explicit_path.is_some() || env_path.is_some();
        manager.source_path = Some(path.clone());
        if !required && !path.exists() {
            return Ok(manager);
        }
        manager.reload_file()?;
        Ok(manager)
    }

    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }

    /// Reloads the selected config file, preserving the active VM on any failure.
    pub fn reload_file(&mut self) -> Result<&ToyotermConfig, ScriptError> {
        let path = self.source_path.clone().ok_or_else(|| {
            ScriptError::new("reload config", "no configuration path is available")
        })?;
        let source = std::fs::read_to_string(&path)
            .map_err(|error| ScriptError::config_file(&path, error))?;
        self.reload(&source)
            .map_err(|error| ScriptError::config_file(&path, error))
    }

    /// Evaluate config in a fresh VM and swap it in only after complete validation.
    pub fn reload(&mut self, source: &str) -> Result<&ToyotermConfig, ScriptError> {
        let (runtime, config, keybindings, event_names) = load_config(source)?;
        self.runtime = runtime;
        self.config = config;
        self.keybindings = keybindings;
        self.event_names = event_names;
        Ok(&self.config)
    }

    pub fn eval(&mut self, source: &str) -> Result<String, ScriptError> {
        self.runtime.eval(source)
    }

    /// Updates the pane exposed by `Toyoterm.current_pane` for subsequent evaluations.
    pub fn set_current_pane(&mut self, pane: PaneId) -> Result<(), ScriptError> {
        self.runtime
            .eval(&format!("Toyoterm.__set_current_pane({})", pane.0))?;
        Ok(())
    }

    /// Runs a configured callback only when the native key resolver found a match.
    pub fn trigger_keybinding(
        &mut self,
        key: &str,
        current_pane: PaneId,
    ) -> Result<bool, ScriptError> {
        let key = key.to_uppercase();
        if !self.keybindings.contains(&key) {
            return Ok(false);
        }
        self.set_current_pane(current_pane)?;
        let key = ruby_string_literal(&key);
        match self.runtime.eval(&format!(
            "Toyoterm.__config.__trigger_binding({key}, Toyoterm.current_pane)"
        ))? {
            value if value == "true" => Ok(true),
            value if value == "false" => Ok(false),
            _ => Err(ScriptError::new(
                "evaluate key binding",
                "callback returned an invalid match state",
            )),
        }
    }

    pub fn take_reload_request(&mut self) -> Result<bool, ScriptError> {
        match self
            .runtime
            .eval("Toyoterm.__take_reload_request")?
            .as_str()
        {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(ScriptError::new(
                "decode mruby command",
                "reload request state is invalid",
            )),
        }
    }

    /// Emits an event only when Ruby registered at least one handler for it.
    pub fn emit_event(&mut self, name: &str, current_pane: PaneId) -> Result<bool, ScriptError> {
        if !self.event_names.contains(name) {
            return Ok(false);
        }
        self.set_current_pane(current_pane)?;
        let name = ruby_string_literal(name);
        match self.runtime.eval(&format!(
            "Toyoterm.__emit_event({name}, Toyoterm.current_pane)"
        ))? {
            value if value == "true" => Ok(true),
            value if value == "false" => Ok(false),
            _ => Err(ScriptError::new(
                "emit mruby event",
                "event handler returned an invalid state",
            )),
        }
    }

    /// Converts commands queued by Ruby into the native command API.
    ///
    /// Pane id zero is a bootstrap placeholder used while startup config is loading.
    pub fn drain_commands(&mut self, current_pane: PaneId) -> Result<Vec<Command>, ScriptError> {
        let mut commands = Vec::new();
        loop {
            let command_type = self.runtime.eval("Toyoterm.__next_command")?;
            if command_type.is_empty() {
                break;
            }

            let pane = self
                .runtime
                .eval("Toyoterm.__current_command_pane")?
                .parse::<u64>()
                .map(PaneId)
                .map_err(|_| ScriptError::new("decode mruby command", "pane id is invalid"))?;
            let pane = if pane.0 == 0 { current_pane } else { pane };
            let payload = self.runtime.eval("Toyoterm.__current_command_payload")?;
            match command_type.as_str() {
                "send_text" => commands.push(Command::SendText {
                    pane,
                    text: payload,
                }),
                other => {
                    return Err(ScriptError::new(
                        "decode mruby command",
                        format!("unsupported command {other}"),
                    ));
                }
            }
        }
        Ok(commands)
    }
}

pub fn default_config_path() -> Option<PathBuf> {
    home_directory().map(|home| home.join(".config").join("toyoterm").join("config.rb"))
}

fn home_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    home.filter(|path| !path.is_empty()).map(PathBuf::from)
}

fn resolve_config_path(
    explicit_path: Option<&Path>,
    env_path: Option<&std::ffi::OsStr>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    explicit_path
        .map(Path::to_owned)
        .or_else(|| env_path.map(PathBuf::from))
        .or_else(|| home.map(|home| home.join(".config").join("toyoterm").join("config.rb")))
}

fn load_config(
    source: &str,
) -> Result<
    (
        MrubyRuntime,
        ToyotermConfig,
        HashSet<String>,
        HashSet<String>,
    ),
    ScriptError,
> {
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
    validate_color("background", &config.colors.background)?;
    validate_color("foreground", &config.colors.foreground)?;
    validate_color("cursor", &config.colors.cursor)?;
    validate_color("selection", &config.colors.selection)?;
    let binding_count = runtime
        .eval("Toyoterm.__config.__binding_count")?
        .parse::<usize>()
        .map_err(|_| ScriptError::new("load key bindings", "binding count is invalid"))?;
    let mut keybindings = HashSet::with_capacity(binding_count);
    for index in 0..binding_count {
        keybindings.insert(runtime.eval(&format!("Toyoterm.__config.__binding_key({index})"))?);
    }

    let event_count = runtime
        .eval("Toyoterm.__event_count")?
        .parse::<usize>()
        .map_err(|_| ScriptError::new("load events", "event count is invalid"))?;
    let mut event_names = HashSet::with_capacity(event_count);
    for index in 0..event_count {
        event_names.insert(runtime.eval(&format!("Toyoterm.__event_name({index})"))?);
    }

    Ok((runtime, config, keybindings, event_names))
}

fn ruby_string_literal(value: &str) -> String {
    let mut literal = String::with_capacity(value.len() + 2);
    literal.push('"');
    for character in value.chars() {
        match character {
            '\\' => literal.push_str("\\\\"),
            '"' => literal.push_str("\\\""),
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

fn validate_color(name: &str, value: &str) -> Result<(), ScriptError> {
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

    #[test]
    fn rejects_invalid_colors_without_replacing_the_config() {
        let mut manager = ConfigManager::new().unwrap();
        manager
            .reload(r##"Toyoterm.configure { |config| config.colors.cursor = "#123456" }"##)
            .unwrap();

        let error = manager
            .reload(r#"Toyoterm.configure { |config| config.colors.cursor = "red" }"#)
            .unwrap_err();

        assert_eq!(error.operation(), "validate config");
        assert_eq!(manager.config().colors.cursor, "#123456");
    }

    #[test]
    fn reloads_the_selected_file_atomically() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "toyoterm-config-{}-{unique}.rb",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "Toyoterm.configure { |config| config.font.size = 17 }",
        )
        .unwrap();
        let mut manager = ConfigManager::load_startup(Some(&path)).unwrap();
        assert_eq!(manager.source_path(), Some(path.as_path()));
        assert_eq!(manager.config().font.size, 17.0);

        std::fs::write(
            &path,
            "Toyoterm.configure { |config| config.font.size = 19 }",
        )
        .unwrap();
        manager.reload_file().unwrap();
        assert_eq!(manager.config().font.size, 19.0);

        std::fs::write(&path, "Toyoterm.configure {").unwrap();
        assert!(manager.reload_file().is_err());
        assert_eq!(manager.config().font.size, 19.0);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn converts_pane_send_text_to_a_native_command() {
        let mut manager = ConfigManager::new().unwrap();
        manager.set_current_pane(PaneId(42)).unwrap();
        manager
            .eval(r#"Toyoterm.current_pane.send_text("echo hello\n")"#)
            .unwrap();

        assert_eq!(
            manager.drain_commands(PaneId(42)).unwrap(),
            vec![Command::SendText {
                pane: PaneId(42),
                text: "echo hello\n".into(),
            }]
        );
        assert!(manager.drain_commands(PaneId(42)).unwrap().is_empty());
    }

    #[test]
    fn resolves_startup_commands_to_the_current_native_pane() {
        let mut manager = ConfigManager::new().unwrap();
        manager
            .reload(r#"Toyoterm.current_pane.send_text("pwd\n")"#)
            .unwrap();

        assert_eq!(
            manager.drain_commands(PaneId(7)).unwrap(),
            vec![Command::SendText {
                pane: PaneId(7),
                text: "pwd\n".into(),
            }]
        );
    }

    #[test]
    fn invokes_only_matching_dynamic_keybindings() {
        let mut manager = ConfigManager::new().unwrap();
        manager
            .reload(
                r#"
                $callback_count = 0
                Toyoterm.configure do |config|
                  config.bind "CTRL+SHIFT+H" do |ctx|
                    $callback_count += 1
                    ctx.pane.send_text("echo from ruby\n")
                  end
                end
                "#,
            )
            .unwrap();

        assert!(!manager.trigger_keybinding("A", PaneId(9)).unwrap());
        assert_eq!(manager.eval("$callback_count").unwrap(), "0");
        assert!(
            manager
                .trigger_keybinding("ctrl+shift+h", PaneId(9))
                .unwrap()
        );
        assert_eq!(manager.eval("$callback_count").unwrap(), "1");
        assert_eq!(
            manager.drain_commands(PaneId(9)).unwrap(),
            vec![Command::SendText {
                pane: PaneId(9),
                text: "echo from ruby\n".into(),
            }]
        );
    }

    #[test]
    fn ruby_keybinding_errors_leave_the_runtime_usable() {
        let mut manager = ConfigManager::new().unwrap();
        manager
            .reload(
                r#"
                Toyoterm.configure do |config|
                  config.bind "CTRL+E" do |ctx|
                    ctx.pane.send_text("must not run\n")
                    raise "broken callback"
                  end
                end
                "#,
            )
            .unwrap();

        let error = manager.trigger_keybinding("CTRL+E", PaneId(4)).unwrap_err();
        assert_eq!(error.operation(), "evaluate mruby");
        assert!(error.message().contains("broken callback"));
        assert_eq!(manager.eval("6 * 7").unwrap(), "42");
        assert!(manager.drain_commands(PaneId(4)).unwrap().is_empty());
    }

    #[test]
    fn exposes_reload_requests_from_ruby_keybindings() {
        let mut manager = ConfigManager::new().unwrap();
        manager
            .reload(
                r#"
                Toyoterm.configure do |config|
                  config.bind("CTRL+SHIFT+R") { Toyoterm.reload_config }
                end
                "#,
            )
            .unwrap();

        assert!(
            manager
                .trigger_keybinding("CTRL+SHIFT+R", PaneId(4))
                .unwrap()
        );
        assert!(manager.take_reload_request().unwrap());
        assert!(!manager.take_reload_request().unwrap());
    }

    #[test]
    fn emits_registered_events_with_the_current_pane() {
        let mut manager = ConfigManager::new().unwrap();
        manager
            .reload(
                r#"
                $event_count = 0
                Toyoterm.on :app_started do |event|
                  $event_count += 1
                  $event_name = event.name
                  $event_pane = event.pane.id
                  event.pane.send_text("echo app started\n")
                end
                "#,
            )
            .unwrap();

        assert!(!manager.emit_event("config_reloaded", PaneId(12)).unwrap());
        assert_eq!(manager.eval("$event_count").unwrap(), "0");
        assert!(manager.emit_event("app_started", PaneId(12)).unwrap());
        assert_eq!(manager.eval("$event_count").unwrap(), "1");
        assert_eq!(manager.eval("$event_name").unwrap(), "app_started");
        assert_eq!(manager.eval("$event_pane").unwrap(), "12");
        assert_eq!(
            manager.drain_commands(PaneId(12)).unwrap(),
            vec![Command::SendText {
                pane: PaneId(12),
                text: "echo app started\n".into(),
            }]
        );
    }

    #[test]
    fn ruby_event_errors_roll_back_commands() {
        let mut manager = ConfigManager::new().unwrap();
        manager
            .reload(
                r#"
                Toyoterm.on :config_reloaded do |event|
                  event.pane.send_text("must not run\n")
                  raise "broken event"
                end
                "#,
            )
            .unwrap();

        let error = manager
            .emit_event("config_reloaded", PaneId(3))
            .unwrap_err();
        assert!(error.message().contains("broken event"));
        assert!(manager.drain_commands(PaneId(3)).unwrap().is_empty());
        assert_eq!(manager.eval("21 * 2").unwrap(), "42");
    }

    #[test]
    fn resolves_config_paths_in_priority_order() {
        let explicit = Path::new("custom.rb");
        let environment = std::ffi::OsStr::new("environment.rb");
        let home = Path::new("/users/toyo");
        assert_eq!(
            resolve_config_path(Some(explicit), Some(environment), Some(home)),
            Some(explicit.to_owned())
        );
        assert_eq!(
            resolve_config_path(None, Some(environment), Some(home)),
            Some(PathBuf::from("environment.rb"))
        );
        assert_eq!(
            resolve_config_path(None, None, Some(home)),
            Some(home.join(".config/toyoterm/config.rb"))
        );
    }
}
