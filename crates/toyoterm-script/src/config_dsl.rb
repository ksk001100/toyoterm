
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
    attr_accessor :background, :foreground, :cursor, :selection, :ansi,
                  :tab_bar, :tab_active, :tab_inactive, :workspace_bar,
                  :status_bar, :pane_border, :search_match, :search_match_active

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
      @tab_bar = "#11151b"
      @tab_active = "#18243a"
      @tab_inactive = "#15191f"
      @workspace_bar = "#0d1014"
      @status_bar = "#101419"
      @pane_border = "#375891"
      @search_match = "#c4972f"
      @search_match_active = "#ffbe3a"
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

    def __snapshot
      [
        @background, @foreground, @cursor, @selection,
        @ansi.is_a?(Array) ? @ansi.dup : @ansi,
        @tab_bar, @tab_active, @tab_inactive, @workspace_bar, @status_bar,
        @pane_border, @search_match, @search_match_active
      ]
    end

    def __restore(snapshot)
      @background = snapshot[0]
      @foreground = snapshot[1]
      @cursor = snapshot[2]
      @selection = snapshot[3]
      @ansi = snapshot[4].is_a?(Array) ? snapshot[4].dup : snapshot[4]
      @tab_bar = snapshot[5]
      @tab_active = snapshot[6]
      @tab_inactive = snapshot[7]
      @workspace_bar = snapshot[8]
      @status_bar = snapshot[9]
      @pane_border = snapshot[10]
      @search_match = snapshot[11]
      @search_match_active = snapshot[12]
      self
    end

    def __apply_with_overrides(theme, baseline)
      overrides = __snapshot
      __restore(theme.__snapshot)
      return self unless baseline
      themed = __snapshot
      overrides.each_with_index do |value, index|
        if index == 4 && value.is_a?(Array) && baseline[index].is_a?(Array) &&
            value.length == baseline[index].length
          value.each_with_index do |color, color_index|
            themed[index][color_index] = color if color != baseline[index][color_index]
          end
        elsif value != baseline[index]
          themed[index] = value
        end
      end
      __restore(themed)
    end
  end

  class WindowConfig
    attr_accessor :opacity, :width, :height, :min_width, :min_height,
                  :decorations, :resizable, :always_on_top, :title

    def initialize
      @opacity = 1.0
      @width = 960
      @height = 600
      @min_width = 320
      @min_height = 180
      @decorations = true
      @resizable = true
      @always_on_top = false
      @title = "toyoterm"
    end
  end

  class UiConfig
    attr_accessor :padding_x, :padding_y, :line_height,
                  :tab_bar, :tab_bar_height, :tab_width,
                  :workspace_bar, :workspace_bar_height, :workspace_width,
                  :status_bar_height, :pane_divider_width,
                  :active_pane_border_width

    def initialize
      @padding_x = 8
      @padding_y = 8
      @line_height = 1.2857143
      @tab_bar = true
      @tab_bar_height = 30
      @tab_width = 160
      @workspace_bar = true
      @workspace_bar_height = 24
      @workspace_width = 160
      @status_bar_height = 24
      @pane_divider_width = 2
      @active_pane_border_width = 2
    end
  end

  class BehaviorConfig
    attr_accessor :scroll_lines, :copy_on_select

    def initialize
      @scroll_lines = 3
      @copy_on_select = false
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
    attr_reader :name, :workspace, :window, :tab, :pane, :title, :cwd, :exit_status

    def initialize(name, workspace = nil, window = nil, tab = nil, pane = nil, title = nil, cwd = nil, exit_status = nil)
      @name = name
      @workspace = workspace
      @window = window
      @tab = tab
      @pane = pane
      @title = title
      @cwd = cwd
      @exit_status = exit_status
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

    def close_tab
      @config.__register_static(@key, :close_tab, nil)
      self
    end

    def new_workspace
      @config.__register_static(@key, :new_workspace, nil)
      self
    end

    def reload_config
      @config.__register_static(@key, :reload_config, nil)
      self
    end

    def search
      @config.__register_static(@key, :search, nil)
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

    def next_tab
      @config.__register_static(@key, :next_tab, nil)
      self
    end

    def previous_tab
      @config.__register_static(@key, :previous_tab, nil)
      self
    end

    def next_workspace
      @config.__register_static(@key, :next_workspace, nil)
      self
    end

    def previous_workspace
      @config.__register_static(@key, :previous_workspace, nil)
      self
    end

    def copy_selection
      @config.__register_static(@key, :copy_selection, nil)
      self
    end

    def paste_clipboard
      @config.__register_static(@key, :paste_clipboard, nil)
      self
    end

    def start_visual_selection
      @config.__register_static(@key, :start_visual_selection, nil)
      self
    end

    def start_visual_mode
      @config.__register_static(@key, :start_visual_mode, nil)
      self
    end

    def enter_visual_mode
      start_visual_mode
    end

    def toggle_visual_mode
      @config.__register_static(@key, :toggle_visual_mode, nil)
      self
    end

    def toggle_visual_selection
      toggle_visual_mode
    end

    def select_visual_selection
      @config.__register_static(@key, :select_visual_selection, nil)
      self
    end

    def select
      select_visual_selection
    end

    def end_visual_selection
      @config.__register_static(@key, :end_visual_selection, nil)
      self
    end

    def exit_visual_mode
      end_visual_selection
    end

    def move_visual_selection(direction)
      direction = direction.to_s.downcase
      unless ["left", "right", "up", "down", "line_start", "line_end"].include?(direction)
        raise ArgumentError, "visual selection direction must be left, right, up, down, line_start, or line_end"
      end
      @config.__register_static(@key, :move_visual_selection, direction)
      self
    end

    def visual_move(direction)
      move_visual_selection(direction)
    end

    def yank_selection
      @config.__register_static(@key, :yank_selection, nil)
      self
    end

    def copy_visual_selection
      yank_selection
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

    def ctrl_alt(key)
      binding(key, "CTRL+ALT")
    end

    def ctrl_super(key)
      binding(key, "CTRL+SUPER")
    end

    def primary(key)
      binding(key, Toyoterm.__primary_modifier)
    end

    def primary_shift(key)
      mods = Toyoterm.__primary_modifier
      binding(key, mods == "SUPER" ? "SHIFT+SUPER" : "#{mods}+SHIFT")
    end

    def primary_alt(key)
      mods = Toyoterm.__primary_modifier
      binding(key, mods == "SUPER" ? "ALT+SUPER" : "#{mods}+ALT")
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

    def key(key)
      StaticBinding.new(@config, key.to_s.upcase)
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
      @ui = UiConfig.new
      @behavior = BehaviorConfig.new
      @default_shell = nil
      @scrollback_lines = 10_000
      @bindings = {}
      @static_bindings = {}
      @leader_key = nil
      @leader_timeout = 1000
      @theme = nil
      @theme_color_checkpoint = nil
    end

    def font(&block)
      block ? block.call(@font) : @font
    end

    def colors(&block)
      block ? block.call(@colors) : @colors
    end

    def theme(name = nil)
      return @theme if name.nil?
      self.theme = name
    end

    def theme=(name)
      name = name.to_s
      raise ArgumentError, "theme name cannot be empty" if name.empty?
      @theme = name
      @theme_color_checkpoint = @colors.__snapshot
      __apply_theme(Toyoterm.__theme(name))
      name
    end

    def __apply_theme(theme)
      return false unless theme
      @colors.__apply_with_overrides(theme, @theme_color_checkpoint)
      @theme_color_checkpoint = nil
      true
    end

    def window(&block)
      block ? block.call(@window) : @window
    end

    def ui(&block)
      block ? block.call(@ui) : @ui
    end

    def behavior(&block)
      block ? block.call(@behavior) : @behavior
    end

    def __checkpoint
      [
        [@font.family, @font.fallback.dup, @font.size, @font.weight],
        [@colors.background, @colors.foreground, @colors.cursor, @colors.selection,
         @colors.ansi.dup, @colors.tab_bar, @colors.tab_active, @colors.tab_inactive,
         @colors.workspace_bar, @colors.status_bar, @colors.pane_border,
         @colors.search_match, @colors.search_match_active],
        [@window.opacity, @window.width, @window.height, @window.min_width,
         @window.min_height, @window.decorations, @window.resizable,
         @window.always_on_top, @window.title, @default_shell, @scrollback_lines],
        [@ui.padding_x, @ui.padding_y, @ui.line_height, @ui.tab_bar,
         @ui.tab_bar_height, @ui.tab_width, @ui.workspace_bar,
         @ui.workspace_bar_height, @ui.workspace_width, @ui.status_bar_height,
         @ui.pane_divider_width, @ui.active_pane_border_width],
        [@behavior.scroll_lines, @behavior.copy_on_select],
        [@leader_key, @leader_timeout, @theme,
         @theme_color_checkpoint && @theme_color_checkpoint.map { |value| value.is_a?(Array) ? value.dup : value }]
      ]
    end

    def __restore(checkpoint)
      font, colors, window, ui, behavior, leader = checkpoint
      @font.family = font[0]
      @font.fallback = font[1]
      @font.size = font[2]
      @font.weight = font[3]
      @colors.background = colors[0]
      @colors.foreground = colors[1]
      @colors.cursor = colors[2]
      @colors.selection = colors[3]
      @colors.ansi = colors[4]
      @colors.tab_bar = colors[5]
      @colors.tab_active = colors[6]
      @colors.tab_inactive = colors[7]
      @colors.workspace_bar = colors[8]
      @colors.status_bar = colors[9]
      @colors.pane_border = colors[10]
      @colors.search_match = colors[11]
      @colors.search_match_active = colors[12]
      @window.opacity = window[0]
      @window.width = window[1]
      @window.height = window[2]
      @window.min_width = window[3]
      @window.min_height = window[4]
      @window.decorations = window[5]
      @window.resizable = window[6]
      @window.always_on_top = window[7]
      @window.title = window[8]
      @default_shell = window[9]
      @scrollback_lines = window[10]
      @ui.padding_x = ui[0]
      @ui.padding_y = ui[1]
      @ui.line_height = ui[2]
      @ui.tab_bar = ui[3]
      @ui.tab_bar_height = ui[4]
      @ui.tab_width = ui[5]
      @ui.workspace_bar = ui[6]
      @ui.workspace_bar_height = ui[7]
      @ui.workspace_width = ui[8]
      @ui.status_bar_height = ui[9]
      @ui.pane_divider_width = ui[10]
      @ui.active_pane_border_width = ui[11]
      @behavior.scroll_lines = behavior[0]
      @behavior.copy_on_select = behavior[1]
      @leader_key = leader[0]
      @leader_timeout = leader[1]
      @theme = leader[2]
      @theme_color_checkpoint = leader[3]
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
      badge_checkpoint = Toyoterm.__badge_checkpoint
      begin
        callback.call(KeyBindingContext.new(pane))
      rescue => error
        Toyoterm.__rollback_commands(checkpoint)
        Toyoterm.__rollback_badges(badge_checkpoint)
        raise error
      end
      true
    end

    def __plugin_checkpoint
      [@bindings.dup, @static_bindings.dup, __checkpoint]
    end

    def __rollback_plugin(checkpoint)
      @bindings = checkpoint[0]
      @static_bindings = checkpoint[1]
      __restore(checkpoint[2])
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

    def create_window(command: nil, cwd: nil, env: nil)
      validate!
      launch = Toyoterm.__normalize_launch(command, cwd, env)
      Toyoterm.__queue_command(launch ? :create_window_with_launch : :create_window, @id, nil, launch)
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

    def new_tab(command: nil, cwd: nil, env: nil)
      validate!
      launch = Toyoterm.__normalize_launch(command, cwd, env)
      Toyoterm.__queue_command(launch ? :new_tab_with_launch : :new_tab, @id, nil, launch)
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

    def screen_text
      validate!
      Toyoterm.__object_data(:pane, @id)[5].dup
    end

    def split(direction, command: nil, cwd: nil, env: nil)
      validate!
      direction = direction.to_s.downcase
      unless ["left", "right", "up", "down"].include?(direction)
        raise ArgumentError, "split direction must be left, right, up, or down"
      end
      launch = Toyoterm.__normalize_launch(command, cwd, env)
      Toyoterm.__queue_command(launch ? :split_with_launch : :split, @id, direction, launch)
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
      value = value.nil? ? nil : value.to_s
      Toyoterm.__set_pane_badge(@id, value)
      Toyoterm.__queue_command(value.nil? ? :clear_pane_badge : :set_pane_badge, @id, value)
    end

    def send_text(text)
      validate!
      text = text.to_s
      raise ArgumentError, "text contains a NUL byte" if text.index("\0")
      Toyoterm.__queue_command(:send_text, @id, text)
      self
    end

    def search(query, direction: :next)
      validate!
      query = query.to_s
      raise ArgumentError, "search query cannot be empty" if query.empty?
      raise ArgumentError, "search query contains a NUL byte" if query.index("\0")
      direction = direction.to_s.downcase
      unless ["next", "previous"].include?(direction)
        raise ArgumentError, "search direction must be next or previous"
      end
      Toyoterm.__queue_command(:search_pane, @id, query, direction)
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

      def theme(name)
        raise ArgumentError, "theme definition requires a block" unless block_given?
        theme = ColorConfig.new
        yield theme
        Toyoterm.__register_theme(name, theme)
        theme
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
  @themes = {}
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

  def self.switch_workspace(name)
    name = name.to_s
    raise ArgumentError, "workspace name cannot be empty" if name.empty?
    raise ArgumentError, "workspace name contains a NUL byte" if name.index("\0")
    __queue_command(:switch_workspace, 0, name)
    nil
  end

  def self.action(name, argument = nil)
    name = name.to_s.downcase
    raise ArgumentError, "action name cannot be empty" if name.empty?

    no_argument = [
      "new_tab", "close_pane", "close_tab", "new_workspace",
      "reload_config", "search", "maximize_window", "toggle_maximize",
      "minimize_window", "toggle_fullscreen", "next_tab", "previous_tab",
      "next_workspace", "previous_workspace", "copy_selection",
      "paste_clipboard", "start_visual_mode", "toggle_visual_mode",
      "start_visual_selection", "select_visual_selection",
      "end_visual_selection", "yank_selection"
    ]
    pane_directions = ["left", "right", "up", "down"]
    visual_motions = pane_directions + ["line_start", "line_end"]

    if no_argument.include?(name)
      raise ArgumentError, "action #{name} does not accept an argument" unless argument.nil?
      normalized_argument = nil
    elsif ["split", "activate_pane"].include?(name)
      normalized_argument = argument.to_s.downcase
      unless pane_directions.include?(normalized_argument)
        raise ArgumentError, "action #{name} requires left, right, up, or down"
      end
    elsif name == "move_visual_selection"
      normalized_argument = argument.to_s.downcase
      unless visual_motions.include?(normalized_argument)
        raise ArgumentError, "move_visual_selection requires left, right, up, down, line_start, or line_end"
      end
    else
      raise ArgumentError, "unsupported action: #{name}"
    end

    __queue_command(:invoke_action, 0, name, normalized_argument)
    nil
  end

  def self.clipboard
    @clipboard
  end

  # Returns a snapshot. Mutating it never changes the host process environment.
  def self.env
    @env.dup
  end

  def self.platform
    :__TOYOTERM_PLATFORM__
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

  def self.themes
    @themes.keys.dup
  end

  def self.__theme(name)
    @themes[name.to_s]
  end

  def self.__register_theme(name, theme)
    name = name.to_s
    raise ArgumentError, "theme name cannot be empty" if name.empty?
    raise ArgumentError, "duplicate theme name: #{name}" if @themes.key?(name)
    @themes[name] = theme
    @config.__apply_theme(theme) if @config.theme == name
    theme
  end

  def self.__validate_theme!
    name = @config.theme
    raise ArgumentError, "unknown theme: #{name}" if name && !@themes.key?(name)
    nil
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
      @plugin_requests.length,
      @themes.dup
    ]
  end

  def self.__rollback_plugin(checkpoint)
    @plugins = checkpoint[0]
    @user_commands = checkpoint[1]
    @event_handlers = checkpoint[2]
    @config.__rollback_plugin(checkpoint[3])
    @plugin_requests.pop while @plugin_requests.length > checkpoint[4]
    @themes = checkpoint[5]
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
    @config_transaction = [@config.__checkpoint, @status_interval, @status_callback, @pane_badges.dup]
    nil
  end

  def self.__rollback_config_transaction
    checkpoint = @config_transaction
    return nil unless checkpoint
    @config.__restore(checkpoint[0])
    @status_interval = checkpoint[1]
    @status_callback = checkpoint[2]
    @pane_badges = checkpoint[3]
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
    badge_checkpoint = __badge_checkpoint
    begin
      @status_callback.call(context).to_s
    ensure
      __rollback_commands(checkpoint)
      __rollback_badges(badge_checkpoint)
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
    badge_checkpoint = __badge_checkpoint
    begin
      callback.call(CommandContext.new(pane))
    rescue => error
      __rollback_commands(checkpoint)
      __rollback_badges(badge_checkpoint)
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

  def self.__emit_native_event(name, workspace_id, window_id, tab_id, pane_id, title, cwd, exit_status)
    event = Event.new(
      name.to_sym,
      workspace_id.nil? ? nil : Workspace.new(workspace_id),
      window_id.nil? ? nil : Window.new(window_id),
      tab_id.nil? ? nil : Tab.new(tab_id),
      pane_id.nil? ? nil : Pane.new(pane_id),
      title,
      cwd,
      exit_status
    )
    __dispatch_event(name, event)
  end

  def self.__dispatch_event(name, event)
    handlers = @event_handlers[name.to_s]
    return false unless handlers
    checkpoint = __command_checkpoint
    badge_checkpoint = __badge_checkpoint
    begin
      handlers.each { |handler| handler.call(event) }
    rescue => error
      __rollback_commands(checkpoint)
      __rollback_badges(badge_checkpoint)
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

  def self.__add_pane(id, title, cwd, pid, command_running, last_exit_status, screen_text)
    @object_data[:pane][id] = [title, cwd, pid, command_running, last_exit_status, screen_text]
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

  def self.__badge_checkpoint
    @pane_badges.dup
  end

  def self.__rollback_badges(checkpoint)
    @pane_badges = checkpoint
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

  def self.__normalize_launch(command, cwd, env)
    return nil if command.nil? && cwd.nil? && env.nil?

    if command.nil?
      program = nil
      args = []
    elsif command.is_a?(String)
      program = command
      args = []
    elsif command.is_a?(Array)
      raise ArgumentError, "command array cannot be empty" if command.empty?
      unless command.all? { |part| part.is_a?(String) }
        raise TypeError, "command array entries must be strings"
      end
      program = command[0]
      args = command[1, command.length - 1]
    else
      raise TypeError, "command must be a string, an array of strings, or nil"
    end
    raise ArgumentError, "command program cannot be empty" if !program.nil? && program.empty?

    unless cwd.nil? || cwd.is_a?(String)
      raise TypeError, "cwd must be a string or nil"
    end
    raise ArgumentError, "cwd cannot be empty" if cwd == ""

    env = {} if env.nil?
    raise TypeError, "env must be a hash or nil" unless env.is_a?(Hash)
    env.each do |key, value|
      raise TypeError, "environment names must be strings" unless key.is_a?(String)
      raise ArgumentError, "environment name cannot be empty" if key.empty?
      raise ArgumentError, "environment name cannot contain =" if key.index("=")
      unless value.nil? || value.is_a?(String)
        raise TypeError, "environment values must be strings or nil"
      end
    end

    values = [program, *args, cwd, *env.keys, *env.values].compact
    raise ArgumentError, "launch value contains a NUL byte" if values.any? { |value| value.index("\0") }
    [program, args, cwd, env]
  end

  def self.__queue_command(type, pane_id, payload, launch = nil)
    @commands << [type, pane_id, payload, launch]
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

  def self.__current_command_search_direction
    @current_command[3]
  end

  def self.__current_command_argument
    @current_command[3]
  end

  def self.__current_launch_has_program
    !@current_command[3][0].nil?
  end

  def self.__current_launch_program
    @current_command[3][0]
  end

  def self.__current_launch_arg_count
    @current_command[3][1].length
  end

  def self.__current_launch_arg(index)
    @current_command[3][1][index]
  end

  def self.__current_launch_has_cwd
    !@current_command[3][2].nil?
  end

  def self.__current_launch_cwd
    @current_command[3][2]
  end

  def self.__current_launch_env_count
    @current_command[3][3].length
  end

  def self.__current_launch_env_key(index)
    @current_command[3][3].keys[index]
  end

  def self.__current_launch_env_value_is_nil(index)
    @current_command[3][3].values[index].nil?
  end

  def self.__current_launch_env_value(index)
    @current_command[3][3].values[index]
  end
end
