use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString, c_char, c_void};
use std::fmt;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::{
    Command, HandleKind, NativeAction, NativeCommand, NativeHandle, PaneId, SplitDirection, TabId,
    WindowId, WorkspaceId,
};

const SLOW_CALLBACK_THRESHOLD: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CallbackKind {
    KeyBinding,
    Event,
}

impl CallbackKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::KeyBinding => "key_binding",
            Self::Event => "event",
        }
    }
}

const CONFIG_DSL: &str = r##"
module Toyoterm
  class FontConfig
    attr_accessor :family, :fallback, :size, :weight

    def initialize
      @family = "monospace"
      @fallback = []
      @size = 14.0
      @weight = 400
    end

    def __fallback_count
      raise TypeError, "font fallback must be an array" unless @fallback.is_a?(Array)
      @fallback.length
    end

    def __fallback_at(index)
      family = @fallback[index]
      raise TypeError, "font fallback entries must be strings" unless family.is_a?(String)
      family
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

  class StaticBinding
    def initialize(config, key)
      @config = config
      @key = key
    end

    def activate_pane(direction)
      @config.__register_static(@key, :activate_pane, direction)
      self
    end

    def split(direction)
      @config.__register_static(@key, :split, direction)
      self
    end

    def new_tab
      @config.__register_static(@key, :new_tab, nil)
      self
    end

    def close_pane
      @config.__register_static(@key, :close_pane, nil)
      self
    end

    def reload_config
      @config.__register_static(@key, :reload_config, nil)
      self
    end
  end

  class KeysConfig
    def initialize(config)
      @config = config
    end

    def ctrl(key)
      binding(key, "CTRL")
    end

    def ctrl_shift(key)
      binding(key, "CTRL+SHIFT")
    end

    def primary(key)
      binding(key, Toyoterm.__primary_modifier)
    end

    def primary_shift(key)
      mods = Toyoterm.__primary_modifier
      binding(key, mods == "SUPER" ? "SHIFT+SUPER" : "#{mods}+SHIFT")
    end

    def alt(key)
      binding(key, "ALT")
    end

    def super_key(key)
      binding(key, "SUPER")
    end

    def leader(key)
      binding(key, "LEADER")
    end

    def physical(key, mods = "")
      prefix = mods.to_s.upcase
      prefix = "#{prefix}+" unless prefix.empty?
      StaticBinding.new(@config, "#{prefix}PHYSICAL:#{key.to_s.upcase}")
    end

    private

    def binding(key, mods)
      StaticBinding.new(@config, "#{mods}+#{key.to_s.upcase}")
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
      @static_bindings = {}
      @leader_key = nil
      @leader_timeout = 1000
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
      raise ArgumentError, "duplicate key binding: #{key}" if @bindings.key?(key) || @static_bindings.key?(key)
      @bindings[key] = block
    end

    def keys(&block)
      keys = KeysConfig.new(self)
      return keys unless block
      block.arity == 0 ? keys.instance_eval(&block) : block.call(keys)
      keys
    end

    def leader(key:, mods: "", timeout: 1000)
      key = key.to_s.upcase
      raise ArgumentError, "leader key cannot be empty" if key.empty?
      mods = mods.to_s.upcase
      @leader_key = mods.empty? ? key : "#{mods}+#{key}"
      @leader_timeout = timeout
      self
    end

    def __leader_key
      @leader_key || ""
    end

    def __leader_timeout
      @leader_timeout
    end

    def __register_static(key, action, argument)
      key = key.to_s.upcase
      raise ArgumentError, "duplicate key binding: #{key}" if @bindings.key?(key) || @static_bindings.key?(key)
      @static_bindings[key] = [action, argument]
    end

    def __static_binding_count
      @static_bindings.length
    end

    def __static_binding_key(index)
      @static_bindings.keys[index]
    end

    def __static_binding_action(index)
      @static_bindings.values[index][0]
    end

    def __static_binding_argument(index)
      @static_bindings.values[index][1]
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

  class InvalidHandleError < RuntimeError
    attr_reader :kind, :id

    def initialize(kind, id)
      @kind = kind
      @id = id
      super("invalid #{kind} handle #{id}")
    end
  end

  class NativeHandle
    attr_reader :id

    def initialize(id)
      unless id.is_a?(Integer) && id >= 0
        raise ArgumentError, "native handle id must be a non-negative integer"
      end
      @id = id
    end

    def ==(other)
      other.class == self.class && other.id == @id
    end

    alias eql? ==

    def hash
      self.class.hash ^ @id.hash
    end

    def inspect
      "#<#{self.class}:#{@id}>"
    end

    def valid?
      Toyoterm.__handle_valid?(__native_kind, @id)
    end

    def validate!
      raise InvalidHandleError.new(__native_kind, @id) unless valid?
      self
    end

    private

    def __native_kind
      raise NotImplementedError, "native handle kind is not defined"
    end
  end

  class Workspace < NativeHandle
    def name
      validate!
      Toyoterm.__object_data(:workspace, @id)[0]
    end

    def windows
      validate!
      Toyoterm.__object_data(:workspace, @id)[1].map { |id| Window.new(id) }
    end

    def activate
      validate!
      Toyoterm.__queue_command(:activate_workspace, @id, nil)
      self
    end

    def create_window
      validate!
      Toyoterm.__queue_command(:create_window, @id, nil)
      self
    end

    private
    def __native_kind; :workspace; end
  end

  class Window < NativeHandle
    def tabs
      validate!
      Toyoterm.__object_data(:window, @id)[0].map { |id| Tab.new(id) }
    end

    def new_tab
      validate!
      Toyoterm.__queue_command(:new_tab, @id, nil)
      self
    end

    def close
      validate!
      Toyoterm.__queue_command(:close_window, @id, nil)
      self
    end

    def focus
      validate!
      Toyoterm.__queue_command(:activate_window, @id, nil)
      self
    end

    private
    def __native_kind; :window; end
  end

  class Tab < NativeHandle
    def title
      validate!
      Toyoterm.__object_data(:tab, @id)[0]
    end

    def panes
      validate!
      Toyoterm.__object_data(:tab, @id)[1].map { |id| Pane.new(id) }
    end

    def close
      validate!
      Toyoterm.__queue_command(:close_tab, @id, nil)
      self
    end

    def focus
      validate!
      Toyoterm.__queue_command(:activate_tab, @id, nil)
      self
    end

    alias activate focus

    private
    def __native_kind; :tab; end
  end

  class Pane < NativeHandle

    def title
      validate!
      Toyoterm.__object_data(:pane, @id)[0]
    end

    def cwd
      validate!
      Toyoterm.__object_data(:pane, @id)[1]
    end

    def pid
      validate!
      Toyoterm.__object_data(:pane, @id)[2]
    end

    def split(direction)
      validate!
      direction = direction.to_s.downcase
      unless ["left", "right", "up", "down"].include?(direction)
        raise ArgumentError, "split direction must be left, right, up, or down"
      end
      Toyoterm.__queue_command(:split, @id, direction)
      self
    end

    def close
      validate!
      Toyoterm.__queue_command(:close_pane, @id, nil)
      self
    end

    def focus
      validate!
      Toyoterm.__queue_command(:activate_pane, @id, nil)
      self
    end

    def badge
      validate!
      Toyoterm.__pane_badge(@id)
    end

    def badge=(value)
      validate!
      Toyoterm.__set_pane_badge(@id, value.nil? ? nil : value.to_s)
    end

    def send_text(text)
      validate!
      text = text.to_s
      raise ArgumentError, "text contains a NUL byte" if text.index("\0")
      Toyoterm.__queue_command(:send_text, @id, text)
      self
    end

    private
    def __native_kind; :pane; end
  end

  class Clipboard
    def initialize
      @text = nil
    end

    def read
      raise RuntimeError, "clipboard is unavailable" if @text.nil?
      @text.dup
    end

    def write(text)
      text = text.to_s
      raise ArgumentError, "clipboard text contains a NUL byte" if text.index("\0")
      Toyoterm.__queue_command(:clipboard_write, 0, text)
      self
    end

    def __replace(text)
      @text = text
    end
  end

  @config = Config.new
  @current_pane = Pane.new(0)
  @current_tab = Tab.new(0)
  @current_window = Window.new(0)
  @current_workspace = Workspace.new(0)
  @clipboard = Clipboard.new
  @commands = []
  @current_command = nil
  @event_handlers = {}
  @live_handles = {
    workspace: [0],
    window: [0],
    tab: [0],
    pane: [0]
  }
  @object_data = { workspace: {}, window: {}, tab: {}, pane: {} }
  @pane_badges = {}

  def self.configure(&block)
    block.call(@config)
  end

  def self.__config
    @config
  end

  def self.current_pane
    @current_pane
  end

  def self.current_tab
    @current_tab
  end

  def self.current_window
    @current_window
  end

  def self.current_workspace
    @current_workspace
  end

  def self.windows
    @object_data[:window].keys.sort.map { |id| Window.new(id) }
  end

  def self.workspaces
    @object_data[:workspace].keys.sort.map { |id| Workspace.new(id) }
  end

  def self.workspace(name)
    pair = @object_data[:workspace].find { |_id, data| data[0] == name.to_s }
    pair ? Workspace.new(pair[0]) : nil
  end

  def self.clipboard
    @clipboard
  end

  def self.__set_clipboard_text(text)
    @clipboard.__replace(text)
  end

  def self.__primary_modifier
    "__TOYOTERM_PRIMARY_MODIFIER__"
  end

  def self.reload_config
    __queue_command(:reload_config, 0, nil)
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

  def self.__set_current_pane(id)
    @current_pane = Pane.new(id)
    @live_handles[:pane] << id unless @live_handles[:pane].include?(id)
  end

  def self.__reset_object_model(workspace, window, tab, pane)
    @current_workspace = Workspace.new(workspace)
    @current_window = Window.new(window)
    @current_tab = Tab.new(tab)
    @current_pane = Pane.new(pane)
    @object_data = { workspace: {}, window: {}, tab: {}, pane: {} }
  end

  def self.__add_workspace(id, name, windows)
    @object_data[:workspace][id] = [name, windows]
  end

  def self.__add_window(id, tabs)
    @object_data[:window][id] = [tabs]
  end

  def self.__add_tab(id, title, panes)
    @object_data[:tab][id] = [title, panes]
  end

  def self.__add_pane(id, title, cwd, pid)
    @object_data[:pane][id] = [title, cwd, pid]
  end

  def self.__object_data(kind, id)
    data = @object_data[kind][id]
    raise InvalidHandleError.new(kind, id) if data.nil?
    data
  end

  def self.__pane_badge(id)
    @pane_badges[id]
  end

  def self.__set_pane_badge(id, value)
    value.nil? ? @pane_badges.delete(id) : @pane_badges[id] = value
  end

  def self.__replace_live_handles(workspaces, windows, tabs, panes)
    @live_handles = {
      workspace: workspaces,
      window: windows,
      tab: tabs,
      pane: panes
    }
  end

  def self.__handle_valid?(kind, id)
    ids = @live_handles[kind]
    !ids.nil? && ids.include?(id)
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
        filename: *const c_char,
        output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_set_current_pane(
        state: *mut c_void,
        pane_id: u64,
        error_output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_set_live_handles(
        state: *mut c_void,
        workspaces: *const u64,
        workspace_count: usize,
        windows: *const u64,
        window_count: usize,
        tabs: *const u64,
        tab_count: usize,
        panes: *const u64,
        pane_count: usize,
        error_output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_reset_object_model(
        state: *mut c_void,
        workspace_id: u64,
        window_id: u64,
        tab_id: u64,
        pane_id: u64,
        error_output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_add_workspace(
        state: *mut c_void,
        workspace_id: u64,
        name: *const c_char,
        name_length: usize,
        windows: *const u64,
        window_count: usize,
        error_output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_add_window(
        state: *mut c_void,
        window_id: u64,
        tabs: *const u64,
        tab_count: usize,
        error_output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_add_tab(
        state: *mut c_void,
        tab_id: u64,
        title: *const c_char,
        title_length: usize,
        panes: *const u64,
        pane_count: usize,
        error_output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_add_pane(
        state: *mut c_void,
        pane_id: u64,
        title: *const c_char,
        title_length: usize,
        cwd: *const c_char,
        cwd_length: usize,
        cwd_available: i32,
        pid: u64,
        pid_available: i32,
        error_output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_set_clipboard_text(
        state: *mut c_void,
        text: *const c_char,
        length: usize,
        available: i32,
        error_output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_string_free(string: *mut c_char);
}

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
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToyotermConfig {
    pub font: FontConfig,
    pub colors: ColorConfig,
    pub window_opacity: f32,
    pub default_shell: Option<String>,
    pub scrollback_lines: usize,
    pub leader: Option<LeaderConfig>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RubyObjectModel {
    pub current_workspace: WorkspaceId,
    pub current_window: WindowId,
    pub current_tab: TabId,
    pub current_pane: PaneId,
    pub workspaces: Vec<RubyWorkspace>,
    pub windows: Vec<RubyWindow>,
    pub tabs: Vec<RubyTab>,
    pub panes: Vec<RubyPane>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RubyWorkspace {
    pub id: WorkspaceId,
    pub name: String,
    pub windows: Vec<WindowId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RubyWindow {
    pub id: WindowId,
    pub tabs: Vec<TabId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RubyTab {
    pub id: TabId,
    pub title: String,
    pub panes: Vec<PaneId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RubyPane {
    pub id: PaneId,
    pub title: String,
    pub cwd: Option<String>,
    pub pid: Option<u32>,
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
            },
            window_opacity: 1.0,
            default_shell: None,
            scrollback_lines: 10_000,
            leader: None,
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
        self.eval_with_filename(source, "(eval)")
    }

    fn set_current_pane(&mut self, pane: PaneId) -> Result<(), ScriptError> {
        let mut error = std::ptr::null_mut();
        // SAFETY: The VM is exclusively borrowed and the shim initializes `error`.
        let status =
            unsafe { toyoterm_mruby_set_current_pane(self.state.as_ptr(), pane.0, &mut error) };
        typed_call_result("set current pane", status, error)
    }

    fn set_live_handles(
        &mut self,
        workspaces: &[u64],
        windows: &[u64],
        tabs: &[u64],
        panes: &[u64],
    ) -> Result<(), ScriptError> {
        let mut error = std::ptr::null_mut();
        // SAFETY: All slices remain live for the call and the VM is exclusively borrowed.
        let status = unsafe {
            toyoterm_mruby_set_live_handles(
                self.state.as_ptr(),
                workspaces.as_ptr(),
                workspaces.len(),
                windows.as_ptr(),
                windows.len(),
                tabs.as_ptr(),
                tabs.len(),
                panes.as_ptr(),
                panes.len(),
                &mut error,
            )
        };
        typed_call_result("set live handles", status, error)
    }

    fn set_object_model(&mut self, model: &RubyObjectModel) -> Result<(), ScriptError> {
        let mut error = std::ptr::null_mut();
        // SAFETY: The VM is exclusively borrowed and the shim initializes `error`.
        let status = unsafe {
            toyoterm_mruby_reset_object_model(
                self.state.as_ptr(),
                model.current_workspace.0,
                model.current_window.0,
                model.current_tab.0,
                model.current_pane.0,
                &mut error,
            )
        };
        typed_call_result("reset object model", status, error)?;

        for workspace in &model.workspaces {
            let windows = workspace
                .windows
                .iter()
                .map(|window| window.0)
                .collect::<Vec<_>>();
            let mut error = std::ptr::null_mut();
            // SAFETY: String and slice storage remains live for the duration of the call.
            let status = unsafe {
                toyoterm_mruby_add_workspace(
                    self.state.as_ptr(),
                    workspace.id.0,
                    workspace.name.as_ptr().cast(),
                    workspace.name.len(),
                    windows.as_ptr(),
                    windows.len(),
                    &mut error,
                )
            };
            typed_call_result("add workspace object", status, error)?;
        }
        for window in &model.windows {
            let tabs = window.tabs.iter().map(|tab| tab.0).collect::<Vec<_>>();
            let mut error = std::ptr::null_mut();
            // SAFETY: Slice storage remains live for the duration of the call.
            let status = unsafe {
                toyoterm_mruby_add_window(
                    self.state.as_ptr(),
                    window.id.0,
                    tabs.as_ptr(),
                    tabs.len(),
                    &mut error,
                )
            };
            typed_call_result("add window object", status, error)?;
        }
        for tab in &model.tabs {
            let panes = tab.panes.iter().map(|pane| pane.0).collect::<Vec<_>>();
            let mut error = std::ptr::null_mut();
            // SAFETY: String and slice storage remains live for the duration of the call.
            let status = unsafe {
                toyoterm_mruby_add_tab(
                    self.state.as_ptr(),
                    tab.id.0,
                    tab.title.as_ptr().cast(),
                    tab.title.len(),
                    panes.as_ptr(),
                    panes.len(),
                    &mut error,
                )
            };
            typed_call_result("add tab object", status, error)?;
        }
        for pane in &model.panes {
            let (cwd, cwd_len, cwd_available) =
                pane.cwd.as_deref().map_or((std::ptr::null(), 0, 0), |cwd| {
                    (cwd.as_ptr().cast::<c_char>(), cwd.len(), 1)
                });
            let mut error = std::ptr::null_mut();
            // SAFETY: Optional string storage remains live for the duration of the call.
            let status = unsafe {
                toyoterm_mruby_add_pane(
                    self.state.as_ptr(),
                    pane.id.0,
                    pane.title.as_ptr().cast(),
                    pane.title.len(),
                    cwd,
                    cwd_len,
                    cwd_available,
                    pane.pid.unwrap_or_default().into(),
                    i32::from(pane.pid.is_some()),
                    &mut error,
                )
            };
            typed_call_result("add pane object", status, error)?;
        }
        Ok(())
    }

    fn set_clipboard_text(&mut self, text: Option<&str>) -> Result<(), ScriptError> {
        let (pointer, length, available) = match text {
            Some(text) => (text.as_ptr().cast::<c_char>(), text.len(), 1),
            None => (std::ptr::null(), 0, 0),
        };
        let mut error = std::ptr::null_mut();
        // SAFETY: The optional string remains live for the call and length bounds the pointer.
        let status = unsafe {
            toyoterm_mruby_set_clipboard_text(
                self.state.as_ptr(),
                pointer,
                length,
                available,
                &mut error,
            )
        };
        typed_call_result("set clipboard text", status, error)
    }

    fn eval_with_filename(&mut self, source: &str, filename: &str) -> Result<String, ScriptError> {
        let source = CString::new(source)
            .map_err(|_| ScriptError::new("evaluate mruby", "source contains a NUL byte"))?;
        let filename = CString::new(filename)
            .map_err(|_| ScriptError::new("evaluate mruby", "filename contains a NUL byte"))?;
        let mut output = std::ptr::null_mut();
        // SAFETY: `state` is live, strings are NUL terminated, and the shim initializes `output`.
        let status = unsafe {
            toyoterm_mruby_eval(
                self.state.as_ptr(),
                source.as_ptr(),
                filename.as_ptr(),
                &mut output,
            )
        };
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

fn typed_call_result(
    operation: &'static str,
    status: i32,
    error: *mut c_char,
) -> Result<(), ScriptError> {
    let message = NonNull::new(error).map(|error| {
        // SAFETY: Error strings are NUL-terminated allocations owned by the shim.
        let message = unsafe { CStr::from_ptr(error.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: The allocation came from the shim and is freed exactly once.
        unsafe { toyoterm_mruby_string_free(error.as_ptr()) };
        message
    });
    match status {
        0 => Ok(()),
        1 => Err(ScriptError::new(
            operation,
            message.unwrap_or_else(|| "mruby call failed without an exception".to_owned()),
        )),
        _ => Err(ScriptError::new(
            operation,
            message.unwrap_or_else(|| "mruby typed call failed".to_owned()),
        )),
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
    native_actions: HashMap<String, NativeAction>,
    event_names: HashSet<String>,
    source_path: Option<PathBuf>,
}

struct LoadedConfig {
    runtime: MrubyRuntime,
    config: ToyotermConfig,
    keybindings: HashSet<String>,
    native_actions: HashMap<String, NativeAction>,
    event_names: HashSet<String>,
}

impl ConfigManager {
    pub fn new() -> Result<Self, ScriptError> {
        let loaded = load_config("", "(default config)")?;
        Ok(Self {
            runtime: loaded.runtime,
            config: loaded.config,
            keybindings: loaded.keybindings,
            native_actions: loaded.native_actions,
            event_names: loaded.event_names,
            source_path: None,
        })
    }

    pub fn config(&self) -> &ToyotermConfig {
        &self.config
    }

    pub fn load_startup(explicit_path: Option<&Path>) -> Result<Self, ScriptError> {
        let (manager, error) = Self::load_startup_recovering(explicit_path)?;
        match error {
            Some(error) => Err(error),
            None => Ok(manager),
        }
    }

    pub(crate) fn load_startup_recovering(
        explicit_path: Option<&Path>,
    ) -> Result<(Self, Option<ScriptError>), ScriptError> {
        let env_path = std::env::var_os("TOYOTERM_CONFIG_FILE").filter(|path| !path.is_empty());
        let home = home_directory();
        let mut manager = Self::new()?;
        let Some(path) = resolve_config_path(explicit_path, env_path.as_deref(), home.as_deref())
        else {
            return Ok((manager, None));
        };
        let required = explicit_path.is_some() || env_path.is_some();
        manager.source_path = Some(path.clone());
        if !required && !path.exists() {
            return Ok((manager, None));
        }
        let error = manager.reload_file().err();
        Ok((manager, error))
    }

    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }

    /// Reloads the selected config file, preserving the active VM on any failure.
    pub fn reload_file(&mut self) -> Result<&ToyotermConfig, ScriptError> {
        let path = self.source_path.clone().ok_or_else(|| {
            ScriptError::new("reload config", "no configuration path is available")
        })?;
        tracing::debug!(target: "toyoterm::config", path = %path.display(), "load config");
        let source = std::fs::read_to_string(&path)
            .map_err(|error| ScriptError::config_file(&path, error))?;
        self.reload_named(&source, &path.display().to_string())
            .map_err(|error| ScriptError::config_file(&path, error))
    }

    /// Evaluate config in a fresh VM and swap it in only after complete validation.
    pub fn reload(&mut self, source: &str) -> Result<&ToyotermConfig, ScriptError> {
        self.reload_named(source, "(config)")
    }

    fn reload_named(
        &mut self,
        source: &str,
        filename: &str,
    ) -> Result<&ToyotermConfig, ScriptError> {
        let loaded = load_config(source, filename)?;
        self.runtime = loaded.runtime;
        self.config = loaded.config;
        self.keybindings = loaded.keybindings;
        self.native_actions = loaded.native_actions;
        self.event_names = loaded.event_names;
        tracing::info!(target: "toyoterm::config", filename, "config loaded");
        Ok(&self.config)
    }

    pub fn eval(&mut self, source: &str) -> Result<String, ScriptError> {
        self.runtime.eval(source)
    }

    pub fn native_action(&self, key: &str) -> Option<NativeAction> {
        self.native_actions.get(&key.to_uppercase()).copied()
    }

    pub fn has_dynamic_keybinding(&self, key: &str) -> bool {
        self.keybindings.contains(&key.to_uppercase())
    }

    /// Updates the pane exposed by `Toyoterm.current_pane` for subsequent evaluations.
    pub fn set_current_pane(&mut self, pane: PaneId) -> Result<(), ScriptError> {
        self.runtime.set_current_pane(pane)
    }

    pub fn set_live_handles(
        &mut self,
        handles: impl IntoIterator<Item = NativeHandle>,
    ) -> Result<(), ScriptError> {
        let mut workspaces = Vec::new();
        let mut windows = Vec::new();
        let mut tabs = Vec::new();
        let mut panes = Vec::new();
        for handle in handles {
            match handle.kind() {
                HandleKind::Workspace => workspaces.push(handle.id()),
                HandleKind::Window => windows.push(handle.id()),
                HandleKind::Tab => tabs.push(handle.id()),
                HandleKind::Pane => panes.push(handle.id()),
            }
        }
        workspaces.sort_unstable();
        windows.sort_unstable();
        tabs.sort_unstable();
        panes.sort_unstable();
        self.runtime
            .set_live_handles(&workspaces, &windows, &tabs, &panes)
    }

    pub(crate) fn set_object_model(&mut self, model: &RubyObjectModel) -> Result<(), ScriptError> {
        self.runtime.set_object_model(model)
    }

    /// Updates the clipboard snapshot exposed to the next Ruby callback.
    pub fn set_clipboard_text(&mut self, text: Option<&str>) -> Result<(), ScriptError> {
        self.runtime.set_clipboard_text(text)
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
        let callback_name = key;
        let key = ruby_string_literal(&callback_name);
        let source = format!("Toyoterm.__config.__trigger_binding({key}, Toyoterm.current_pane)");
        match self.eval_callback(CallbackKind::KeyBinding, &callback_name, &source)? {
            value if value == "true" => Ok(true),
            value if value == "false" => Ok(false),
            _ => Err(ScriptError::new(
                "evaluate key binding",
                "callback returned an invalid match state",
            )),
        }
    }

    /// Emits an event only when Ruby registered at least one handler for it.
    pub fn emit_event(&mut self, name: &str, current_pane: PaneId) -> Result<bool, ScriptError> {
        if !self.event_names.contains(name) {
            return Ok(false);
        }
        self.set_current_pane(current_pane)?;
        let callback_name = name;
        let name = ruby_string_literal(callback_name);
        let source = format!("Toyoterm.__emit_event({name}, Toyoterm.current_pane)");
        match self.eval_callback(CallbackKind::Event, callback_name, &source)? {
            value if value == "true" => Ok(true),
            value if value == "false" => Ok(false),
            _ => Err(ScriptError::new(
                "emit mruby event",
                "event handler returned an invalid state",
            )),
        }
    }

    fn eval_callback(
        &mut self,
        kind: CallbackKind,
        name: &str,
        source: &str,
    ) -> Result<String, ScriptError> {
        let started = Instant::now();
        let result = self.runtime.eval(source);
        record_callback_duration(kind, name, started.elapsed(), result.is_ok());
        result
    }

    /// Converts commands queued by Ruby into the native command API.
    ///
    /// Pane id zero is a bootstrap placeholder used while startup config is loading.
    #[cfg(test)]
    pub(crate) fn drain_commands(
        &mut self,
        current_pane: PaneId,
    ) -> Result<Vec<NativeCommand>, ScriptError> {
        self.drain_commands_with_context(WorkspaceId(0), WindowId(0), TabId(0), current_pane)
    }

    pub(crate) fn drain_commands_with_context(
        &mut self,
        current_workspace: WorkspaceId,
        current_window: WindowId,
        current_tab: TabId,
        current_pane: PaneId,
    ) -> Result<Vec<NativeCommand>, ScriptError> {
        let mut commands = Vec::new();
        loop {
            let command_type = self.runtime.eval("Toyoterm.__next_command")?;
            if command_type.is_empty() {
                break;
            }

            let raw_id = self
                .runtime
                .eval("Toyoterm.__current_command_pane")?
                .parse::<u64>()
                .map_err(|_| ScriptError::new("decode mruby command", "handle id is invalid"))?;
            let pane = if raw_id == 0 {
                current_pane
            } else {
                PaneId(raw_id)
            };
            let payload = self.runtime.eval("Toyoterm.__current_command_payload")?;
            match command_type.as_str() {
                "send_text" => commands.push(NativeCommand::Mux(Command::SendText {
                    pane,
                    text: payload,
                })),
                "split" => commands.push(NativeCommand::Mux(Command::Split {
                    pane,
                    direction: parse_direction(&payload)?,
                })),
                "close_pane" => commands.push(NativeCommand::Mux(Command::ClosePane(pane))),
                "activate_pane" => commands.push(NativeCommand::Mux(Command::ActivatePane(pane))),
                "close_tab" => commands.push(NativeCommand::Mux(Command::CloseTab(TabId(
                    resolve_bootstrap_id(raw_id, current_tab.0),
                )))),
                "activate_tab" => commands.push(NativeCommand::Mux(Command::ActivateTab(TabId(
                    resolve_bootstrap_id(raw_id, current_tab.0),
                )))),
                "new_tab" => commands.push(NativeCommand::Mux(Command::NewTabIn(WindowId(
                    resolve_bootstrap_id(raw_id, current_window.0),
                )))),
                "close_window" => commands.push(NativeCommand::Mux(Command::CloseWindow(
                    WindowId(resolve_bootstrap_id(raw_id, current_window.0)),
                ))),
                "activate_window" => commands.push(NativeCommand::Mux(Command::ActivateWindow(
                    WindowId(resolve_bootstrap_id(raw_id, current_window.0)),
                ))),
                "activate_workspace" => {
                    commands.push(NativeCommand::Mux(Command::ActivateWorkspace(WorkspaceId(
                        resolve_bootstrap_id(raw_id, current_workspace.0),
                    ))))
                }
                "create_window" => commands.push(NativeCommand::Mux(Command::CreateWindow(
                    WorkspaceId(resolve_bootstrap_id(raw_id, current_workspace.0)),
                ))),
                "clipboard_write" => commands.push(NativeCommand::ClipboardWrite(payload)),
                "reload_config" => commands.push(NativeCommand::ReloadConfig),
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

const fn resolve_bootstrap_id(id: u64, current: u64) -> u64 {
    if id == 0 { current } else { id }
}

fn record_callback_duration(kind: CallbackKind, name: &str, elapsed: Duration, succeeded: bool) {
    let duration_ms = elapsed.as_secs_f64() * 1_000.0;
    if is_slow_callback(elapsed) {
        tracing::warn!(
            target: "toyoterm::script",
            callback_kind = kind.as_str(),
            callback_name = name,
            duration_ms,
            threshold_ms = SLOW_CALLBACK_THRESHOLD.as_millis() as u64,
            succeeded,
            "slow Ruby callback"
        );
    } else {
        tracing::debug!(
            target: "toyoterm::script",
            callback_kind = kind.as_str(),
            callback_name = name,
            duration_ms,
            succeeded,
            "Ruby callback completed"
        );
    }
}

fn is_slow_callback(elapsed: Duration) -> bool {
    elapsed >= SLOW_CALLBACK_THRESHOLD
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

fn load_config(source: &str, filename: &str) -> Result<LoadedConfig, ScriptError> {
    let mut runtime = MrubyRuntime::new()?;
    let config_dsl =
        CONFIG_DSL.replace("__TOYOTERM_PRIMARY_MODIFIER__", platform_primary_modifier());
    runtime.eval_with_filename(&config_dsl, "(toyoterm DSL)")?;
    runtime.eval_with_filename(source, filename)?;

    let defaults = ToyotermConfig::default();
    let family = runtime.eval("Toyoterm.__config.font.family")?;
    if family.trim().is_empty() {
        return Err(ScriptError::new(
            "validate config",
            "font family cannot be empty",
        ));
    }
    let fallback_count = runtime
        .eval("Toyoterm.__config.font.__fallback_count")?
        .parse::<usize>()
        .map_err(|_| {
            ScriptError::new("validate config", "font fallback count must be an integer")
        })?;
    if fallback_count > 32 {
        return Err(ScriptError::new(
            "validate config",
            "font fallback supports at most 32 families",
        ));
    }
    let mut fallback = Vec::with_capacity(fallback_count);
    for index in 0..fallback_count {
        let fallback_family =
            runtime.eval(&format!("Toyoterm.__config.font.__fallback_at({index})"))?;
        if fallback_family.trim().is_empty() {
            return Err(ScriptError::new(
                "validate config",
                "font fallback entries cannot be empty",
            ));
        }
        if fallback_family == family || fallback.contains(&fallback_family) {
            return Err(ScriptError::new(
                "validate config",
                format!("duplicate font family in fallback: {fallback_family}"),
            ));
        }
        fallback.push(fallback_family);
    }
    let font_size = parse_positive_f32("font size", &runtime.eval("Toyoterm.__config.font.size")?)?;
    let font_weight = runtime
        .eval("Toyoterm.__config.font.weight")?
        .parse::<u16>()
        .map_err(|_| ScriptError::new("validate config", "font weight must be an integer"))?;
    if !(1..=1000).contains(&font_weight) {
        return Err(ScriptError::new(
            "validate config",
            "font weight must be between 1 and 1000",
        ));
    }
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
    let leader_key = runtime.eval("Toyoterm.__config.__leader_key")?;
    let leader = if leader_key.is_empty() {
        None
    } else {
        let timeout_ms = runtime
            .eval("Toyoterm.__config.__leader_timeout")?
            .parse::<u64>()
            .map_err(|_| {
                ScriptError::new("validate config", "leader timeout must be an integer")
            })?;
        if timeout_ms == 0 {
            return Err(ScriptError::new(
                "validate config",
                "leader timeout must be positive",
            ));
        }
        Some(LeaderConfig {
            key: leader_key,
            timeout_ms,
        })
    };

    let config = ToyotermConfig {
        font: FontConfig {
            family,
            fallback,
            size: font_size,
            weight: font_weight,
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
        leader,
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

    let static_count = runtime
        .eval("Toyoterm.__config.__static_binding_count")?
        .parse::<usize>()
        .map_err(|_| ScriptError::new("load key bindings", "static binding count is invalid"))?;
    let mut native_actions = HashMap::with_capacity(static_count);
    for index in 0..static_count {
        let key = runtime.eval(&format!("Toyoterm.__config.__static_binding_key({index})"))?;
        let action = runtime.eval(&format!(
            "Toyoterm.__config.__static_binding_action({index})"
        ))?;
        let argument = runtime.eval(&format!(
            "Toyoterm.__config.__static_binding_argument({index})"
        ))?;
        native_actions.insert(key, decode_native_action(&action, &argument)?);
    }

    let event_count = runtime
        .eval("Toyoterm.__event_count")?
        .parse::<usize>()
        .map_err(|_| ScriptError::new("load events", "event count is invalid"))?;
    let mut event_names = HashSet::with_capacity(event_count);
    for index in 0..event_count {
        event_names.insert(runtime.eval(&format!("Toyoterm.__event_name({index})"))?);
    }

    Ok(LoadedConfig {
        runtime,
        config,
        keybindings,
        native_actions,
        event_names,
    })
}

fn platform_primary_modifier() -> &'static str {
    if cfg!(target_os = "macos") {
        "SUPER"
    } else {
        "CTRL"
    }
}

fn decode_native_action(action: &str, argument: &str) -> Result<NativeAction, ScriptError> {
    match action {
        "new_tab" => Ok(NativeAction::NewTab),
        "close_pane" => Ok(NativeAction::ClosePane),
        "reload_config" => Ok(NativeAction::ReloadConfig),
        "split" => parse_direction(argument).map(NativeAction::Split),
        "activate_pane" => parse_direction(argument).map(NativeAction::ActivatePane),
        other => Err(ScriptError::new(
            "load key bindings",
            format!("unsupported native action {other}"),
        )),
    }
}

fn parse_direction(direction: &str) -> Result<SplitDirection, ScriptError> {
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

fn ruby_string_literal(value: &str) -> String {
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
    fn classifies_callbacks_at_the_slow_threshold() {
        assert!(!is_slow_callback(Duration::from_millis(99)));
        assert!(is_slow_callback(Duration::from_millis(100)));
    }

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
                    font.fallback = ["Noto Sans Mono CJK JP", "Noto Color Emoji"]
                    font.size = 16
                    font.weight = 500
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
        assert_eq!(
            config.font.fallback,
            ["Noto Sans Mono CJK JP", "Noto Color Emoji"]
        );
        assert_eq!(config.font.size, 16.0);
        assert_eq!(config.font.weight, 500);
        assert_eq!(config.colors.background, "#111111");
        assert_eq!(config.window_opacity, 0.92);
        assert_eq!(config.default_shell.as_deref(), Some("/bin/zsh"));
        assert_eq!(config.scrollback_lines, 50_000);
    }

    #[test]
    fn rejects_font_weight_outside_css_range() {
        let mut manager = ConfigManager::new().unwrap();
        let error = manager
            .reload("Toyoterm.configure { |config| config.font.weight = 1001 }")
            .unwrap_err();
        assert!(
            error
                .message()
                .contains("font weight must be between 1 and 1000")
        );
    }

    #[test]
    fn rejects_invalid_font_fallbacks() {
        let mut manager = ConfigManager::new().unwrap();
        let error = manager
            .reload("Toyoterm.configure { |config| config.font.fallback = 'emoji' }")
            .unwrap_err();
        assert!(error.message().contains("font fallback must be an array"));

        let error = manager
            .reload("Toyoterm.configure { |config| config.font.fallback = [''] }")
            .unwrap_err();
        assert!(error.message().contains("cannot be empty"));

        let error = manager
            .reload("Toyoterm.configure { |config| config.font.fallback = ['Noto', 'Noto'] }")
            .unwrap_err();
        assert!(error.message().contains("duplicate font family"));
    }

    #[test]
    fn bundled_minimal_configuration_is_executable() {
        let mut manager = ConfigManager::new().unwrap();
        manager
            .reload(include_str!("../examples/minimal_config.rb"))
            .unwrap();
        assert_eq!(manager.config().font.size, 14.0);
        assert_eq!(manager.config().scrollback_lines, 10_000);
        assert!(
            manager
                .trigger_keybinding("CTRL+SHIFT+H", PaneId(7))
                .unwrap()
        );
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
    fn reports_config_filename_line_and_ruby_backtrace() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "toyoterm-broken-config-{}-{unique}.rb",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"def fail_config
  raise "broken config"
end
fail_config
"#,
        )
        .unwrap();

        let error = match ConfigManager::load_startup(Some(&path)) {
            Ok(_) => panic!("broken config unexpectedly loaded"),
            Err(error) => error,
        };
        std::fs::remove_file(&path).unwrap();
        let message = error.to_string();

        assert!(message.contains(&path.display().to_string()), "{message}");
        assert!(message.contains(":2"), "{message}");
        assert!(message.contains(":4"), "{message}");
        assert!(message.contains("broken config"), "{message}");
    }

    #[test]
    fn gui_startup_recovers_with_defaults_and_keeps_the_broken_path() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "toyoterm-gui-config-{}-{unique}.rb",
            std::process::id()
        ));
        std::fs::write(&path, "raise 'broken GUI config'").unwrap();

        let (manager, error) = ConfigManager::load_startup_recovering(Some(&path)).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(manager.source_path(), Some(path.as_path()));
        assert_eq!(manager.config(), &ToyotermConfig::default());
        assert!(
            error
                .expect("broken config should be reported")
                .message()
                .contains("broken GUI config")
        );
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
            vec![NativeCommand::Mux(Command::SendText {
                pane: PaneId(42),
                text: "echo hello\n".into(),
            })]
        );
        assert!(manager.drain_commands(PaneId(42)).unwrap().is_empty());
    }

    #[test]
    fn exposes_the_synced_ruby_object_model() {
        let mut manager = ConfigManager::new().unwrap();
        manager
            .set_live_handles([
                NativeHandle::from(WorkspaceId(10)),
                NativeHandle::from(WindowId(20)),
                NativeHandle::from(TabId(30)),
                NativeHandle::from(PaneId(40)),
            ])
            .unwrap();
        manager
            .set_object_model(&RubyObjectModel {
                current_workspace: WorkspaceId(10),
                current_window: WindowId(20),
                current_tab: TabId(30),
                current_pane: PaneId(40),
                workspaces: vec![RubyWorkspace {
                    id: WorkspaceId(10),
                    name: "backend".into(),
                    windows: vec![WindowId(20)],
                }],
                windows: vec![RubyWindow {
                    id: WindowId(20),
                    tabs: vec![TabId(30)],
                }],
                tabs: vec![RubyTab {
                    id: TabId(30),
                    title: "server".into(),
                    panes: vec![PaneId(40)],
                }],
                panes: vec![RubyPane {
                    id: PaneId(40),
                    title: "shell".into(),
                    cwd: Some("/srv/app".into()),
                    pid: Some(1234),
                }],
            })
            .unwrap();

        assert_eq!(
            manager.eval("Toyoterm.current_workspace.name").unwrap(),
            "backend"
        );
        assert_eq!(
            manager.eval("Toyoterm.workspace('backend').id").unwrap(),
            "10"
        );
        assert_eq!(
            manager.eval("Toyoterm.workspaces.map(&:id)").unwrap(),
            "[10]"
        );
        assert_eq!(manager.eval("Toyoterm.windows.map(&:id)").unwrap(), "[20]");
        assert_eq!(
            manager.eval("Toyoterm.current_window.tabs[0].id").unwrap(),
            "30"
        );
        assert_eq!(
            manager.eval("Toyoterm.current_tab.title").unwrap(),
            "server"
        );
        assert_eq!(
            manager.eval("Toyoterm.current_tab.panes[0].title").unwrap(),
            "shell"
        );
        assert_eq!(
            manager.eval("Toyoterm.current_pane.cwd").unwrap(),
            "/srv/app"
        );
        assert_eq!(manager.eval("Toyoterm.current_pane.pid").unwrap(), "1234");
        assert_eq!(manager.eval("Toyoterm.workspace('missing')").unwrap(), "");

        manager.eval("Toyoterm.current_pane.badge = 'dev'").unwrap();
        assert_eq!(manager.eval("Toyoterm.current_pane.badge").unwrap(), "dev");
    }

    #[test]
    fn converts_object_model_operations_to_native_commands() {
        let mut manager = ConfigManager::new().unwrap();
        manager
            .eval(
                "Toyoterm.current_pane.split(:left); Toyoterm.current_pane.focus; \
                 Toyoterm.current_tab.close; Toyoterm.current_window.new_tab; \
                 Toyoterm.current_window.close; Toyoterm.current_workspace.activate; \
                 Toyoterm.current_workspace.create_window",
            )
            .unwrap();

        assert_eq!(
            manager
                .drain_commands_with_context(WorkspaceId(10), WindowId(20), TabId(30), PaneId(40),)
                .unwrap(),
            vec![
                NativeCommand::Mux(Command::Split {
                    pane: PaneId(40),
                    direction: SplitDirection::Left,
                }),
                NativeCommand::Mux(Command::ActivatePane(PaneId(40))),
                NativeCommand::Mux(Command::CloseTab(TabId(30))),
                NativeCommand::Mux(Command::NewTabIn(WindowId(20))),
                NativeCommand::Mux(Command::CloseWindow(WindowId(20))),
                NativeCommand::Mux(Command::ActivateWorkspace(WorkspaceId(10))),
                NativeCommand::Mux(Command::CreateWindow(WorkspaceId(10))),
            ]
        );
    }

    #[test]
    fn ruby_native_handles_are_typed_id_values() {
        let mut manager = ConfigManager::new().unwrap();
        manager.set_current_pane(PaneId(42)).unwrap();

        assert_eq!(
            manager
                .eval("Toyoterm.current_pane.class.superclass")
                .unwrap(),
            "Toyoterm::NativeHandle"
        );
        assert_eq!(manager.eval("Toyoterm.current_pane.id").unwrap(), "42");
        assert_eq!(
            manager.eval("Toyoterm.current_pane.inspect").unwrap(),
            "#<Toyoterm::Pane:42>"
        );
        assert_eq!(
            manager
                .eval("Toyoterm::Pane.new(7) == Toyoterm::Pane.new(7)")
                .unwrap(),
            "true"
        );
        assert_eq!(
            manager
                .eval("Toyoterm::Pane.new(7) == Toyoterm::Tab.new(7)")
                .unwrap(),
            "false"
        );

        let error = manager.eval("Toyoterm::Pane.new(-1)").unwrap_err();
        assert!(error.message().contains("non-negative integer"));
    }

    #[test]
    fn deleted_ruby_handles_raise_a_typed_exception() {
        let mut manager = ConfigManager::new().unwrap();
        manager
            .set_live_handles([
                NativeHandle::new(HandleKind::Workspace, 1),
                NativeHandle::new(HandleKind::Pane, 7),
            ])
            .unwrap();
        manager.set_current_pane(PaneId(7)).unwrap();
        manager.eval("$saved_pane = Toyoterm.current_pane").unwrap();
        assert_eq!(manager.eval("$saved_pane.valid?").unwrap(), "true");

        manager
            .set_live_handles([NativeHandle::new(HandleKind::Workspace, 1)])
            .unwrap();
        assert_eq!(manager.eval("$saved_pane.valid?").unwrap(), "false");
        let error = manager.eval("$saved_pane.send_text('stale')").unwrap_err();
        assert!(error.message().contains("Toyoterm::InvalidHandleError"));
        assert!(error.message().contains("invalid pane handle 7"));
        assert!(manager.drain_commands(PaneId(9)).unwrap().is_empty());
    }

    #[test]
    fn exposes_clipboard_read_and_write_to_ruby() {
        let mut manager = ConfigManager::new().unwrap();
        manager
            .set_clipboard_text(Some("こんにちは\nclipboard"))
            .unwrap();

        assert_eq!(
            manager.eval("Toyoterm.clipboard.read").unwrap(),
            "こんにちは\nclipboard"
        );
        manager
            .eval(r#"Toyoterm.clipboard.write("copied from Ruby")"#)
            .unwrap();
        assert_eq!(
            manager.drain_commands(PaneId(42)).unwrap(),
            vec![NativeCommand::ClipboardWrite("copied from Ruby".into())]
        );
    }

    #[test]
    fn clipboard_text_cannot_interpolate_ruby_source() {
        let mut manager = ConfigManager::new().unwrap();
        let text = r#"#{raise "clipboard interpolation ran"}"#;

        manager.set_clipboard_text(Some(text)).unwrap();

        assert_eq!(manager.eval("Toyoterm.clipboard.read").unwrap(), text);
    }

    #[test]
    fn typed_clipboard_transfer_preserves_embedded_nul_bytes() {
        let mut manager = ConfigManager::new().unwrap();
        manager.set_clipboard_text(Some("left\0right")).unwrap();

        assert_eq!(
            manager
                .eval("Toyoterm.clipboard.read.bytes.join(',')")
                .unwrap(),
            "108,101,102,116,0,114,105,103,104,116"
        );
    }

    #[test]
    fn typed_mruby_calls_preserve_ruby_exceptions() {
        let mut manager = ConfigManager::new().unwrap();
        manager
            .eval(
                "def Toyoterm.__set_current_pane(id); raise ArgumentError, \"bad pane #{id}\"; end",
            )
            .unwrap();

        let error = manager.set_current_pane(PaneId(23)).unwrap_err();
        assert_eq!(error.operation(), "set current pane");
        assert!(error.message().contains("bad pane 23"));
    }

    #[test]
    fn reports_an_unavailable_clipboard_to_ruby() {
        let mut manager = ConfigManager::new().unwrap();
        let error = manager.eval("Toyoterm.clipboard.read").unwrap_err();
        assert!(error.message().contains("clipboard is unavailable"));
    }

    #[test]
    fn ruby_callback_errors_roll_back_clipboard_writes() {
        let mut manager = ConfigManager::new().unwrap();
        manager
            .reload(
                r#"
                Toyoterm.configure do |config|
                  config.bind "CTRL+C" do
                    Toyoterm.clipboard.write("must not be copied")
                    raise "broken clipboard callback"
                  end
                end
                "#,
            )
            .unwrap();

        let error = manager.trigger_keybinding("CTRL+C", PaneId(4)).unwrap_err();
        assert!(error.message().contains("broken clipboard callback"));
        assert!(manager.drain_commands(PaneId(4)).unwrap().is_empty());
    }

    #[test]
    fn resolves_startup_commands_to_the_current_native_pane() {
        let mut manager = ConfigManager::new().unwrap();
        manager
            .reload(r#"Toyoterm.current_pane.send_text("pwd\n")"#)
            .unwrap();

        assert_eq!(
            manager.drain_commands(PaneId(7)).unwrap(),
            vec![NativeCommand::Mux(Command::SendText {
                pane: PaneId(7),
                text: "pwd\n".into(),
            })]
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
            vec![NativeCommand::Mux(Command::SendText {
                pane: PaneId(9),
                text: "echo from ruby\n".into(),
            })]
        );
    }

    #[test]
    fn compiles_static_key_dsl_to_native_actions() {
        let mut manager = ConfigManager::new().unwrap();
        manager
            .reload(
                r#"
                Toyoterm.configure do |config|
                  config.keys do
                    ctrl_shift("v").split(:right)
                    ctrl_shift("j").activate_pane(:down)
                    ctrl("t").new_tab
                    alt("q").close_pane
                    primary("p").close_pane
                    primary_shift("o").reload_config
                    ctrl_shift("r").reload_config
                    physical("KeyH", "CTRL").activate_pane(:left)
                  end
                end
                "#,
            )
            .unwrap();

        assert_eq!(
            manager.native_action("CTRL+SHIFT+V"),
            Some(NativeAction::Split(SplitDirection::Right))
        );
        assert_eq!(
            manager.native_action("CTRL+SHIFT+J"),
            Some(NativeAction::ActivatePane(SplitDirection::Down))
        );
        assert_eq!(
            manager.native_action("CTRL+PHYSICAL:KEYH"),
            Some(NativeAction::ActivatePane(SplitDirection::Left))
        );
        assert_eq!(manager.native_action("CTRL+T"), Some(NativeAction::NewTab));
        assert_eq!(
            manager.native_action("ALT+Q"),
            Some(NativeAction::ClosePane)
        );
        assert_eq!(
            manager.native_action("CTRL+SHIFT+R"),
            Some(NativeAction::ReloadConfig)
        );
        assert_eq!(
            manager.native_action(&format!("{}+P", platform_primary_modifier())),
            Some(NativeAction::ClosePane)
        );
        assert_eq!(
            manager.native_action(if cfg!(target_os = "macos") {
                "SHIFT+SUPER+O"
            } else {
                "CTRL+SHIFT+O"
            }),
            Some(NativeAction::ReloadConfig)
        );
    }

    #[test]
    fn loads_leader_configuration_and_compiles_leader_actions() {
        let mut manager = ConfigManager::new().unwrap();
        let config = manager
            .reload(
                r#"
                Toyoterm.configure do |config|
                  config.leader key: "b", mods: "CTRL", timeout: 750
                  config.keys do
                    leader("v").split(:right)
                    leader("t").new_tab
                  end
                end
                "#,
            )
            .unwrap();

        assert_eq!(
            config.leader,
            Some(LeaderConfig {
                key: "CTRL+B".into(),
                timeout_ms: 750,
            })
        );
        assert_eq!(
            manager.native_action("LEADER+V"),
            Some(NativeAction::Split(SplitDirection::Right))
        );
        assert_eq!(
            manager.native_action("LEADER+T"),
            Some(NativeAction::NewTab)
        );
    }

    #[test]
    fn rejects_invalid_leader_timeout() {
        let mut manager = ConfigManager::new().unwrap();
        let error = manager
            .reload("Toyoterm.configure { |config| config.leader key: 'b', timeout: 0 }")
            .unwrap_err();
        assert!(error.message().contains("leader timeout must be positive"));
    }

    #[test]
    fn rejects_duplicate_static_and_dynamic_bindings() {
        let mut manager = ConfigManager::new().unwrap();
        let error = manager
            .reload(
                r#"
                Toyoterm.configure do |config|
                  config.keys { ctrl("x").new_tab }
                  config.bind("CTRL+X") { }
                end
                "#,
            )
            .unwrap_err();
        assert!(error.message().contains("duplicate key binding"));
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
        assert_eq!(
            manager.drain_commands(PaneId(4)).unwrap(),
            vec![NativeCommand::ReloadConfig]
        );
        assert!(manager.drain_commands(PaneId(4)).unwrap().is_empty());
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
            vec![NativeCommand::Mux(Command::SendText {
                pane: PaneId(12),
                text: "echo app started\n".into(),
            })]
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
