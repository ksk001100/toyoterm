module Toyoterm
  class Keymap
    attr_reader :name, :id, :parent

    def initialize(name, id, parent)
      @name, @id, @parent = name, id, parent
      @registry = Config.new
    end

    def keys(&block)
      @registry.keys(&block)
    end

    def bindings
      result = @parent ? @parent.bindings.dup : {}
      own = @registry.__key_binding_info.transform_values do |info|
        info.merge(keymap: @name, keymap_id: @id).freeze
      end
      Toyoterm.__merge_key_layer(result, own)
      result.keys.sort.inject({}) { |sorted, key| sorted[key] = result[key]; sorted }.freeze
    end

    def activate(pane: nil)
      Toyoterm.__activate_keymap(self, pane, 300)
    end

    def __checkpoint
      @registry.__registration_checkpoint
    end

    def __restore(checkpoint)
      @registry.__rollback_registrations(checkpoint)
    end

    def __invoke(key, pane)
      @registry.__trigger_binding(key, pane)
    end
  end

  class Mode
    attr_reader :name, :keymap, :kind

    def initialize(name, keymap, kind, callback)
      @name, @keymap, @kind, @callback = name, keymap, kind, callback
      freeze
    end

    def enable(pane: Toyoterm.current_pane)
      Toyoterm.__change_mode(self, pane, true)
    end

    def disable(pane: Toyoterm.current_pane)
      Toyoterm.__change_mode(self, pane, false)
    end

    def enabled?(pane: Toyoterm.current_pane)
      Toyoterm.__mode_enabled?(self, pane)
    end

    def __notify(enabled, context)
      @callback.call(enabled, context) if @callback
    end
  end

  class Pane
    def enable_mode(name)
      Toyoterm.mode(name).enable(pane: self)
    end

    def disable_mode(name)
      Toyoterm.mode(name).disable(pane: self)
    end

    def modes
      Toyoterm.active_modes(pane: self)
    end
  end

  @keymaps = {}
  @keymap_activations = {}
  @modes = {}
  @mode_instances = {}
  @mode_owners = []

  def self.keymap(name, parent: nil, &block)
    name = __identifier(name, "keymap name")
    unless block
      raise ArgumentError, "parent requires a keymap definition block" if parent
      return @keymaps[name] || raise(ArgumentError, "undefined keymap: #{name}")
    end
    raise ArgumentError, "duplicate keymap: #{name}" if @keymaps.key?(name)
    parent = keymap(parent) if parent.is_a?(String) || parent.is_a?(Symbol)
    __validate_keymap(parent) if parent
    @registration_id += 1
    map = Keymap.new(name, @registration_id, parent)
    checkpoint = __callback_checkpoint
    begin
      map.keys(&block)
    rescue => error
      __rollback_callback(checkpoint)
      raise error
    end
    @keymaps[name] = map
    map
  end

  def self.keymaps
    @keymaps.keys.sort.freeze
  end

  def self.mode(name, keymap: nil, kind: :minor, &block)
    name = __identifier(name, "mode name")
    if keymap.nil?
      raise ArgumentError, "mode definition requires a keymap" if block
      return @modes[name] || raise(ArgumentError, "undefined mode: #{name}")
    end
    raise ArgumentError, "duplicate mode: #{name}" if @modes.key?(name)
    unless [:major, :minor].include?(kind)
      raise ArgumentError, "mode kind must be :major or :minor"
    end
    map = keymap.is_a?(Keymap) ? keymap : self.keymap(keymap)
    __validate_keymap(map)
    @modes[name] = Mode.new(name, map, kind, block)
  end

  def self.modes
    @modes.keys.sort.freeze
  end

  def self.active_modes(pane: Toyoterm.current_pane)
    scope = __key_scope(pane)
    @mode_instances.keys.select { |key| key[1] == scope }.map { |key| key[0] }.sort.freeze
  end

  def self.__key_scope(pane)
    return nil if pane.nil?
    raise TypeError, "keymap scope must be a Pane or nil" unless pane.is_a?(Pane)
    pane.validate!
    pane.id
  end

  def self.__validate_keymap(map)
    unless map.is_a?(Keymap) && @keymaps[map.name].equal?(map)
      raise ArgumentError, "keymap is not registered in this configuration"
    end
  end

  def self.__activate_keymap(map, pane, priority, track = true)
    __validate_keymap(map)
    scope = __key_scope(pane)
    raise RuntimeError, "Pane-local keymaps require a runtime Pane" if scope == 0
    @registration_id += 1
    id = @registration_id
    @keymap_activations[id] = [map, scope, priority].freeze
    begin
      __effective_key_bindings(scope)
    rescue => error
      @keymap_activations.delete(id)
      raise error
    end
    Registration.new(:keymap, map.name, id, track)
  end

  def self.__mode_enabled?(mode, pane)
    @mode_instances.key?([mode.name, __key_scope(pane)])
  end

  def self.__change_mode(mode, pane, enabled)
    scope = __key_scope(pane)
    raise ArgumentError, "major modes require a Pane" if scope.nil? && mode.kind == :major
    key = [mode.name, scope].freeze
    return false if @mode_instances.key?(key) == enabled
    raise RuntimeError, "recursive mode transition: #{mode.name}" if @mode_owners.include?(key)
    checkpoint = __callback_checkpoint
    context = CallbackContext.new(pane || current_pane)
    begin
      if enabled
        if mode.kind == :major
          @mode_instances.keys.dup.each do |other|
            if other[1] == scope && @modes[other[0]].kind == :major
              __change_mode(@modes[other[0]], pane, false)
            end
          end
        end
        activation = __activate_keymap(mode.keymap, pane, mode.kind == :major ? 100 : 200, false)
        @mode_instances[key] = [activation, []]
      end
      @mode_owners << key
      begin
        __with_task_owner(key) { mode.__notify(enabled, context) }
      ensure
        @mode_owners.pop
      end
      unless enabled
        activation, resources = @mode_instances.delete(key)
        resources.reverse_each(&:remove)
        activation.remove
      end
      true
    rescue => error
      __rollback_callback(checkpoint)
      raise error
    end
  end

  def self.__track_registration(registration)
    owner = __task_owner
    if owner && @mode_instances[owner]
      resources = @mode_instances[owner][1]
      resources.delete_if { |resource| !resource.active? }
      resources << registration
    end
    nil
  end

  # Higher layers mask exact bindings and conflicting ancestor/descendant
  # bindings, but unrelated suffixes of a shared prefix continue to fall back.
  def self.__merge_key_layer(result, layer)
    layer.each do |key, entry|
      result.delete_if do |other, _value|
        other == key || other.start_with?("#{key} ") || key.start_with?("#{other} ")
      end
      result[key] = entry
    end
    result
  end

  def self.__effective_key_bindings(scope)
    result = @config.__key_binding_info.dup
    globals, locals = @keymap_activations.to_a.partition { |_id, entry| entry[1].nil? }
    layers = globals.sort_by { |id, entry| [entry[2], id] }
    layers += locals.select { |_id, entry| entry[1] == scope }.sort_by { |id, entry| [entry[2], id] } unless scope.nil?
    layers.each_with_index do |(_id, entry), index|
      layer = entry[0].bindings.transform_values { |info| info.merge(layer: index + 1).freeze }
      __merge_key_layer(result, layer)
    end
    result.keys.sort.inject({}) { |sorted, key| sorted[key] = result[key]; sorted }.freeze
  end

  def self.key_bindings(pane: Toyoterm.current_pane)
    __effective_key_bindings(__key_scope(pane))
  end

  def self.describe_key(key, pane: Toyoterm.current_pane)
    name = key.is_a?(Array) ? __key_sequence(key) : __string(key, "key binding").upcase
    key_bindings(pane: pane)[name]
  end

  def self.__prune_keymap_scopes
    closed = @mode_instances.keys.select { |key| !__key_scope_alive?(key[1]) }
    closed.each do |key|
      activation, resources = @mode_instances.delete(key)
      resources.reverse_each(&:remove)
      activation.remove
    end
    @keymap_activations.delete_if { |_id, entry| !__key_scope_alive?(entry[1]) }
  end

  def self.__key_scope_alive?(scope)
    scope.nil? || __handle_valid?(:pane, scope)
  end

  def self.__key_scope_count
    __prune_keymap_scopes
    return 0 if @keymap_activations.empty?
    @key_scopes = [nil] + @keymap_activations.values.map { |entry| entry[1] }.compact.uniq.sort
    @key_scopes.length
  end

  def self.__select_key_scope(index)
    @selected_key_scope = @key_scopes[index]
    @selected_key_entries = __effective_key_bindings(@selected_key_scope).values
    @selected_key_scope.nil? ? "global" : @selected_key_scope.to_s
  end

  def self.__key_scope_entry_count
    @selected_key_entries.length
  end

  def self.__key_scope_entry(index, field)
    @selected_key_entries[index][field.to_sym]
  end

  def self.__invoke_keymap_binding(id, key, pane)
    map = @keymaps.values.find { |candidate| candidate.id == id }
    raise ArgumentError, "undefined keymap ID: #{id}" unless map
    checkpoint = __callback_checkpoint
    begin
      map.__invoke(key, pane)
    rescue => error
      __rollback_callback(checkpoint)
      raise error
    end
  end

  class << self
    alias __core_registration_checkpoint __registration_checkpoint
    alias __core_rollback_registrations __rollback_registrations
    alias __core_remove_registration __remove_registration
    alias __core_registration_active? __registration_active?

    def __registration_checkpoint
      maps = @keymaps.values.map { |map| [map, map.__checkpoint] }
      instances = @mode_instances.transform_values { |entry| [entry[0], entry[1].dup] }
      [__core_registration_checkpoint, @keymaps.dup, maps,
       @keymap_activations.dup, @modes.dup, instances]
    end

    def __rollback_registrations(checkpoint)
      __core_rollback_registrations(checkpoint[0])
      @keymaps = checkpoint[1]
      checkpoint[2].each { |map, state| map.__restore(state) }
      @keymap_activations, @modes, @mode_instances = checkpoint[3], checkpoint[4], checkpoint[5]
    end

    def __remove_registration(kind, name, id)
      return __core_remove_registration(kind, name, id) unless kind == :keymap
      !@keymap_activations.delete(id).nil?
    end

    def __registration_active?(kind, name, id)
      return __core_registration_active?(kind, name, id) unless kind == :keymap
      @keymap_activations.key?(id)
    end
  end
end
