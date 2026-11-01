module Toyoterm
  @transient_keymaps = {}
  @prepared_transient = nil

  class Keymap
    def activate_transient(pane: Toyoterm.current_pane, once: true)
      Toyoterm.__activate_transient(self, pane, once)
    end
  end

  def self.__activate_transient(map, pane, once)
    raise RuntimeError, 'transient keymaps require a runtime request' unless @task_runtime_ready
    raise RuntimeError, 'transient keymaps cannot be activated while loading a Ruby source' if __loading_source?
    __validate_keymap(map)
    __boolean(once, 'once')
    scope = __key_scope(pane)
    raise RuntimeError, 'Pane-local keymaps require a runtime Pane' if scope == 0
    __transient_binding_keys(map)
    raise ArgumentError, 'at most 64 transient keymaps are allowed' if @transient_keymaps.length >= 64
    @registration_id += 1
    id = @registration_id
    @transient_keymaps[id] = [map, scope, once, __task_owner].freeze
    Registration.new(:transient_keymap, map.name, id)
  end

  def self.__transient_binding_keys(map)
    keys = map.bindings.keys
    raise ArgumentError, 'transient keymap must have bindings' if keys.empty?
    if keys.any? { |key| ['ESCAPE', 'CTRL+G'].include?(key.split(' ').first) }
      raise ArgumentError, 'Escape and Ctrl+G are reserved for transient cancellation'
    end
    keys
  end

  def self.__transients_for(scope)
    globals, locals = @transient_keymaps.to_a.partition { |_id, entry| entry[1].nil? }
    locals = locals.select { |_id, entry| entry[1] == scope }
    (locals.reverse + globals.reverse)
  end

  def self.transient_keymaps(pane: Toyoterm.current_pane)
    __transients_for(__key_scope(pane)).map do |id, entry|
      { id: id, keymap: entry[0].name, pane: entry[1], once: entry[2] }.freeze
    end.freeze
  end

  def self.key_bindings(pane: Toyoterm.current_pane)
    scope = __key_scope(pane)
    top = __transients_for(scope).first
    return __effective_key_bindings(scope) unless top
    top[1][0].bindings.transform_values { |info| info.merge(transient: top[0]).freeze }.freeze
  end

  def self.__transient_count
    @selected_transients = __transients_for(@selected_key_scope)
    @selected_transients.length
  end
  def self.__transient_field(index, field)
    id, entry = @selected_transients[index]
    case field
    when :id then id
    when :once then entry[2]
    when :key_count
      @selected_transient_keys = __transient_binding_keys(entry[0])
      @selected_transient_keys.length
    end
  end
  def self.__transient_key(index); @selected_transient_keys[index]; end

  # Consumption is outside the callback transaction, just like one-shot timers.
  # A failed body must not resurrect a one-shot map already retired natively.
  def self.__prepare_transient(id, key, pane)
    entry = @transient_keymaps[id]
    entry = nil if entry && !entry[1].nil? && entry[1] != pane
    @transient_keymaps.delete(id) if entry && (key.nil? || entry[2])
    entry = nil unless __handle_valid?(:pane, pane)
    @prepared_transient = entry && [id, entry, key]
  end

  def self.__invoke_transient(pane)
    prepared, @prepared_transient = @prepared_transient, nil
    return false unless prepared
    _id, entry, key = prepared
    return true if key.nil?
    map = entry[0]
    info = map.bindings[key] || raise(ArgumentError, 'transient binding no longer exists')
    __with_task_owner(entry[3]) do
      case info[:kind]
      when :ruby
        owner = @keymaps.values.find { |candidate| candidate.id == info[:keymap_id] }
        raise ArgumentError, 'transient binding owner no longer exists' unless owner
        owner.__invoke(key, pane)
      when :command then __invoke_command(info[:argument], pane)
      else CallbackContext.new(pane).actions.action(info[:action], info[:argument])
      end
    end
    true
  end

  class << self
    alias __before_transient_checkpoint __registration_checkpoint
    alias __before_transient_rollback __rollback_registrations
    alias __before_transient_remove __remove_registration
    alias __before_transient_active __registration_active?
    alias __before_transient_prune __prune_keymap_scopes
    alias __before_transient_scope_count __key_scope_count

    def __registration_checkpoint
      [__before_transient_checkpoint, @transient_keymaps.dup]
    end
    def __rollback_registrations(checkpoint)
      __before_transient_rollback(checkpoint[0])
      @transient_keymaps = checkpoint[1]
    end
    def __remove_registration(kind, name, id)
      return __before_transient_remove(kind, name, id) unless kind == :transient_keymap
      !@transient_keymaps.delete(id).nil?
    end
    def __registration_active?(kind, name, id)
      return __before_transient_active(kind, name, id) unless kind == :transient_keymap
      @transient_keymaps.key?(id)
    end
    def __prune_keymap_scopes
      __before_transient_prune
      @transient_keymaps.delete_if { |_id, entry| !__key_scope_alive?(entry[1]) }
    end
    def __key_scope_count
      __before_transient_scope_count
      return 0 if @keymap_activations.empty? && @transient_keymaps.empty?
      @key_scopes = ([nil] + @keymap_activations.values.map { |e| e[1] } +
                     @transient_keymaps.values.map { |e| e[1] }).uniq
      @key_scopes.length
    end
  end
end
