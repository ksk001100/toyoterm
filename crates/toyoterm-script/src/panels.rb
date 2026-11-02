module Toyoterm
  @panels = {}

  class Panel
    attr_reader :id
    def initialize(id); @id = id; freeze; end
    def active?; Toyoterm.__panel_active?(@id); end
    def update(lines:, title: nil); Toyoterm.__update_panel(@id, title, lines); end
    def close; active? && Toyoterm.__close_prompt(@id); end
    alias remove close
  end

  def self.__panel_lines(lines)
    raise TypeError, 'panel lines must be an Array' unless lines.is_a?(Array)
    raise ArgumentError, 'panels support at most 4096 lines' if lines.length > 4096
    values = lines.map { |line| __prompt_text(line, 'panel line') }.freeze
    raise ArgumentError, 'panel lines exceed 4 MiB' if values.inject(0) { |n, line| n + line.bytesize } > 4 * 1024 * 1024
    values
  end

  def self.__panel_active?(id)
    @panels.key?(id) && __prompt_active?(id)
  end

  def self.show_panel(title: 'Panel', lines:)
    raise RuntimeError, 'show_panel requires a runtime request' unless @task_runtime_ready
    raise RuntimeError, 'show_panel cannot be used while loading a Ruby source' if __loading_source?
    title = __prompt_text(title, 'panel title', 256)
    lines = __panel_lines(lines)
    raise RuntimeError, 'an input, selection or panel is already pending' unless @select_callbacks.empty?
    @select_id += 1
    id = @select_id
    @select_callbacks[id] = [proc {}, __task_context, [].freeze, nil, __task_owner].freeze
    @panels[id] = true
    __register_prompt_lifetime(id)
    __queue_command(:open_panel, 0, id.to_s, [title, lines].freeze)
    Panel.new(id)
  end

  def self.__update_panel(id, title, lines)
    return false unless __panel_active?(id)
    title = __prompt_text(title, 'panel title', 256) unless title.nil?
    lines = __panel_lines(lines)
    __queue_command(:update_panel, 0, id.to_s, [title, lines].freeze)
    true
  end

  def self.__current_panel_has_title; !@current_command[3][0].nil?; end

  class << self
    alias __before_panels_checkpoint __registration_checkpoint
    alias __before_panels_rollback __rollback_registrations
    alias __before_panels_prepare __prepare_prompt_result
    alias __before_panels_close __close_prompt
    alias __before_panels_rollback_commands __rollback_commands

    def __registration_checkpoint
      [__before_panels_checkpoint, @panels.dup]
    end
    def __rollback_registrations(checkpoint)
      __before_panels_rollback(checkpoint[0])
      @panels = checkpoint[1]
    end
    def __prepare_prompt_result(id)
      __before_panels_prepare(id)
      @panels.delete(id)
    end
    def __close_prompt(id)
      result = __before_panels_close(id)
      @panels.delete(id)
      result
    end
    def __rollback_commands(checkpoint)
      __before_panels_rollback_commands(checkpoint)
      @panels.delete_if { |id, _value| !__prompt_active?(id) }
    end
  end
end
