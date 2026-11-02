module Toyoterm
  @prompt_histories = {}
  @prompt_runtime_ready = false
  @prepared_prompt_result = nil
  @completion_result = nil

  class PromptLifetime
    def initialize(id); @id = id; freeze; end
    def active?; Toyoterm.__prompt_active?(@id); end
    def remove; Toyoterm.__close_prompt(@id); end
  end

  def self.__prompt_active?(id); @select_callbacks.key?(id); end
  def self.__register_prompt_lifetime(id); __track_registration(PromptLifetime.new(id)); end
  def self.__close_prompt(id)
    return false unless @select_callbacks.delete(id)
    __queue_command(:close_prompt, 0, id.to_s)
    true
  end

  def self.__begin_prompt_request
    @prompt_runtime_ready = true
  end

  def self.__prompt_text(value, label, limit = 4096)
    text = __string(value, label, false, true)
    raise ArgumentError, "#{label} exceeds #{limit} bytes" if text.bytesize > limit
    raise ArgumentError, "#{label} must be UTF-8" unless text.unpack('U*').pack('U*') == text
    raise ArgumentError, "#{label} contains controls" if text.unpack('U*').any? { |c| c < 32 || (127..159).include?(c) }
    text
  end

  def self.prompt_history(name)
    name = __identifier(name, 'prompt history name')
    (@prompt_histories[name] || []).dup.freeze
  end

  def self.clear_prompt_history(name)
    !@prompt_histories.delete(__identifier(name, 'prompt history name')).nil?
  end

  def self.__prompt_candidates(completions)
    raise TypeError, 'completions must be an Array' unless completions.is_a?(Array)
    raise ArgumentError, 'at most 4096 completions are allowed' if completions.length > 4096
    values = completions.map { |value| __prompt_text(value, 'prompt completion') }.uniq.freeze
    raise ArgumentError, 'completions exceed 4 MiB' if values.inject(0) { |n, value| n + value.bytesize } > 4 * 1024 * 1024
    values
  end

  def self.read_string(title: 'Input', initial: '', completions: [], completer: nil, require_match: false, history: nil, &block)
    raise ArgumentError, 'read_string requires a block' unless block
    raise RuntimeError, 'read_string requires a runtime request' unless @prompt_runtime_ready
    raise RuntimeError, 'read_string cannot be used while loading a Ruby source' if __loading_source?
    title = __prompt_text(title, 'prompt title', 256)
    initial = __prompt_text(initial, 'prompt initial value')
    __boolean(require_match, 'require_match')
    values = __prompt_candidates(completions)
    raise TypeError, 'completer must respond to call' unless completer.nil? || completer.respond_to?(:call)
    raise ArgumentError, 'choose completions or completer' if completer && !values.empty?
    raise ArgumentError, 'require_match needs completions or a completer' if require_match && values.empty? && completer.nil?
    history = __identifier(history, 'prompt history name') unless history.nil?
    if history && !@prompt_histories.key?(history) && @prompt_histories.length >= 64
      raise ArgumentError, 'at most 64 prompt histories are allowed'
    end
    raise RuntimeError, 'an input or selection is already pending' unless @select_callbacks.empty?
    @prompt_histories[history] ||= [] if history
    @select_id += 1
    id = @select_id
    @select_callbacks[id] = [block, __task_context, values, [require_match, history, completer].freeze, __task_owner].freeze
    __register_prompt_lifetime(id)
    __queue_command(:open_prompt, 0, id.to_s, [title, values, initial, require_match, history ? @prompt_histories[history].dup : [], !completer.nil?])
    nil
  end

  def self.__current_prompt_initial; @current_command[3][2]; end
  def self.__current_prompt_require_match; @current_command[3][3]; end
  def self.__current_prompt_history_count; @current_command[3][4].length; end
  def self.__current_prompt_history(index); @current_command[3][4][index]; end
  def self.__current_prompt_dynamic; @current_command[3][5]; end

  def self.__prepare_prompt_result(id)
    return if @prepared_prompt_result && @prepared_prompt_result[0] == id
    @prepared_prompt_result = [id, @select_callbacks.delete(id)]
  end

  def self.__compute_prompt_completion(id, revision, query)
    @completion_result = nil
    entry = @select_callbacks[id]
    return false unless entry && entry[3] && entry[3][2]
    query = __prompt_text(query, 'completion query')
    checkpoint = __callback_checkpoint
    begin
      values = __with_task_owner(entry[4]) { __prompt_candidates(entry[3][2].call(query, entry[1])) }
    ensure
      # Providers compute candidate data. Host-managed mutations are discarded
      # on success as well as failure; arbitrary Ruby/I/O remains trusted code.
      __rollback_callback(checkpoint)
    end
    entry = entry.dup
    entry[2] = values
    @select_callbacks[id] = entry.freeze
    @completion_result = values
    true
  end
  def self.__completion_count; @completion_result.length; end
  def self.__completion_item(index); @completion_result[index]; end

  class << self
    alias __before_prompts_checkpoint __registration_checkpoint
    alias __before_prompts_rollback __rollback_registrations

    def __registration_checkpoint
      [__before_prompts_checkpoint, @prompt_histories.transform_values(&:dup), @select_callbacks.dup]
    end

    def __rollback_registrations(checkpoint)
      __before_prompts_rollback(checkpoint[0])
      @prompt_histories = checkpoint[1]
      @select_callbacks = checkpoint[2]
    end

    def __invoke_select_callback(id, selection)
      __prepare_prompt_result(id)
      entry = @prepared_prompt_result[1]
      @prepared_prompt_result = nil
      return false unless entry
      callback, context, values, options, owner = entry
      unless selection.nil?
        selection = __prompt_text(selection, 'prompt result') if options
        if (!options || options[0]) && !values.include?(selection)
          raise ArgumentError, options ? 'prompt result is not a completion' : 'select result is not one of the requested items'
        end
      end
      checkpoint = __callback_checkpoint
      begin
        if !selection.nil? && options && options[1]
          if !@prompt_histories.key?(options[1]) && @prompt_histories.length >= 64
            raise ArgumentError, 'at most 64 prompt histories are allowed'
          end
          history = @prompt_histories[options[1]] ||= []
          history.delete(selection)
          history << selection
          history.shift while history.length > 128
        end
        __with_task_owner(owner) { callback.call(selection, context) }
      rescue => error
        __rollback_callback(checkpoint)
        raise error
      end
      true
    end
  end
end
