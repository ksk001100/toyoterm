module Toyoterm
  class ActionCall
    attr_reader :name, :argument, :context
    def initialize(name, argument, context)
      @name, @argument, @context = name.freeze, argument && argument.freeze, context
      freeze
    end
  end

  @action_advice = {}
  @action_depth = 0

  def self.__action_name(name)
    name = __identifier(name, 'action name').downcase
    raise ArgumentError, "unsupported action: #{name}" unless StaticBinding::ACTIONS.key?(name.to_sym)
    name.freeze
  end

  def self.actions
    StaticBinding::ACTIONS.keys.map(&:to_s).map(&:freeze).sort.freeze
  end

  def self.action_info(name)
    name = __identifier(name, 'action name').downcase
    return nil unless StaticBinding::ACTIONS.key?(name.to_sym)
    choices = StaticBinding::ACTIONS[name.to_sym]
    { name: name.freeze, kind: :action, description: name.tr('_', ' ').freeze,
      argument_required: !choices.nil?, arguments: (choices || []).dup.freeze,
      scope: GLOBAL_ACTIONS.include?(name.to_sym) ? :application : :pane,
      advice: action_advice(name) }.freeze
  end

  def self.action_keys(name)
    name = __action_name(name)
    key_bindings.select { |_key, info| info[:action].to_s == name }.keys.sort.freeze
  end

  def self.advise_action(name, kind: :before, &block)
    raise ArgumentError, 'action advice requires a block' unless block
    name = __action_name(name)
    raise ArgumentError, 'advice kind must be :before, :after, or :around' unless [:before, :after, :around].include?(kind)
    @registration_id += 1
    location = block.source_location
    source = @current_source_path || @current_source_label || (location && location[0])
    info = { id: @registration_id, kind: kind, source: source && source.dup.freeze, line: location && location[1] }.freeze
    (@action_advice[name] ||= []) << [@registration_id, kind, block, info].freeze
    Registration.new(:action_advice, name, @registration_id)
  end

  def self.action_advice(name)
    name = __action_name(name)
    (@action_advice[name] || []).map { |entry| entry[3] }.freeze
  end

  def self.__action_advice_count
    @advised_action_names = @action_advice.keys.sort
    @advised_action_names.length
  end
  def self.__action_advice_name(index); @advised_action_names[index]; end

  def self.__wrap_action_advice(callback, operation, name, context)
    proc do |argument|
      active, called = true, false
      proceed = proc do |replacement = argument|
        raise RuntimeError, 'action continuation is no longer active' unless active
        raise RuntimeError, 'action continuation was already called' if called
        called = true
        _name, replacement = __normalize_action(name, replacement)
        operation.call(replacement)
      end
      begin
        callback.call(proceed, ActionCall.new(name, argument, context))
      ensure
        active = false
      end
    end
  end

  def self.__invoke_native_action(name, argument, workspace, window, tab, pane)
    context = CallbackContext.new(Pane.new(pane), Workspace.new(workspace), MuxWindow.new(window), Tab.new(tab))
    if name == 'user_command'
      call_command(argument, context: context)
    else
      __queue_action(name, argument, context)
    end
  end

  def self.operations
    native = actions.map { |name| action_info(name) }
    ruby = commands.map { |name| command_info(name).merge(kind: :command, argument_required: command_info(name)[:arguments].any? { |field| field[:required] && !field.key?(:default) }).freeze }
    (native + ruby).sort_by { |entry| [entry[:name], entry[:kind].to_s] }.freeze
  end

  def self.invoke_operation(kind, name, argument: nil, arguments: {}, context: nil)
    context = @command_contexts.last || CallbackContext.new(current_pane) if context.nil?
    raise TypeError, 'operation context must be a CallbackContext' unless context.is_a?(CallbackContext)
    case kind
    when :action
      raise ArgumentError, 'native actions use argument:, not command arguments' unless arguments.is_a?(Hash) && arguments.empty?
      context.actions.action(name, argument)
    when :command
      raise ArgumentError, 'Ruby commands do not accept an operation argument' unless argument.nil?
      call_command(name, context: context, arguments: arguments)
    else raise ArgumentError, 'operation kind must be :action or :command'
    end
  end

  def self.operation_palette
    entries = operations
    labels = entries.map { |info| "#{info[:kind]}: #{info[:name]}" }
    commands_at_open = @user_commands.dup
    context = @command_contexts.last || CallbackContext.new(current_pane)
    select(title: 'Operations', items: labels) do |label|
      next if label.nil?
      info = entries[labels.index(label)]
      if info[:kind] == :command && !@user_commands[info[:name]].equal?(commands_at_open[info[:name]])
        raise ArgumentError, "command changed while palette was open: #{info[:name]}"
      end
      if info[:kind] == :command
        prompt_command(info[:name], context: context)
      elsif info[:argument_required]
        read_string(title: "#{info[:name]} argument", completions: info[:arguments].map(&:to_s), require_match: true) do |value|
          invoke_operation(:action, info[:name], argument: value, context: context) unless value.nil?
        end
      else
        invoke_operation(info[:kind], info[:name], context: context)
      end
    end
    nil
  end

  class << self
    alias __queue_action_unadvised __queue_action
    alias __before_actions_checkpoint __registration_checkpoint
    alias __before_actions_rollback __rollback_registrations
    alias __before_actions_remove __remove_registration
    alias __before_actions_active __registration_active?

    def __queue_action(name, argument, context)
      name, argument = __normalize_action(name, argument)
      advice = (@action_advice[name] || []).dup
      return __queue_action_unadvised(name, argument, context) if advice.empty?
      [context.workspace, context.window, context.tab, context.pane].each(&:validate!) unless GLOBAL_ACTIONS.include?(name.to_sym)
      raise RuntimeError, 'action nesting exceeds 64 calls' if @action_depth >= 64
      checkpoint = __callback_checkpoint
      @action_depth += 1
      begin
        call = ActionCall.new(name, argument, context)
        advice.each { |entry| entry[2].call(call) if entry[1] == :before }
        operation = proc { |value| __queue_action_unadvised(name, value, context) }
        advice.reverse_each { |entry| operation = __wrap_action_advice(entry[2], operation, name, context) if entry[1] == :around }
        operation.call(argument)
        advice.each { |entry| entry[2].call(call) if entry[1] == :after }
      rescue => error
        __rollback_callback(checkpoint)
        raise error
      ensure
        @action_depth -= 1
      end
      nil
    end

    def __registration_checkpoint
      [__before_actions_checkpoint, @action_advice.transform_values(&:dup)]
    end
    def __rollback_registrations(checkpoint)
      __before_actions_rollback(checkpoint[0])
      @action_advice = checkpoint[1]
    end
    def __remove_registration(kind, name, id)
      return __before_actions_remove(kind, name, id) unless kind == :action_advice
      entries = @action_advice[name]
      return false unless entries
      before = entries.length
      entries.delete_if { |entry| entry[0] == id }
      @action_advice.delete(name) if entries.empty?
      before != entries.length
    end
    def __registration_active?(kind, name, id)
      return __before_actions_active(kind, name, id) unless kind == :action_advice
      (@action_advice[name] || []).any? { |entry| entry[0] == id }
    end
  end
end
