
module Toyoterm
  module PluginNamespaces
  end
  VERSION = "__TOYOTERM_VERSION__".freeze
  API_VERSION = "__TOYOTERM_API_VERSION__".freeze
  CAPABILITIES = [
    :async_process, :callback_context, :mixed_bar_groups, :registration_handles,
    :select_overlay, :targeted_actions, :typed_events
  ].freeze
  GLOBAL_ACTIONS = [
    :reload_config, :maximize_window, :toggle_maximize, :minimize_window,
    :toggle_fullscreen, :new_workspace
  ].freeze

  NATIVE_EVENTS = [__TOYOTERM_NATIVE_EVENTS__].freeze

  def self.__string(value, label, allow_nil = false, allow_empty = false)
    return nil if allow_nil && value.nil?
    raise TypeError, "#{label} must be a String" unless value.is_a?(String)
    raise ArgumentError, "#{label} cannot be empty" if !allow_empty && value.empty?
    raise ArgumentError, "#{label} contains a NUL byte" if value.include?("\0")
    value.dup.freeze
  end

  def self.__number(value, label, minimum = nil, maximum = nil)
    raise TypeError, "#{label} must be numeric" unless value.is_a?(Numeric)
    number = value.to_f
    raise ArgumentError, "#{label} must be finite" unless number.finite?
    if !minimum.nil? && number < minimum
      message = minimum == 0.000001 ? "#{label} must be positive" : "#{label} must be at least #{minimum}"
      raise ArgumentError, message
    end
    if !maximum.nil? && number > maximum
      raise ArgumentError, "#{label} must be at most #{maximum}"
    end
    value
  end

  def self.__identifier(value, label)
    unless value.is_a?(String) || value.is_a?(Symbol)
      raise TypeError, "#{label} must be a String or Symbol"
    end
    __string(value.to_s, label)
  end

  def self.__boolean(value, label)
    raise TypeError, "#{label} must be true or false" unless value == true || value == false
    value
  end

  def self.__color(value, label)
    value = __string(value, label)
    unless value.length == 7 && value[0] == "#" && value[1, 6].each_byte.all? { |byte| (48..57).include?(byte) || (65..70).include?(byte) || (97..102).include?(byte) }
      raise ArgumentError, "#{label} must be a #RRGGBB color"
    end
    value
  end

  def self.__deep_copy(value, freeze_value = false)
    copy = case value
           when String then value.dup
           when Array then value.map { |item| __deep_copy(item, freeze_value) }
           when Hash
             result = {}
             value.each { |key, item| result[__deep_copy(key, freeze_value)] = __deep_copy(item, freeze_value) }
             result
           else value
           end
    copy.freeze if freeze_value && copy.respond_to?(:freeze)
    copy
  end

  class FontConfig
    attr_reader :family, :fallback, :size, :weight

    def initialize
      @family = "monospace"
      @fallback = []
      @size = 14.0
      @weight = 400
      self.family = @family
      self.fallback = @fallback
    end

    def family=(value)
      @family = Toyoterm.__string(value, "font family")
    end

    def fallback=(value)
      raise TypeError, "font fallback must be an array" unless value.is_a?(Array)
      raise ArgumentError, "font fallback supports at most 32 families" if value.length > 32
      families = value.map { |family| Toyoterm.__string(family, "font fallback entry") }
      if families.include?(@family) || families.uniq.length != families.length
        raise ArgumentError, "duplicate font family in fallback"
      end
      @fallback = families.freeze
    end

    def size=(value)
      @size = Toyoterm.__number(value, "font size", 0.000001)
    end

    def weight=(value)
      raise TypeError, "font weight must be an Integer" unless value.is_a?(Integer)
      raise ArgumentError, "font weight must be between 1 and 1000" unless (1..1000).include?(value)
      @weight = value
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

  class ColorPalette < Array
    def initialize(values)
      super()
      values.each_with_index { |value, index| self[index] = value }
    end

    def []=(index, value)
      super(index, Toyoterm.__color(value, "colors.ansi[#{index}]"))
    end
  end

  class ColorConfig
    COLOR_NAMES = [
      :background, :foreground, :cursor, :selection, :tab_bar, :tab_active,
      :tab_inactive, :workspace_bar, :status_bar, :pane_border,
      :zoomed_pane_border, :search_match, :search_match_active
    ].freeze
    attr_reader :background, :foreground, :cursor, :selection, :ansi,
                :tab_bar, :tab_active, :tab_inactive, :workspace_bar,
                :status_bar, :pane_border, :zoomed_pane_border, :search_match, :search_match_active

    COLOR_NAMES.each do |name|
      define_method("#{name}=") do |value|
        instance_variable_set("@#{name}", Toyoterm.__color(value, "colors.#{name}"))
      end
    end

    def ansi=(value)
      raise TypeError, "colors.ansi must be an Array" unless value.is_a?(Array)
      raise ArgumentError, "colors.ansi must contain exactly 16 colors" unless value.length == 16
      @ansi = ColorPalette.new(value)
    end

    def initialize
      @background = "#090b0e"
      @foreground = "#dce1e8"
      @cursor = "#f5f7fa"
      @selection = "#375891"
      self.ansi = [
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
      @zoomed_pane_border = "#ffbe3a"
      @search_match = "#c4972f"
      @search_match_active = "#ffbe3a"
      COLOR_NAMES.each do |name|
        send("#{name}=", instance_variable_get("@#{name}"))
      end
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
        Toyoterm.__deep_copy(@ansi),
        @tab_bar, @tab_active, @tab_inactive, @workspace_bar, @status_bar,
        @pane_border, @search_match, @search_match_active, @zoomed_pane_border
      ]
    end

    def __restore(snapshot)
      self.background = snapshot[0]
      self.foreground = snapshot[1]
      self.cursor = snapshot[2]
      self.selection = snapshot[3]
      self.ansi = snapshot[4]
      self.tab_bar = snapshot[5]
      self.tab_active = snapshot[6]
      self.tab_inactive = snapshot[7]
      self.workspace_bar = snapshot[8]
      self.status_bar = snapshot[9]
      self.pane_border = snapshot[10]
      self.search_match = snapshot[11]
      self.search_match_active = snapshot[12]
      self.zoomed_pane_border = snapshot[13]
      self
    end

    def __apply_with_overrides(theme, baseline)
      overrides = __snapshot
      __restore(Toyoterm.__deep_copy(theme.__snapshot))
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

  class BarConfig
    def initialize
      @widgets = []
    end

    def add(position, value = nil, &block)
      raise ArgumentError, "bar widget position must be :left, :center, or :right" unless position.is_a?(Symbol)
      unless [:left, :center, :right].include?(position)
        raise ArgumentError, "bar widget position must be :left, :center, or :right"
      end
      if block && !value.nil?
        raise ArgumentError, "bar widget accepts either a value or a block, not both"
      end
      widget = block || value
      @widgets << [position, widget]
      widget
    end

    def group(position, separator: " | ", &block)
      validate_position(position)
      raise ArgumentError, "bar group requires a block" unless block
      group = BarGroup.new(separator)
      block.call(group)
      @widgets << [position, group]
      group
    end

    def __widgets
      @widgets
    end

    def __copy
      copy = BarConfig.new
      @widgets.each { |widget| copy.__widgets << widget.dup }
      copy
    end

    def __checkpoint
      @widgets.map do |position, widget|
        [position, widget.respond_to?(:__checkpoint) ? widget.__checkpoint : nil]
      end
    end

    def __rollback(checkpoint)
      checkpoint.each_with_index do |(_, state), index|
        widget = @widgets[index][1]
        widget.__rollback(state) if state && widget.respond_to?(:__rollback)
      end
    end

    private

    def validate_position(position)
      raise ArgumentError, "bar widget position must be :left, :center, or :right" unless position.is_a?(Symbol)
      unless [:left, :center, :right].include?(position)
        raise ArgumentError, "bar widget position must be :left, :center, or :right"
      end
    end
  end

  class BarGroup
    def initialize(separator)
      raise TypeError, "bar group separator must be a String" unless separator.is_a?(String)
      raise ArgumentError, "bar group separator cannot contain NUL" if separator.include?("\0")
      @separator = separator
      @widgets = []
    end

    def add(value = nil, &block)
      if block && !value.nil?
        raise ArgumentError, "bar group widget accepts either a value or a block, not both"
      end
      @widgets << (block || value)
      self
    end

    def add_async(program, *args, interval: 1.0, initial: "", cwd: nil, &block)
      unless interval.is_a?(Numeric) && interval.to_f.finite? && interval >= 0.1
        raise ArgumentError, "async bar interval must be at least 0.1 seconds"
      end
      initial = Toyoterm.__string(initial, "async bar initial text", false, true)
      raise ArgumentError, "async bar initial text cannot contain NUL" if initial.include?("\0")
      @widgets << AsyncBarWidget.new(program, args, interval.to_f, initial, cwd, block)
      self
    end

    def call(context)
      @widgets.map do |widget|
        value = widget.respond_to?(:call) ? widget.call(context) : widget
        value.nil? ? "" : value.to_s
      end
        .reject { |text| text.nil? || text.empty? }
        .join(@separator)
    end

    def __checkpoint
      @widgets.map do |widget|
        widget.respond_to?(:__checkpoint) ? widget.__checkpoint : nil
      end
    end

    def __rollback(checkpoint)
      @widgets.each_with_index do |widget, index|
        state = checkpoint[index]
        widget.__rollback(state) if state && widget.respond_to?(:__rollback)
      end
    end
  end

  class AsyncBarWidget
    def initialize(program, args, interval, initial, cwd, formatter)
      @program = program
      @args = args
      @interval = interval
      @initial = initial
      @cwd = cwd
      @formatter = formatter
      @task = nil
      @result = nil
      @next_at = 0.0
    end

    def call(context)
      now = Time.now.to_f
      if @task && @task.complete?
        @result = @task.result
        @task = nil
        @next_at = now + @interval
      end

      if @task.nil? && now >= @next_at
        cwd = @cwd.respond_to?(:call) ? @cwd.call(context) : @cwd
        @task = Toyoterm.async(@program, *@args, cwd: cwd)
      end

      return @initial if @result.nil?
      value = @formatter ? @formatter.call(@result) : @result.stdout
      value.nil? ? "" : value.to_s
    end

    def __checkpoint
      [@task, @result, @next_at]
    end

    def __rollback(checkpoint)
      @task, @result, @next_at = checkpoint
    end
  end

  class ImageConfig
    attr_reader :path, :opacity

    def initialize
      @path = nil
      @opacity = 1.0
    end

    def path=(value)
      unless value.nil? || value.is_a?(String)
        raise TypeError, "background image must be a path String or nil"
      end
      if value && (value.empty? || value.include?("\0"))
        raise ArgumentError, "background image path must be non-empty and contain no NUL"
      end
      @path = value && value.dup.freeze
    end

    def opacity=(value)
      @opacity = Toyoterm.__number(value, "background image opacity", 0, 1)
    end

  end

  class WindowConfig
    attr_reader :opacity
    attr_reader :width, :height, :min_width, :min_height,
                :decorations, :resizable, :always_on_top, :title

    [:width, :height, :min_width, :min_height].each do |name|
      define_method("#{name}=") do |value|
        instance_variable_set("@#{name}", Toyoterm.__number(value, "window.#{name}", 0.000001))
      end
    end

    [:decorations, :resizable, :always_on_top].each do |name|
      define_method("#{name}=") do |value|
        instance_variable_set("@#{name}", Toyoterm.__boolean(value, "window.#{name}"))
      end
    end

    def title=(value)
      @title = Toyoterm.__string(value, "window.title")
      raise ArgumentError, "window.title cannot be blank" if @title.strip.empty?
      @title
    end

    def image(&block)
      block.call(@image) if block
      @image
    end

    def opacity=(value)
      raise TypeError, "window opacity must be a number" unless value.is_a?(Numeric)
      raise ArgumentError, "window opacity must be finite" unless value.to_f.finite?
      @opacity = [[value, 0.0].max, 1.0].min
    end

    def initialize
      @opacity = 1.0
      @image = ImageConfig.new
      @width = 960
      @height = 600
      @min_width = 320
      @min_height = 180
      @decorations = true
      @resizable = true
      @always_on_top = false
      @title = "toyoterm"
      self.title = @title
    end

    def bar(position, interval: 1.0, &block)
      Toyoterm.__register_window_bar(position, interval, &block)
    end
  end

  class UiConfig
    attr_reader :padding_x, :padding_y, :line_height,
                :tab_bar, :tab_bar_height, :tab_width,
                :workspace_bar, :workspace_bar_height, :workspace_width,
                :status_bar_height, :pane_divider_width,
                :active_pane_border_width

    [:padding_x, :padding_y, :pane_divider_width, :active_pane_border_width].each do |name|
      define_method("#{name}=") do |value|
        instance_variable_set("@#{name}", Toyoterm.__number(value, "ui.#{name}", 0))
      end
    end

    [:line_height, :tab_bar_height, :tab_width, :workspace_bar_height,
     :workspace_width, :status_bar_height].each do |name|
      define_method("#{name}=") do |value|
        instance_variable_set("@#{name}", Toyoterm.__number(value, "ui.#{name}", 0.000001))
      end
    end

    [:tab_bar, :workspace_bar].each do |name|
      define_method("#{name}=") do |value|
        instance_variable_set("@#{name}", Toyoterm.__boolean(value, "ui.#{name}"))
      end
    end

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
    attr_reader :scroll_lines, :copy_on_select, :allow_osc52_copy, :allow_osc_notifications,
                :allow_osc_attention_requests, :allow_osc_open_url

    def scroll_lines=(value)
      @scroll_lines = Toyoterm.__number(value, "behavior.scroll_lines", 0.000001)
    end

    [:copy_on_select, :allow_osc52_copy, :allow_osc_notifications,
     :allow_osc_attention_requests, :allow_osc_open_url].each do |name|
      define_method("#{name}=") do |value|
        instance_variable_set("@#{name}", Toyoterm.__boolean(value, "behavior.#{name}"))
      end
    end

    def initialize
      @scroll_lines = 3
      @copy_on_select = false
      @allow_osc52_copy = false
      @allow_osc_notifications = false
      @allow_osc_attention_requests = false
      @allow_osc_open_url = false
    end
  end

  class CallbackContext
    attr_reader :workspace, :window, :tab, :pane

    def initialize(pane, workspace = nil, window = nil, tab = nil)
      @workspace, @window, @tab, @pane = Toyoterm.__resolve_context(
        pane, workspace, window, tab
      )
    end

    def actions
      @actions ||= ActionProxy.new(self)
    end
  end

  KeyBindingContext = CallbackContext

  CommandContext = CallbackContext

  class BarContext < CallbackContext
    def initialize(workspace, window, tab, pane)
      super(pane, workspace, window, tab)
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
      @title = title && title.dup.freeze
      @cwd = cwd && cwd.dup.freeze
      @exit_status = exit_status
    end

    def context
      @context ||= CallbackContext.new(
        @pane || Toyoterm.current_pane,
        @workspace || Toyoterm.current_workspace,
        @window || Toyoterm.current_window,
        @tab || Toyoterm.current_tab
      )
    end

    def subject
      @pane || @tab || @window || @workspace
    end
  end

  class ActionProxy
    def initialize(context)
      @context = context
    end

    def action(name, argument = nil)
      Toyoterm.__queue_action(name, argument, @context)
    end

  end

  class Registration
    attr_reader :kind, :name, :id

    def initialize(kind, name, id)
      @kind = kind
      @name = name.freeze
      @id = id
    end

    def active?
      Toyoterm.__registration_active?(@kind, @name, @id)
    end

    def remove
      return false unless active?
      Toyoterm.__remove_registration(@kind, @name, @id)
    end
  end

  class StaticBinding
    def initialize(config, key)
      @config = config
      @key = key
    end

    # Shared by the binding DSL and Toyoterm.action. Use this table so
    # adding an action cannot silently omit one of the two entry points.
    ACTIONS = {
      new_tab: nil, close_pane: nil, close_tab: nil, new_workspace: nil,
      reload_config: nil, search: nil, maximize_window: nil,
      toggle_maximize: nil, minimize_window: nil, toggle_fullscreen: nil,
      toggle_zoom: nil, next_tab: nil, previous_tab: nil,
      next_workspace: nil, previous_workspace: nil, copy_selection: nil,
      next_prompt: nil, previous_prompt: nil,
      next_mark: nil, previous_mark: nil,
      select_next_command_output: nil, select_previous_command_output: nil,
      select_last_command_output: nil,
      paste_clipboard: nil, start_visual_mode: nil, toggle_visual_mode: nil,
      start_visual_selection: nil, select_visual_selection: nil,
      end_visual_selection: nil, yank_selection: nil,
      split: [:left, :right, :up, :down],
      activate_pane: [:left, :right, :up, :down],
      move_visual_selection: [:left, :right, :up, :down, :line_start, :line_end]
    }.freeze
    ACTIONS.each do |name, arguments|
      if arguments
        define_method(name) { |argument| action(name, argument) }
      else
        define_method(name) { action(name) }
      end
    end

    def action(name, argument = nil)
      name, argument = Toyoterm.__normalize_action(name, argument)
      @config.__register_static(@key, name, argument)
      self
    end

    def run(&block)
      @config.__register_dynamic(@key, &block)
      self
    end

    def command(name)
      name = Toyoterm.__identifier(name, "user command name")
      @config.__register_static(@key, :user_command, name)
      self
    end
  end

  StaticBinding::ACTIONS.each do |name, arguments|
    if arguments
      ActionProxy.define_method(name) { |argument| action(name, argument) }
    else
      ActionProxy.define_method(name) { action(name) }
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
      StaticBinding.new(@config, Toyoterm.__string(key, "key").upcase)
    end

    def physical(key, mods = "")
      key = Toyoterm.__string(key, "physical key")
      prefix = Toyoterm.__string(mods, "physical modifiers", false, true).upcase
      prefix = "#{prefix}+" unless prefix.empty?
      StaticBinding.new(@config, "#{prefix}PHYSICAL:#{key.upcase}")
    end

    def unbind(key)
      @config.__unbind(key)
    end

    private

    def binding(key, mods)
      key = Toyoterm.__string(key, "key")
      StaticBinding.new(@config, "#{mods}+#{key.upcase}")
    end
  end

  class Config
    attr_reader :default_shell, :scrollback_lines

    def default_shell=(value)
      @default_shell = Toyoterm.__string(value, "default_shell", true, true)
    end

    def scrollback_lines=(value)
      raise TypeError, "scrollback_lines must be an Integer" unless value.is_a?(Integer)
      raise ArgumentError, "scrollback_lines must be non-negative" if value < 0
      @scrollback_lines = value
    end

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
      block.call(@font) if block
      @font
    end

    def colors(&block)
      block.call(@colors) if block
      @colors
    end

    def theme
      @theme
    end

    def theme=(name)
      name = Toyoterm.__identifier(name, "theme name")
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
      block.call(@window) if block
      @window
    end

    def ui(&block)
      block.call(@ui) if block
      @ui
    end

    def behavior(&block)
      block.call(@behavior) if block
      @behavior
    end

    def to_h
      colors = {}
      ColorConfig::COLOR_NAMES.each { |name| colors[name] = @colors.send(name) }
      colors[:ansi] = @colors.ansi
      Toyoterm.__deep_copy({
        font: { family: @font.family, fallback: @font.fallback, size: @font.size, weight: @font.weight },
        colors: colors,
        window: {
          opacity: @window.opacity, width: @window.width, height: @window.height,
          min_width: @window.min_width, min_height: @window.min_height,
          decorations: @window.decorations, resizable: @window.resizable,
          always_on_top: @window.always_on_top, title: @window.title,
          image: { path: @window.image.path, opacity: @window.image.opacity }
        },
        ui: {
          padding_x: @ui.padding_x, padding_y: @ui.padding_y, line_height: @ui.line_height,
          tab_bar: @ui.tab_bar, tab_bar_height: @ui.tab_bar_height, tab_width: @ui.tab_width,
          workspace_bar: @ui.workspace_bar, workspace_bar_height: @ui.workspace_bar_height,
          workspace_width: @ui.workspace_width, status_bar_height: @ui.status_bar_height,
          pane_divider_width: @ui.pane_divider_width,
          active_pane_border_width: @ui.active_pane_border_width
        },
        behavior: {
          scroll_lines: @behavior.scroll_lines, copy_on_select: @behavior.copy_on_select,
          allow_osc52_copy: @behavior.allow_osc52_copy,
          allow_osc_notifications: @behavior.allow_osc_notifications,
          allow_osc_attention_requests: @behavior.allow_osc_attention_requests,
          allow_osc_open_url: @behavior.allow_osc_open_url
        },
        default_shell: @default_shell, scrollback_lines: @scrollback_lines,
        theme: @theme
      }, true)
    end

    def __checkpoint
      [
        [Toyoterm.__deep_copy(@font.family), Toyoterm.__deep_copy(@font.fallback), @font.size, @font.weight],
        [Toyoterm.__deep_copy(@colors.background), Toyoterm.__deep_copy(@colors.foreground),
         Toyoterm.__deep_copy(@colors.cursor), Toyoterm.__deep_copy(@colors.selection),
         Toyoterm.__deep_copy(@colors.ansi), Toyoterm.__deep_copy(@colors.tab_bar),
         Toyoterm.__deep_copy(@colors.tab_active), Toyoterm.__deep_copy(@colors.tab_inactive),
         @colors.workspace_bar, @colors.status_bar, @colors.pane_border,
         @colors.search_match, @colors.search_match_active, @colors.zoomed_pane_border],
        [@window.opacity, @window.width, @window.height, @window.min_width,
         @window.min_height, @window.decorations, @window.resizable,
         @window.always_on_top, Toyoterm.__deep_copy(@window.title),
         Toyoterm.__deep_copy(@default_shell), @scrollback_lines,
         Toyoterm.__deep_copy(@window.image.path), @window.image.opacity],
        [@ui.padding_x, @ui.padding_y, @ui.line_height, @ui.tab_bar,
         @ui.tab_bar_height, @ui.tab_width, @ui.workspace_bar,
         @ui.workspace_bar_height, @ui.workspace_width, @ui.status_bar_height,
         @ui.pane_divider_width, @ui.active_pane_border_width],
        [@behavior.scroll_lines, @behavior.copy_on_select, @behavior.allow_osc52_copy,
         @behavior.allow_osc_notifications, @behavior.allow_osc_attention_requests,
         @behavior.allow_osc_open_url],
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
      @colors.zoomed_pane_border = colors[13]
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
      @window.image.path = window[11]
      @window.image.opacity = window[12]
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
      @behavior.allow_osc52_copy = behavior[2]
      @behavior.allow_osc_notifications = behavior[3]
      @behavior.allow_osc_attention_requests = behavior[4]
      @behavior.allow_osc_open_url = behavior[5]
      @leader_key = leader[0]
      @leader_timeout = leader[1]
      @theme = leader[2]
      @theme_color_checkpoint = leader[3]
      nil
    end

    def __register_dynamic(key, &block)
      raise ArgumentError, "key binding requires a block" unless block
      key = Toyoterm.__string(key, "key binding").upcase
      raise ArgumentError, "key binding cannot be empty" if key.empty?
      raise ArgumentError, "duplicate key binding: #{key}" if @bindings.key?(key) || @static_bindings.key?(key)
      @bindings[key] = [block, Toyoterm.__registration_owner]
    end

    def keys(&block)
      keys = KeysConfig.new(self)
      return keys unless block
      block.arity == 0 ? keys.instance_eval(&block) : block.call(keys)
      keys
    end

    def leader(key:, mods: "", timeout: 1000)
      key = Toyoterm.__string(key, "leader key").upcase
      mods = Toyoterm.__string(mods, "leader modifiers", false, true).upcase
      raise TypeError, "leader timeout must be an Integer" unless timeout.is_a?(Integer)
      raise ArgumentError, "leader timeout must be positive" if timeout <= 0
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
      key = Toyoterm.__string(key, "key binding").upcase
      raise ArgumentError, "duplicate key binding: #{key}" if @bindings.key?(key) || @static_bindings.key?(key)
      @static_bindings[key] = [action, argument, Toyoterm.__registration_owner]
    end

    def __unbind(key)
      key = Toyoterm.__string(key, "key binding").upcase
      removed = @bindings.delete(key) || @static_bindings.delete(key)
      !removed.nil?
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
      entry = @bindings[key.to_s.upcase]
      return false unless entry
      callback = entry[0]
      checkpoint = Toyoterm.__command_checkpoint
      badge_checkpoint = Toyoterm.__badge_checkpoint
      async_checkpoint = Toyoterm.__async_request_checkpoint
      begin
        callback.call(KeyBindingContext.new(pane))
      rescue => error
        Toyoterm.__rollback_commands(checkpoint)
        Toyoterm.__rollback_badges(badge_checkpoint)
        Toyoterm.__rollback_async_requests(async_checkpoint)
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
      Toyoterm.__object_data(:workspace, @id)[0].dup.freeze
    end

    def windows
      validate!
      Toyoterm.__object_data(:workspace, @id)[1].map { |id| MuxWindow.new(id) }
    end

    def activate
      validate!
      Toyoterm.__queue_command(:activate_workspace, @id, nil)
      self
    end

    def new_window(command: nil, cwd: nil, env: nil)
      validate!
      launch = Toyoterm.__normalize_launch(command, cwd, env)
      Toyoterm.__queue_command(launch ? :create_window_with_launch : :create_window, @id, nil, launch)
      nil
    end


    private
    def __native_kind; :workspace; end
  end

  class MuxWindow < NativeHandle
    def tabs
      validate!
      Toyoterm.__object_data(:window, @id)[0].map { |id| Tab.new(id) }
    end

    def new_tab(command: nil, cwd: nil, env: nil)
      validate!
      launch = Toyoterm.__normalize_launch(command, cwd, env)
      Toyoterm.__queue_command(launch ? :new_tab_with_launch : :new_tab, @id, nil, launch)
      nil
    end

    def close
      validate!
      Toyoterm.__queue_command(:close_window, @id, nil)
      self
    end

    def activate
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
      Toyoterm.__object_data(:tab, @id)[0].dup.freeze
    end

    def panes
      validate!
      Toyoterm.__object_data(:tab, @id)[1].map { |id| Pane.new(id) }
    end

    def zoomed?
      validate!
      Toyoterm.__object_data(:tab, @id)[2]
    end

    def close
      validate!
      Toyoterm.__queue_command(:close_tab, @id, nil)
      self
    end

    def activate
      validate!
      Toyoterm.__queue_command(:activate_tab, @id, nil)
      self
    end


    private
    def __native_kind; :tab; end
  end

  class Pane < NativeHandle

    def title
      validate!
      Toyoterm.__object_data(:pane, @id)[0].dup.freeze
    end

    def icon_title
      validate!
      value = Toyoterm.__object_data(:pane, @id)[11]
      value && value.dup.freeze
    end

    def cwd
      validate!
      value = Toyoterm.__object_data(:pane, @id)[1]
      value && value.dup.freeze
    end

    def pid
      validate!
      Toyoterm.__object_data(:pane, @id)[6]
    end

    def remote_host
      validate!
      value = Toyoterm.__object_data(:pane, @id)[2]
      value && value.dup.freeze
    end

    def user_vars
      validate!
      Toyoterm.__deep_copy(Toyoterm.__object_data(:pane, @id)[5])
    end

    def shell_integration_version
      validate!
      Toyoterm.__object_data(:pane, @id)[3]
    end

    def shell_integration_shell
      validate!
      value = Toyoterm.__object_data(:pane, @id)[4]
      value && value.dup.freeze
    end

    def command_running?
      validate!
      Toyoterm.__object_data(:pane, @id)[7]
    end

    def last_exit_status
      validate!
      Toyoterm.__object_data(:pane, @id)[8]
    end

    def screen_text
      validate!
      Toyoterm.__object_data(:pane, @id)[9].dup
    end

    def zoomed?
      validate!
      Toyoterm.__object_data(:pane, @id)[10]
    end

    def split(direction, command: nil, cwd: nil, env: nil)
      validate!
      direction = Toyoterm.__identifier(direction, "split direction").downcase
      unless ["left", "right", "up", "down"].include?(direction)
        raise ArgumentError, "split direction must be left, right, up, or down"
      end
      launch = Toyoterm.__normalize_launch(command, cwd, env)
      Toyoterm.__queue_command(launch ? :split_with_launch : :split, @id, direction, launch)
      nil
    end

    def close
      validate!
      Toyoterm.__queue_command(:close_pane, @id, nil)
      self
    end

    def activate
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
      value = value.nil? ? nil : Toyoterm.__string(value, "pane badge", false, true)
      Toyoterm.__set_pane_badge(@id, value)
      Toyoterm.__queue_command(value.nil? ? :clear_pane_badge : :set_pane_badge, @id, value)
    end

    def send_text(text)
      validate!
      text = Toyoterm.__string(text, "text", false, true)
      Toyoterm.__queue_command(:send_text, @id, text)
      self
    end

    def search(query, direction: :next)
      validate!
      query = Toyoterm.__string(query, "search query")
      direction = Toyoterm.__identifier(direction, "search direction").downcase
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
      text = Toyoterm.__string(text, "clipboard text", false, true)
      Toyoterm.__queue_command(:clipboard_write, 0, text)
      self
    end

    def __replace(text)
      @text = text
    end
  end

  class ProcessResult
    attr_reader :stdout, :stderr, :exit_status, :error_kind

    def initialize(stdout, stderr, exit_status, error_kind = nil)
      @stdout = stdout.dup.freeze
      @stderr = stderr.dup.freeze
      @exit_status = exit_status
      @error_kind = error_kind
    end

    def success?
      @error_kind.nil? && @exit_status == 0
    end

    def launch_error?
      @error_kind == :launch
    end
  end

  # A handle for a process started by Toyoterm.async.  The handle is safe to
  # retain in a widget closure, so callers do not need global variables just
  # to render the latest result.
  class AsyncTask
    attr_reader :id, :context

    def initialize(id, context)
      @id = id
      @context = context
      @result = nil
      @cancelled = false
    end

    def pending?
      @result.nil? && !@cancelled
    end

    def complete?
      @cancelled || !@result.nil?
    end

    def result
      @result
    end

    def success?
      value = result
      !value.nil? && value.success?
    end

    def error
      return nil unless @result && !@result.success?
      @result.stderr
    end

    def value!
      raise RuntimeError, "asynchronous task is pending" if pending?
      raise RuntimeError, "asynchronous task was cancelled" if cancelled?
      raise RuntimeError, (error.nil? || error.empty? ? "process exited with status #{@result.exit_status}" : error) if !success?
      @result
    end

    def cancel
      return false unless pending?
      @cancelled = true
      Toyoterm.__cancel_async(@id)
      true
    end

    def cancelled?
      @cancelled
    end

    def __complete(result)
      @result = result unless @cancelled
      self
    end
  end

  class Plugin
    class Definition
      attr_reader :name, :version, :api_requirement

      def initialize(name)
        @name = Toyoterm.__identifier(name, "plugin name")
        @version = nil
        @api_requirement = nil
      end

      def version=(value)
        @version = Toyoterm.__string(value, "plugin version")
      end

      def api_requirement=(value)
        @api_requirement = Toyoterm.__string(
          value, "plugin API requirement", true, true
        )
      end

      def command(name, replace: false, &block)
        Toyoterm.command(name, replace: replace, &block)
      end

      def on(name, &block)
        Toyoterm.on(name, &block)
      end

      def keys(&block)
        Toyoterm.__config.keys(&block)
      end

      def theme(name)
        raise ArgumentError, "theme definition requires a block" unless block_given?
        theme = ColorConfig.new
        yield theme
        Toyoterm.__register_theme(name, ColorConfig.new.__restore(theme.__snapshot))
        theme
      end

      def __validate!
        raise ArgumentError, "plugin name cannot be empty" if @name.empty?
        raise ArgumentError, "plugin version is required" if @version.nil?
        @api_requirement = "".freeze if @api_requirement.nil?
      end
    end

    def self.define(name)
      raise RuntimeError, "Plugin.define can only be used while loading a plugin" unless Toyoterm.__loading_plugin?
      raise ArgumentError, "plugin definition requires a block" unless block_given?
      definition = Definition.new(name)
      yield definition
      definition.__validate!
      Toyoterm.__register_plugin(definition)
      definition.freeze
      definition
    end
  end

  @config = Config.new
  @current_pane = Pane.new(0)
  @current_tab = Tab.new(0)
  @current_window = MuxWindow.new(0)
  @current_workspace = Workspace.new(0)
  @clipboard = Clipboard.new
  @env = {}
  @commands = []
  @current_command = nil
  @event_handlers = {}
  @user_commands = {}
  @window_bars = {}
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
  @async_task_id = 0
  @async_callbacks = {}
  @async_tasks = {}
  @async_requests = []
  @async_cancellations = []
  @current_async_request = nil
  @registration_id = 0
  @logs = []
  @plugin_namespace_id = 0
  @select_id = 0
  @select_callbacks = {}

  def self.configure(&block)
    raise ArgumentError, "configuration requires a block" unless block
    block.call(@config)
    @config
  end

  def self.version
    VERSION
  end

  def self.api_version
    API_VERSION
  end

  def self.supports?(capability)
    CAPABILITIES.include?(__identifier(capability, "capability").to_sym)
  end

  def self.config
    @config.to_h
  end

  def self.log(level, message)
    level = __identifier(level, "log level").to_sym
    unless [:debug, :info, :warn, :error].include?(level)
      raise ArgumentError, "log level must be :debug, :info, :warn, or :error"
    end
    message = Toyoterm.__string(message, "log message", false, true)
    @logs << [level, message]
    nil
  end

  def self.__next_log
    @current_log = @logs.shift
    @current_log ? @current_log[0].to_s : ""
  end

  def self.__current_log_message
    @current_log[1]
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

  def self.__resolve_context(pane, workspace = nil, window = nil, tab = nil)
    pane ||= @current_pane
    if tab.nil? && pane
      pair = @object_data[:tab].find { |_id, data| data[1].include?(pane.id) }
      tab = Tab.new(pair[0]) if pair
    end
    if window.nil? && tab
      pair = @object_data[:window].find { |_id, data| data[0].include?(tab.id) }
      window = MuxWindow.new(pair[0]) if pair
    end
    if workspace.nil? && window
      pair = @object_data[:workspace].find { |_id, data| data[1].include?(window.id) }
      workspace = Workspace.new(pair[0]) if pair
    end
    [workspace || @current_workspace, window || @current_window,
     tab || @current_tab, pane]
  end

  def self.windows
    @object_data[:window].keys.sort.map { |id| MuxWindow.new(id) }
  end

  def self.workspaces
    @object_data[:workspace].keys.sort.map { |id| Workspace.new(id) }
  end

  def self.find_workspace(name)
    name = __identifier(name, "workspace name")
    pair = @object_data[:workspace].find { |_id, data| data[0] == name }
    pair ? Workspace.new(pair[0]) : nil
  end

  def self.open_workspace(name)
    name = Toyoterm.__identifier(name, "workspace name")
    __queue_command(:switch_workspace, 0, name)
    nil
  end

  def self.action(name, argument = nil)
    __queue_action(name, argument, CallbackContext.new(current_pane))
  end

  def self.__queue_action(name, argument, context)
    name, argument = __normalize_action(name, argument)
    unless GLOBAL_ACTIONS.include?(name.to_sym)
      [context.workspace, context.window, context.tab, context.pane].each(&:validate!)
    end
    target = [context.workspace.id, context.window.id, context.tab.id, context.pane.id]
    __queue_command(:invoke_action, context.pane.id, name, argument, target)
    nil
  end

  def self.__normalize_action(name, argument)
    name = __identifier(name, "action name").downcase
    name = name.to_sym
    unless StaticBinding::ACTIONS.key?(name)
      raise ArgumentError, "unsupported action: #{name}"
    end
    choices = StaticBinding::ACTIONS[name]
    if choices
      if argument.nil?
        raise ArgumentError, "action #{name} requires #{choices[0...-1].join(', ')}, or #{choices[-1]}"
      end
      argument = __identifier(argument, "action argument").downcase
      unless choices.include?(argument.to_sym)
        raise ArgumentError, "action #{name} requires #{choices[0...-1].join(', ')}, or #{choices[-1]}"
      end
    else
      raise ArgumentError, "action #{name} does not accept an argument" unless argument.nil?
    end
    [name.to_s, argument]
  end

  def self.clipboard
    @clipboard
  end

  # Returns a snapshot. Mutating it never changes the host process environment.
  def self.env
    __deep_copy(@env)
  end

  def self.platform
    :__TOYOTERM_PLATFORM__
  end

  def self.read_file(path)
    path = __string(path, "path")
    __host_read_file(path)
  end

  def self.spawn(program, *args, cwd: nil)
    program = __string(program, "program")
    values = [program] + args.map { |arg| __string(arg, "process argument", false, true) }
    unless cwd.nil?
      cwd = __string(cwd, "cwd")
    end
    ProcessResult.new(*__host_spawn(values, cwd))
  end

  def self.async(program, *args, cwd: nil, &block)
    async_spawn(program, *args, cwd: cwd, &block)
  end

  def self.async_spawn(program, *args, cwd: nil, &block)
    program = __string(program, "program")
    values = [program] + args.map { |arg| __string(arg, "process argument", false, true) }
    unless cwd.nil?
      cwd = __string(cwd, "cwd")
    end

    @async_task_id += 1
    task_id = @async_task_id
    context = CallbackContext.new(current_pane)
    @async_callbacks[task_id] = [block, context]
    @async_requests << [task_id, program, values[1..-1], cwd]
    task = AsyncTask.new(task_id, context)
    @async_tasks[task_id] = task
    task
  end

  def self.select(title: "Select", items:, &block)
    raise ArgumentError, "select requires a block" unless block
    raise RuntimeError, "select cannot be used while loading a plugin" if __loading_plugin?
    title = __string(title, "select title", false, true)
    raise ArgumentError, "select title is too long" if title.bytesize > 256
    raise ArgumentError, "select title cannot contain a line break" if title.include?("\n") || title.include?("\r")
    raise TypeError, "select items must be an Array" unless items.is_a?(Array)
    raise ArgumentError, "select items cannot be empty" if items.empty?
    raise ArgumentError, "select supports at most 4096 items" if items.length > 4096
    total_bytes = 0
    values = items.map do |item|
      item = __string(item, "select item")
      raise ArgumentError, "select item is too long" if item.bytesize > 4096
      raise ArgumentError, "select item cannot contain a line break" if item.include?("\n") || item.include?("\r")
      total_bytes += item.bytesize
      item
    end
    raise ArgumentError, "select items exceed 4 MiB" if total_bytes > 4 * 1024 * 1024
    raise RuntimeError, "a selection is already pending" unless @select_callbacks.empty?
    values.freeze

    @select_id += 1
    id = @select_id
    @select_callbacks[id] = [block, CallbackContext.new(current_pane), values]
    __queue_command(:open_selector, 0, id.to_s, [title, values])
    nil
  end

  def self.plugin(path)
    path = __string(path, "plugin path")
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
    name = __identifier(name, "theme name")
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

  def self.__next_plugin_namespace_id
    @plugin_namespace_id += 1
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
    @plugins[index].api_requirement
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
    bars = {}
    @window_bars.each { |position, entry| bars[position] = [entry[0], entry[1].__copy] }
    @config_transaction = [
      __plugin_checkpoint, bars, @pane_badges.dup,
      __async_request_checkpoint, @logs.length
    ]
    nil
  end

  def self.__rollback_config_transaction
    checkpoint = @config_transaction
    return nil unless checkpoint
    __rollback_plugin(checkpoint[0])
    @window_bars = checkpoint[1]
    @pane_badges = checkpoint[2]
    __rollback_async_requests(checkpoint[3])
    @logs.pop while @logs.length > checkpoint[4]
    @config_transaction = nil
    nil
  end

  def self.__commit_config_transaction
    @config_transaction = nil
    nil
  end

  def self.__registration_owner
    @current_plugin_path
  end

  def self.on(name, &block)
    raise ArgumentError, "event handler requires a block" unless block
    name = Toyoterm.__identifier(name, "event name")
    unless NATIVE_EVENTS.include?(name.to_sym)
      raise ArgumentError, "unknown event: #{name}"
    end
    @registration_id += 1
    (@event_handlers[name] ||= []) << [@registration_id, block, __registration_owner]
    Registration.new(:event, name, @registration_id)
  end

  def self.command(name, replace: false, &block)
    raise ArgumentError, "user command requires a block" unless block
    name = Toyoterm.__identifier(name, "user command name")
    if @user_commands.key?(name) && !replace
      raise ArgumentError, "duplicate user command: #{name}"
    end
    @registration_id += 1
    @user_commands[name] = [@registration_id, block, __registration_owner]
    Registration.new(:command, name, @registration_id)
  end

  def self.__remove_registration(kind, name, id)
    case kind
    when :event
      handlers = @event_handlers[name]
      return false unless handlers
      before = handlers.length
      handlers.delete_if { |entry| entry[0] == id }
      @event_handlers.delete(name) if handlers.empty?
      handlers.length != before
    when :command
      entry = @user_commands[name]
      return false unless entry && entry[0] == id
      @user_commands.delete(name)
      true
    else
      false
    end
  end

  def self.__registration_active?(kind, name, id)
    case kind
    when :event
      handlers = @event_handlers[name]
      !handlers.nil? && handlers.any? { |entry| entry[0] == id }
    when :command
      entry = @user_commands[name]
      !entry.nil? && entry[0] == id
    else
      false
    end
  end

  def self.__register_window_bar(position, interval, &block)
    raise ArgumentError, "window bar requires a block" unless block
    raise ArgumentError, "window bar position must be :top or :bottom" unless position.is_a?(Symbol)
    unless [:top, :bottom].include?(position)
      raise ArgumentError, "window bar position must be :top or :bottom"
    end
    raise ArgumentError, "window bar is already configured for #{position}" if @window_bars.key?(position)
    interval = __number(interval, "window bar interval")
    raise ArgumentError, "window bar interval must be at least 0.1 seconds" if interval < 0.1
    bar = BarConfig.new
    block.call(bar)
    @window_bars[position] = [interval, bar]
    bar
  end

  def self.__bar_count
    @window_bars.length
  end

  def self.__bar_position(index)
    @window_bars.keys[index]
  end

  def self.__bar_interval(index)
    interval = @window_bars.values[index][0]
    unless interval.is_a?(Numeric)
      raise TypeError, "window bar interval must be numeric"
    end
    interval
  end

  def self.__invoke_bar(position)
    entry = @window_bars[position.to_sym]
    return nil unless entry
    context = BarContext.new(current_workspace, current_window, current_tab, current_pane)
    checkpoint = __command_checkpoint
    badge_checkpoint = __badge_checkpoint
    async_checkpoint = __async_request_checkpoint
    bar_checkpoint = entry[1].__checkpoint
    begin
      widgets = entry[1].__widgets.map do |widget|
        value = widget[1].respond_to?(:call) ? widget[1].call(context) : widget[1]
        text = value.nil? ? "" : value.to_s
        raise ArgumentError, "bar widget text cannot contain NUL" if text.include?("\0")
        [widget[0], text]
      end
      widgets.inject("#{widgets.length};") do |encoded, widget|
        alignment = { left: "l", center: "c", right: "r" }[widget[0]]
        encoded << alignment << "#{widget[1].bytesize}:" << widget[1]
      end
    rescue => error
      __rollback_async_requests(async_checkpoint)
      entry[1].__rollback(bar_checkpoint)
      raise error
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
    entry = @user_commands[name.to_s]
    raise ArgumentError, "undefined user command: #{name}" unless entry
    callback = entry[1]
    checkpoint = __command_checkpoint
    badge_checkpoint = __badge_checkpoint
    async_checkpoint = __async_request_checkpoint
    begin
      callback.call(CommandContext.new(pane))
    rescue => error
      __rollback_commands(checkpoint)
      __rollback_badges(badge_checkpoint)
      __rollback_async_requests(async_checkpoint)
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
      window_id.nil? ? nil : MuxWindow.new(window_id),
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
    handlers.dup.each do |entry|
      checkpoint = __callback_checkpoint
      begin
        entry[1].call(event)
      rescue => error
        __rollback_callback(checkpoint)
        Toyoterm.log(:error, "event handler #{name} failed: #{error}")
      end
    end
    true
  end

  def self.__callback_checkpoint
    bars = {}
    @window_bars.each { |position, entry| bars[position] = [entry[0], entry[1].__copy] }
    [
      __command_checkpoint, __badge_checkpoint, __async_request_checkpoint,
      __plugin_checkpoint, bars, @logs.length
    ]
  end

  def self.__rollback_callback(checkpoint)
    __rollback_commands(checkpoint[0])
    __rollback_badges(checkpoint[1])
    __rollback_async_requests(checkpoint[2])
    __rollback_plugin(checkpoint[3])
    @window_bars = checkpoint[4]
    @logs.pop while @logs.length > checkpoint[5]
    nil
  end

  def self.__set_current_pane(id)
    @current_pane = Pane.new(id)
    @live_handles[:pane] << id unless @live_handles[:pane].include?(id)
  end

  def self.__reset_object_model(workspace, window, tab, pane)
    @current_workspace = Workspace.new(workspace)
    @current_window = MuxWindow.new(window)
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

  def self.__add_tab(id, title, panes, zoomed)
    @object_data[:tab][id] = [title, panes, zoomed]
  end

  def self.__add_pane(id, title, cwd, remote_host, shell_integration_version, shell_integration_shell, user_vars, pid, command_running, last_exit_status, screen_text, zoomed, icon_title)
    @object_data[:pane][id] = [title, cwd, remote_host, shell_integration_version, shell_integration_shell, Hash[*user_vars], pid, command_running, last_exit_status, screen_text, zoomed, icon_title]
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

  def self.__queue_command(type, pane_id, payload, launch = nil, context = nil)
    @commands << [type, pane_id, payload, launch, context]
  end

  def self.__command_checkpoint
    @commands.length
  end

  def self.__rollback_commands(checkpoint)
    while @commands.length > checkpoint
      command = @commands.pop
      @select_callbacks.delete(command[2].to_i) if command && command[0] == :open_selector
    end
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

  def self.__current_selector_title
    @current_command[3][0]
  end

  def self.__current_selector_item_count
    @current_command[3][1].length
  end

  def self.__current_selector_item(index)
    @current_command[3][1][index]
  end

  def self.__invoke_select_callback(id, selection)
    entry = @select_callbacks.delete(id)
    return false unless entry
    callback, context, items = entry
    unless selection.nil? || items.include?(selection)
      raise ArgumentError, "select result is not one of the requested items"
    end
    checkpoint = __command_checkpoint
    badge_checkpoint = __badge_checkpoint
    async_checkpoint = __async_request_checkpoint
    begin
      callback.call(selection, context)
    rescue => error
      __rollback_commands(checkpoint)
      __rollback_badges(badge_checkpoint)
      __rollback_async_requests(async_checkpoint)
      raise error
    end
    true
  end

  def self.__current_command_argument
    @current_command[3]
  end

  def self.__current_command_context(index)
    @current_command[4][index]
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

  def self.__invoke_async_callback(id, stdout, stderr, exit_status, launch_error = false)
    task = @async_tasks.delete(id)
    return true unless task
    result = ProcessResult.new(stdout, stderr, exit_status, launch_error ? :launch : nil)
    task.__complete(result)
    callback_entry = @async_callbacks.delete(id)
    return true unless callback_entry && callback_entry[0] && !task.cancelled?
    callback, context = callback_entry
    checkpoint = __command_checkpoint
    badge_checkpoint = __badge_checkpoint
    async_checkpoint = __async_request_checkpoint
    begin
      callback.call(result, context)
    rescue => error
      __rollback_commands(checkpoint)
      __rollback_badges(badge_checkpoint)
      __rollback_async_requests(async_checkpoint)
      raise error
    end
    true
  end

  def self.__cancel_async(id)
    @async_callbacks.delete(id)
    @async_tasks.delete(id)
    request = @async_requests.find { |entry| entry[0] == id }
    if request
      @async_requests.delete(request)
    else
      @async_cancellations << id
    end
    nil
  end

  def self.__next_async_cancellation
    @async_cancellations.shift || 0
  end

  def self.__async_request_checkpoint
    @async_requests.length
  end

  def self.__rollback_async_requests(checkpoint)
    while @async_requests.length > checkpoint
      req = @async_requests.pop
      if req
        @async_callbacks.delete(req[0])
        @async_tasks.delete(req[0])
      end
    end
  end

  def self.__next_async_request
    @current_async_request = @async_requests.shift
    @current_async_request ? @current_async_request[0] : 0
  end

  def self.__current_async_program
    @current_async_request[1]
  end

  def self.__current_async_arg_count
    @current_async_request[2].length
  end

  def self.__current_async_arg(index)
    @current_async_request[2][index]
  end

  def self.__current_async_has_cwd
    !@current_async_request[3].nil?
  end

  def self.__current_async_cwd
    @current_async_request[3]
  end
end
