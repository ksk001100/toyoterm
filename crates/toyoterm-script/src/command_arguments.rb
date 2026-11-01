module Toyoterm
  COMMAND_ARGUMENT_KEYS = [:name, :type, :prompt, :required, :default, :choices,
                           :min, :max, :history, :completer, :require_match].freeze
  @command_argument_stack = []
  EMPTY_COMMAND_ARGUMENTS = {}.freeze

  def self.command_arguments
    @command_argument_stack.last || EMPTY_COMMAND_ARGUMENTS
  end

  def self.__command_argument_value(field, value)
    if value.nil?
      raise ArgumentError, "missing command argument: #{field[:name]}" if field[:required]
      return nil
    end
    label = "command argument #{field[:name]}"
    value = case field[:type]
    when :string then __prompt_text(value, label)
    when :integer
      raise TypeError, "#{label} must be an Integer" unless value.is_a?(Integer)
      value
    when :number then __number(value, label)
    when :boolean then __boolean(value, label)
    end
    if !field[:min].nil? && value < field[:min] || !field[:max].nil? && value > field[:max]
      raise ArgumentError, "#{label} is outside its allowed range"
    end
    if field[:choices] && !field[:choices].include?(value)
      raise ArgumentError, "#{label} is not an allowed choice"
    end
    value
  end

  def self.__command_schema(definitions)
    raise TypeError, 'command arguments must be an Array' unless definitions.is_a?(Array)
    raise ArgumentError, 'commands support at most 16 arguments' if definitions.length > 16
    names = []
    definitions.map do |definition|
      raise TypeError, 'command argument definition must be a Hash' unless definition.is_a?(Hash)
      raise ArgumentError, 'unknown command argument option' unless (definition.keys - COMMAND_ARGUMENT_KEYS).empty?
      name = __identifier(definition[:name], 'command argument name')
      __prompt_text(name, 'command argument name', 128)
      raise ArgumentError, "duplicate command argument: #{name}" if names.include?(name)
      names << name
      type = definition.fetch(:type, :string)
      raise ArgumentError, 'argument type must be :string, :integer, :number, or :boolean' unless [:string, :integer, :number, :boolean].include?(type)
      required = definition.fetch(:required, !definition.key?(:default))
      __boolean(required, 'required')
      field = { name: name, type: type, required: required,
                prompt: __prompt_text(definition.fetch(:prompt, name), 'argument prompt', 256) }
      if definition.key?(:min) || definition.key?(:max)
        raise ArgumentError, 'argument ranges require an integer or number' unless [:integer, :number].include?(type)
        field[:min] = __number(definition[:min], 'argument minimum') if definition.key?(:min)
        field[:max] = __number(definition[:max], 'argument maximum') if definition.key?(:max)
        raise ArgumentError, 'argument range is reversed' if field[:min] && field[:max] && field[:min] > field[:max]
      end
      if definition.key?(:choices)
        choices = definition[:choices]
        raise TypeError, 'argument choices must be an Array' unless choices.is_a?(Array)
        raise ArgumentError, 'argument choices must contain 1..4096 values' unless choices.length.between?(1, 4096)
        field[:choices] = choices.map { |value| __command_argument_value(field, value) }.uniq.freeze
        candidates = field[:choices].map { |value| value.nil? ? '' : value.to_s }
        candidates.unshift('') if !required && !candidates.include?('')
        __prompt_candidates(candidates)
      end
      provider = definition[:completer]
      unless provider.nil?
        raise ArgumentError, 'dynamic command completion requires a string argument without choices' unless type == :string && !field[:choices]
        raise TypeError, 'argument completer must respond to call' unless provider.respond_to?(:call)
        field[:completer] = provider
      end
      if definition.key?(:require_match)
        raise ArgumentError, 'require_match requires a dynamic completer' unless provider
        __boolean(definition[:require_match], 'require_match')
      end
      field[:require_match] = !!field[:choices] || type == :boolean || definition.fetch(:require_match, false)
      field[:history] = definition[:history].nil? ? nil : __identifier(definition[:history], 'argument history')
      field[:default] = __command_argument_value(field, definition[:default]) if definition.key?(:default)
      field.freeze
    end.freeze
  end

  def self.__command_schema_info(fields)
    fields.map do |field|
      field.reject { |key, _value| key == :completer }.merge(dynamic: field.key?(:completer)).freeze
    end.freeze
  end

  def self.__command_values(fields, arguments, partial = false)
    raise TypeError, 'command argument values must be a Hash' unless arguments.is_a?(Hash)
    values = {}
    arguments.each do |key, value|
      name = __identifier(key, 'command argument name')
      raise ArgumentError, "duplicate command argument value: #{name}" if values.key?(name)
      field = fields.find { |item| item[:name] == name }
      raise ArgumentError, "unknown command argument: #{name}" unless field
      values[name] = __command_argument_value(field, value)
    end
    unless partial
      fields.each do |field|
        values[field[:name]] = __command_argument_value(field, field[:default]) unless values.key?(field[:name])
      end
    end
    values.freeze
  end

  def self.__parse_command_input(field, text)
    if text.empty? && !field[:required]
      return field[:default] if field.key?(:default) && (field[:default].nil? || field[:default] == '')
      return nil
    end
    value = case field[:type]
    when :string then text
    when :integer then Integer(text, 10)
    when :number then Float(text)
    when :boolean
      raise ArgumentError, 'enter true or false' unless ['true', 'false'].include?(text)
      text == 'true'
    end
    __command_argument_value(field, value)
  end

  def self.__check_prompt_command(name, entry, context)
    raise ArgumentError, "command changed while collecting arguments: #{name}" unless @user_commands[name].equal?(entry)
    [context.workspace, context.window, context.tab, context.pane].each(&:validate!)
  end

  def self.prompt_command(name, arguments: {}, context: nil)
    name = __identifier(name, 'user command name')
    entry = @user_commands[name] || raise(ArgumentError, "undefined user command: #{name}")
    context = @command_contexts.last || CommandContext.new(current_pane) if context.nil?
    raise TypeError, 'command context must be a CallbackContext' unless context.is_a?(CallbackContext)
    __check_prompt_command(name, entry, context)
    values = __command_values(entry[3], arguments, true)
    __collect_command_arguments(name, entry, context, values, 0)
    nil
  end

  def self.__collect_command_arguments(name, entry, context, values, index, retry_text = nil)
    __check_prompt_command(name, entry, context)
    fields = entry[3]
    index += 1 while index < fields.length && values.key?(fields[index][:name])
    if index == fields.length
      call_command(name, arguments: values, context: context)
      return
    end
    field = fields[index]
    choices = field[:choices] || (field[:type] == :boolean ? [true, false] : [])
    candidates = choices.map { |value| value.nil? ? '' : value.to_s }
    candidates.unshift('') if !field[:required] && field[:require_match] && !field[:completer] && !candidates.include?('')
    provider = field[:completer] && proc { |query, _context| field[:completer].call(query, context) }
    initial = retry_text.nil? ? (field.key?(:default) && !field[:default].nil? ? field[:default].to_s : '') : retry_text
    title = retry_text.nil? ? field[:prompt] : "Invalid #{field[:type]}: #{field[:name]}"
    # The input lifecycle belongs to the defining mode, while command execution
    # retains the ordinary command transaction and captured target semantics.
    __with_task_owner(entry[4] || __task_owner) do
      read_string(title: title, initial: initial, completions: candidates, completer: provider,
                  require_match: field[:require_match], history: field[:history]) do |text|
        next if text.nil?
        __check_prompt_command(name, entry, context)
        begin
          value = __parse_command_input(field, text)
        rescue ArgumentError, TypeError => error
          log(:warn, error.message)
          __collect_command_arguments(name, entry, context, values, index, text)
          next
        end
        __collect_command_arguments(name, entry, context, values.merge(field[:name] => value).freeze, index + 1)
      end
    end
  end
end
