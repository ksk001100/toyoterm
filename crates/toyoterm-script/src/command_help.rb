module Toyoterm
  def self.command_definition(name)
    info = command_info(name)
    return nil unless info && info[:source]
    { name: info[:name], source: info[:source], line: info[:line] }.freeze
  end

  # Help is a bounded presentation of metadata; the full values remain
  # available through inspection APIs and the definition callback.
  def self.__help_value(value)
    text = value.is_a?(String) ? value : value.inspect
    chars = text.each_char.take(160)
    shortened = chars.join
    display = value.is_a?(String) ? shortened.inspect : shortened
    display + (shortened.bytesize < text.bytesize ? '…' : '')
  end

  def self.__command_help_items(info, context, definition)
    rows = ["Command: #{__help_value(info[:name])}",
            "Description: #{__help_value(info[:description])}"]
    keys = key_bindings(pane: context.pane).values.select do |binding|
      binding[:kind] == :command && binding[:argument] == info[:name]
    end.map { |binding| binding[:key] }
    rows << "Keys: #{keys.empty? ? '(none in this Pane)' : keys.length}"
    keys.take(32).each { |key| rows << "  Key: #{__help_value(key)}" }
    rows << '  More keys: use Toyoterm.command_keys' if keys.length > 32
    rows << "Arguments: #{info[:arguments].length}"
    info[:arguments].each do |field|
      rows << "  Argument: #{__help_value(field[:name])} (#{field[:type]}, #{field[:required] ? 'required' : 'optional'})"
      rows << "    Prompt: #{__help_value(field[:prompt])}"
      rows << "    Default: #{__help_value(field[:default])}" if field.key?(:default)
      rows << "    Minimum: #{__help_value(field[:min])}" if field.key?(:min)
      rows << "    Maximum: #{__help_value(field[:max])}" if field.key?(:max)
      if field[:choices]
        rows << "    Choices: #{field[:choices].length}"
        field[:choices].take(16).each { |choice| rows << "      Choice: #{__help_value(choice)}" }
        rows << '      More choices: use Toyoterm.command_info' if field[:choices].length > 16
      end
      rows << "    Dynamic completion: #{field[:dynamic]}, require match: #{field[:require_match]}"
      rows << "    History: #{__help_value(field[:history])}" if field[:history]
    end
    advice = command_advice(info[:name])
    rows << "Advice: #{advice.length}"
    advice.take(32).each { |item| rows << "  Advice: #{item[:kind]} (registration #{item[:id]})" }
    rows << '  More advice: use Toyoterm.command_advice' if advice.length > 32
    if definition
      rows << "Definition: #{__help_value(definition[:source])}:#{definition[:line] || '(line unavailable)'}"
    else
      rows << 'Definition: unavailable'
    end
    rows.freeze
  end

  def self.__show_command_help(entry, context, on_definition)
    name = entry[2][:name]
    raise ArgumentError, "command changed while help was open: #{name}" unless @user_commands[name].equal?(entry)
    [context.workspace, context.window, context.tab, context.pane].each(&:validate!)
    definition = command_definition(name)
    rows = __command_help_items(entry[2], context, definition)
    definition_row = definition && rows.last
    select(title: 'Command help', items: rows) do |row|
      next unless row && row == definition_row && on_definition
      raise ArgumentError, "command changed while help was open: #{name}" unless @user_commands[name].equal?(entry)
      [context.workspace, context.window, context.tab, context.pane].each(&:validate!)
      on_definition.call(definition, context)
    end
    nil
  end

  def self.command_help(name = nil, &on_definition)
    context = @command_contexts.last || __task_context
    unless name.nil?
      name = __identifier(name, 'user command name')
      entry = @user_commands[name] || raise(ArgumentError, "undefined user command: #{name}")
      return __show_command_help(entry, context, on_definition)
    end
    entries = commands.map { |command| @user_commands[command] }
    return nil if entries.empty?
    labels = entries.each_with_index.map do |entry, index|
      info = entry[2]
      "#{index + 1}. #{__help_value(info[:name])} — #{__help_value(info[:description])}"
    end
    select(title: 'Command help', items: labels) do |label|
      next if label.nil?
      __show_command_help(entries[labels.index(label)], context, on_definition)
    end
    nil
  end
end
