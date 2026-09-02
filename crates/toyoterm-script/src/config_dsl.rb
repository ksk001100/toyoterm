
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
    attr_accessor :background, :foreground, :cursor, :selection, :ansi

    def initialize
      @background = "#090b0e"
      @foreground = "#dce1e8"
      @cursor = "#f5f7fa"
      @selection = "#375891"
      @ansi = [
        "#000000", "#cd0000", "#00cd00", "#cdcd00",
        "#0000ee", "#cd00cd", "#00cdcd", "#e5e5e5",
        "#7f7f7f", "#ff0000", "#00ff00", "#ffff00",
        "#5c5cff", "#ff00ff", "#00ffff", "#ffffff"
      ]
    end

    def __ansi_count
      raise TypeError, "colors.ansi must be an array" unless @ansi.is_a?(Array)
      @ansi.length
    end

    def __ansi_at(index)
      color = @ansi[index]
      raise TypeError, "colors.ansi entries must be strings" unless color.is_a?(String)
      color
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

  CommandContext = KeyBindingContext

  class StatusContext
    attr_reader :workspace, :window, :tab, :pane

    def initialize(workspace, window, tab, pane)
      @workspace = workspace
      @window = window
      @tab = tab
      @pane = pane
    end
  end

  class Event
    attr_reader :name, :workspace, :window, :tab, :pane, :title, :cwd

    def initialize(name, workspace = nil, window = nil, tab = nil, pane = nil, title = nil, cwd = nil)
      @name = name
      @workspace = workspace
      @window = window
      @tab = tab
      @pane = pane
      @title = title
      @cwd = cwd
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

    def command_palette
      @config.__register_static(@key, :command_palette, nil)
      self
    end

    def maximize_window
      @config.__register_static(@key, :maximize_window, nil)
      self
    end

    def toggle_maximize
      @config.__register_static(@key, :toggle_maximize, nil)
      self
    end

    def minimize_window
      @config.__register_static(@key, :minimize_window, nil)
      self
    end

    def toggle_fullscreen
      @config.__register_static(@key, :toggle_fullscreen, nil)
      self
    end

    def command(name)
      @config.__register_static(@key, :user_command, name)
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

    def __checkpoint
      [
        [@font.family, @font.fallback.dup, @font.size, @font.weight],
        [@colors.background, @colors.foreground, @colors.cursor, @colors.selection,
         @colors.ansi.dup],
        [@window.opacity, @default_shell, @scrollback_lines],
        [@leader_key, @leader_timeout]
      ]
    end

    def __restore(checkpoint)
      font, colors, window, leader = checkpoint
      @font.family = font[0]
      @font.fallback = font[1]
      @font.size = font[2]
      @font.weight = font[3]
      @colors.background = colors[0]
      @colors.foreground = colors[1]
      @colors.cursor = colors[2]
      @colors.selection = colors[3]
      @colors.ansi = colors[4]
      @window.opacity = window[0]
      @default_shell = window[1]
      @scrollback_lines = window[2]
      @leader_key = leader[0]
      @leader_timeout = leader[1]
      nil
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

    def __plugin_checkpoint
      [@bindings.dup, @static_bindings.dup]
    end

    def __rollback_plugin(checkpoint)
      @bindings = checkpoint[0]
      @static_bindings = checkpoint[1]
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

    def command_running?
      validate!
      Toyoterm.__object_data(:pane, @id)[3]
    end

    def last_exit_status
      validate!
      Toyoterm.__object_data(:pane, @id)[4]
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

  class ProcessResult
    attr_reader :stdout, :stderr, :exit_status

    def initialize(stdout, stderr, exit_status)
      @stdout = stdout
      @stderr = stderr
      @exit_status = exit_status
    end

    def success?
      @exit_status == 0
    end
  end

  class Plugin
    class Definition
      attr_reader :name
      attr_accessor :version, :requires

      def initialize(name)
        @name = name.to_s
        @version = nil
        @requires = nil
      end

      def command(name, &block)
        Toyoterm.command(name, &block)
      end

      def on(name, &block)
        Toyoterm.on(name, &block)
      end

      def bind(key, &block)
        Toyoterm.__config.bind(key, &block)
      end

      def keys(&block)
        Toyoterm.__config.keys(&block)
      end

      def __validate!
        raise ArgumentError, "plugin name cannot be empty" if @name.empty?
        raise ArgumentError, "plugin version is required" if @version.nil? || @version.to_s.empty?
        @version = @version.to_s
        @requires = @requires.nil? ? "" : @requires.to_s
      end
    end

    def self.define(name)
      raise RuntimeError, "Plugin.define can only be used while loading a plugin" unless Toyoterm.__loading_plugin?
      raise ArgumentError, "plugin definition requires a block" unless block_given?
      definition = Definition.new(name)
      yield definition
      definition.__validate!
      Toyoterm.__register_plugin(definition)
      definition
    end
  end

  @config = Config.new
  @current_pane = Pane.new(0)
  @current_tab = Tab.new(0)
  @current_window = Window.new(0)
  @current_workspace = Workspace.new(0)
  @clipboard = Clipboard.new
  @env = {}
  @commands = []
  @current_command = nil
  @event_handlers = {}
  @user_commands = {}
  @status_callback = nil
  @status_interval = nil
  @live_handles = {
    workspace: [0],
    window: [0],
    tab: [0],
    pane: [0]
  }
  @object_data = { workspace: {}, window: {}, tab: {}, pane: {} }
  @pane_badges = {}
  @plugins = []
  @plugin_requests = []
  @current_plugin_path = nil

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

  # Returns a snapshot. Mutating it never changes the host process environment.
  def self.env
    @env.dup
  end

  def self.read_file(path)
    path = path.to_s
    raise ArgumentError, "path contains a NUL byte" if path.index("\0")
    __host_read_file(path)
  end

  def self.spawn(program, *args)
    program = program.to_s
    raise ArgumentError, "program cannot be empty" if program.empty?
    values = [program] + args.map { |arg| arg.to_s }
    if values.any? { |value| value.index("\0") }
      raise ArgumentError, "program and arguments cannot contain NUL bytes"
    end
    ProcessResult.new(*__host_spawn(values))
  end

  def self.plugin(path)
    path = path.to_s
    raise ArgumentError, "plugin path cannot be empty" if path.empty?
    raise ArgumentError, "plugin path contains a NUL byte" if path.index("\0")
    @plugin_requests << [path, @current_plugin_path]
    nil
  end

  def self.plugins
    @plugins.dup
  end

  def self.__loading_plugin?
    !@current_plugin_path.nil?
  end

  def self.__begin_plugin(path)
    @current_plugin_path = path
  end

  def self.__end_plugin
    @current_plugin_path = nil
  end

  def self.__register_plugin(plugin)
    if @plugins.any? { |loaded| loaded.name == plugin.name }
      raise ArgumentError, "duplicate plugin name: #{plugin.name}"
    end
    @plugins << plugin
  end

  def self.__plugin_checkpoint
    event_handlers = {}
    @event_handlers.each { |name, handlers| event_handlers[name] = handlers.dup }
    [
      @plugins.dup,
      @user_commands.dup,
      event_handlers,
      @config.__plugin_checkpoint,
      @plugin_requests.length
    ]
  end

  def self.__rollback_plugin(checkpoint)
    @plugins = checkpoint[0]
    @user_commands = checkpoint[1]
    @event_handlers = checkpoint[2]
    @config.__rollback_plugin(checkpoint[3])
    @plugin_requests.pop while @plugin_requests.length > checkpoint[4]
  end

  def self.__plugin_request_count
    @plugin_requests.length
  end

  def self.__plugin_request_path(index)
    @plugin_requests[index][0]
  end

  def self.__plugin_request_parent(index)
    @plugin_requests[index][1] || ""
  end

  def self.__discard_plugin_requests(count)
    @plugin_requests.shift(count)
  end

  def self.__plugin_count
    @plugins.length
  end

  def self.__plugin_name(index)
    @plugins[index].name
  end

  def self.__plugin_version(index)
    @plugins[index].version
  end

  def self.__plugin_requires(index)
    @plugins[index].requires
  end

  def self.__replace_env(entries)
    @env = {}
    index = 0
    while index < entries.length
      @env[entries[index]] = entries[index + 1]
      index += 2
    end
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

  # The host validates mutations made in the persistent VM after each
  # request. Keep a VM-side checkpoint so invalid changes can be rolled back.
  def self.__begin_config_transaction
    @config_transaction = [@config.__checkpoint, @status_interval, @status_callback]
    nil
  end

  def self.__rollback_config_transaction
    checkpoint = @config_transaction
    return nil unless checkpoint
    @config.__restore(checkpoint[0])
    @status_interval = checkpoint[1]
    @status_callback = checkpoint[2]
    @config_transaction = nil
    nil
  end

  def self.__commit_config_transaction
    @config_transaction = nil
    nil
  end

  def self.on(name, &block)
    raise ArgumentError, "event handler requires a block" unless block
    name = name.to_s
    raise ArgumentError, "event name cannot be empty" if name.empty?
    (@event_handlers[name] ||= []) << block
    block
  end

  def self.command(name, &block)
    raise ArgumentError, "user command requires a block" unless block
    name = name.to_s
    raise ArgumentError, "user command name cannot be empty" if name.empty?
    raise ArgumentError, "duplicate user command: #{name}" if @user_commands.key?(name)
    @user_commands[name] = block
    block
  end

  def self.status(interval: 1.0, &block)
    raise ArgumentError, "status callback requires a block" unless block
    raise ArgumentError, "status callback is already configured" if @status_callback
    @status_interval = interval
    @status_callback = block
    block
  end

  def self.__status_interval
    unless @status_interval.nil? || @status_interval.is_a?(Numeric)
      raise TypeError, "status interval must be numeric"
    end
    @status_interval
  end

  def self.__invoke_status
    return nil unless @status_callback
    context = StatusContext.new(current_workspace, current_window, current_tab, current_pane)
    checkpoint = __command_checkpoint
    begin
      @status_callback.call(context).to_s
    ensure
      __rollback_commands(checkpoint)
    end
  end

  def self.__command_count
    @user_commands.length
  end

  def self.__command_name(index)
    @user_commands.keys[index]
  end

  def self.__invoke_command(name, pane)
    callback = @user_commands[name.to_s]
    raise ArgumentError, "undefined user command: #{name}" unless callback
    checkpoint = __command_checkpoint
    begin
      callback.call(CommandContext.new(pane))
    rescue => error
      __rollback_commands(checkpoint)
      raise error
    end
    true
  end

  def self.__event_count
    @event_handlers.length
  end

  def self.__event_name(index)
    @event_handlers.keys[index]
  end

  def self.__emit_event(name, pane)
    __dispatch_event(name, Event.new(name.to_sym, nil, nil, nil, pane))
  end

  def self.__emit_native_event(name, workspace_id, window_id, tab_id, pane_id, title, cwd)
    event = Event.new(
      name.to_sym,
      workspace_id.nil? ? nil : Workspace.new(workspace_id),
      window_id.nil? ? nil : Window.new(window_id),
      tab_id.nil? ? nil : Tab.new(tab_id),
      pane_id.nil? ? nil : Pane.new(pane_id),
      title,
      cwd
    )
    __dispatch_event(name, event)
  end

  def self.__dispatch_event(name, event)
    handlers = @event_handlers[name.to_s]
    return false unless handlers
    checkpoint = __command_checkpoint
    begin
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

  def self.__add_pane(id, title, cwd, pid, command_running, last_exit_status)
    @object_data[:pane][id] = [title, cwd, pid, command_running, last_exit_status]
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
