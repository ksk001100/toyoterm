module Toyoterm
  @resource_scope_id = 0
  @resource_scopes = {}
  @pruning_resource_scopes = false

  class ResourceScope
    attr_reader :id, :name, :pane
    def initialize(id, name, pane)
      @id, @name, @pane, @resources = id, name, pane, []
      freeze
    end
    def active?; Toyoterm.__resource_scope_active?(self); end
    def close; Toyoterm.__close_resource_scope(self); end
    alias remove close
    def resources
      return [].freeze unless active?
      @resources.delete_if { |resource| !resource.active? }
      @resources.dup.freeze
    end
    def within(&block)
      raise ArgumentError, 'resource scope within requires a block' unless block
      raise RuntimeError, 'resource scope is closed' unless active?
      @pane.validate! if @pane
      checkpoint = Toyoterm.__callback_checkpoint
      begin
        Toyoterm.__with_task_owner(self) { block.call(self) }
      rescue => error
        Toyoterm.__rollback_callback(checkpoint)
        raise error
      end
    end
    def wrap(&block)
      raise ArgumentError, 'resource scope wrap requires a block' unless block
      proc do |*arguments, **keywords|
        next nil unless active?
        within { keywords.empty? ? block.call(*arguments) : block.call(*arguments, **keywords) }
      end
    end
    def __resources; @resources; end
    def __pane_id; @pane && @pane.id; end
  end

  def self.resource_scope(name: nil, pane: nil, &block)
    name = __identifier(name, 'resource scope name') unless name.nil?
    target = __key_scope(pane)
    raise RuntimeError, 'Pane-local resource scopes require a runtime Pane' if target == 0
    raise ArgumentError, 'at most 256 resource scopes may be active' if @resource_scopes.length >= 256
    checkpoint = __callback_checkpoint
    begin
      scope = ResourceScope.new(@resource_scope_id += 1, name, pane)
      @resource_scopes[scope.id] = scope
      __track_registration(scope)
      scope.within(&block) if block
      scope
    rescue => error
      __rollback_callback(checkpoint)
      raise error
    end
  end

  def self.resource_scopes
    __prune_resource_scopes
    @resource_scopes.keys.sort.map { |id| @resource_scopes[id] }.freeze
  end
  def self.__resource_scope_active?(scope)
    @resource_scopes[scope.id].equal?(scope)
  end
  def self.__close_resource_scope(scope)
    return false unless __resource_scope_active?(scope)
    checkpoint = __callback_checkpoint
    begin
      pending = [scope]
      until pending.empty?
        resource = pending.pop
        if resource.is_a?(ResourceScope)
          next unless __resource_scope_active?(resource)
          @resource_scopes.delete(resource.id)
          pending.concat(resource.__resources)
          resource.__resources.clear
        elsif @pruning_resource_scopes && resource.is_a?(Registration) && resource.kind == :mode_activation
          key = __scope_mode_instance(resource.name, resource.id)
          if key
            activation, resources = @mode_instances.delete(key)
            pending << activation
            pending.concat(resources)
          end
        else
          resource.remove
        end
      end
      true
    rescue => error
      __rollback_callback(checkpoint)
      raise error
    end
  end
  def self.__prune_resource_scopes
    previous = @pruning_resource_scopes
    @pruning_resource_scopes = true
    begin
      @resource_scopes.values.dup.each do |scope|
        scope.close if scope.pane && !scope.pane.valid?
      end
    ensure
      @pruning_resource_scopes = previous
    end
  end

  def self.__scope_mode_instance(name, id)
    @mode_instances.keys.find { |key| key[0] == name && @mode_instances[key][0].id == id }
  end

  class << self
    alias __before_scopes_owner_pane __owner_pane
    alias __before_scopes_track __track_registration
    alias __before_scopes_checkpoint __registration_checkpoint
    alias __before_scopes_rollback __rollback_registrations
    alias __before_scopes_prune __prune_keymap_scopes
    alias __before_scopes_change_mode __change_mode
    alias __before_scopes_remove __remove_registration
    alias __before_scopes_active __registration_active?

    def __owner_pane(owner)
      owner.is_a?(ResourceScope) ? owner.__pane_id : __before_scopes_owner_pane(owner)
    end
    def __change_mode(mode, pane, enabled)
      owner = __task_owner
      changed = __before_scopes_change_mode(mode, pane, enabled)
      if enabled && changed && owner.is_a?(ResourceScope)
        entry = @mode_instances[[mode.name, __key_scope(pane)]]
        Registration.new(:mode_activation, mode.name, entry[0].id)
      end
      changed
    end
    def __remove_registration(kind, name, id)
      return __before_scopes_remove(kind, name, id) unless kind == :mode_activation
      key = __scope_mode_instance(name, id)
      return false unless key
      __change_mode(@modes[name], key[1] ? Pane.new(key[1]) : nil, false)
    end
    def __registration_active?(kind, name, id)
      return __before_scopes_active(kind, name, id) unless kind == :mode_activation
      !__scope_mode_instance(name, id).nil?
    end
    def __track_registration(registration)
      owner = __task_owner
      return __before_scopes_track(registration) unless owner.is_a?(ResourceScope)
      unless owner.active?
        registration.remove
        raise RuntimeError, 'cannot register resources in a closed scope'
      end
      resources = owner.__resources
      resources.delete_if { |resource| !resource.active? }
      resources << registration
      nil
    end
    def __registration_checkpoint
      [__before_scopes_checkpoint, @resource_scopes.dup,
       @resource_scopes.values.map { |scope| [scope, scope.__resources.dup] }]
    end
    def __rollback_registrations(checkpoint)
      __before_scopes_rollback(checkpoint[0])
      @resource_scopes.each { |id, scope| scope.__resources.clear unless checkpoint[1].key?(id) }
      @resource_scopes = checkpoint[1]
      checkpoint[2].each { |scope, resources| scope.__resources.replace(resources) }
    end
    def __prune_keymap_scopes
      previous = @pruning_resource_scopes
      @pruning_resource_scopes = true
      begin
        __before_scopes_prune
        __prune_resource_scopes
      ensure
        @pruning_resource_scopes = previous
      end
    end
  end
end
