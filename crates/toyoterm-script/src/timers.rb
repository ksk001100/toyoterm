module Toyoterm
  class Timer
    attr_reader :id, :context, :delay, :interval
    def initialize(id, context, delay, interval)
      @id, @context, @delay, @interval = id, context, delay, interval
      freeze
    end
    def active?; Toyoterm.__timer_active?(@id); end
    def cancel; Toyoterm.__cancel_timer(@id); end
    alias remove cancel
  end

  class AsyncTask
    alias remove cancel
    alias active? pending?
    def __checkpoint; [@result, @cancelled]; end
    def __restore(state); @result, @cancelled = state; end
  end

  @timer_id = 0
  @timers = {}
  @task_runtime_ready = false
  @resource_owners = []
  @async_owners = {}
  @prepared_timer = nil
  @prepared_async = nil

  def self.__task_owner
    @resource_owners.empty? ? @mode_owners.last : @resource_owners.last
  end

  def self.__owner_pane(owner); owner && owner[1]; end

  def self.__task_context
    owner = __task_owner
    scope = __owner_pane(owner)
    if scope
      CallbackContext.new(Pane.new(scope).validate!)
    else
      CallbackContext.new(current_pane)
    end
  end

  def self.__with_task_owner(owner)
    @resource_owners << owner
    begin
      yield
    ensure
      @resource_owners.pop
    end
  end

  def self.__timer_context_valid?(context)
    context.pane.valid?
  end

  def self.__begin_task_request
    @task_runtime_ready = true
    __prune_keymap_scopes
    @timers.delete_if { |_id, entry| !__timer_context_valid?(entry[2]) }
  end

  def self.after(seconds, &block)
    __schedule_timer(seconds, false, block)
  end

  def self.every(seconds, &block)
    __schedule_timer(seconds, true, block)
  end

  def self.timers
    @timers.keys.sort.map { |id| @timers[id][0] }.freeze
  end

  def self.__schedule_timer(seconds, repeat, block, context = nil, track = true)
    raise ArgumentError, 'timer requires a block' unless block
    raise RuntimeError, 'timers require a runtime request' unless @task_runtime_ready
    raise RuntimeError, 'timers cannot be created while loading a Ruby source' if __loading_source?
    seconds = __number(seconds, 'timer delay', repeat ? 0.01 : 0, 365 * 86400)
    raise ArgumentError, 'at most 1024 timers may be active' if @timers.length >= 1024
    milliseconds = (seconds * 1000).ceil
    context ||= __task_context
    timer = Timer.new(@timer_id += 1, context, milliseconds / 1000.0, repeat ? milliseconds / 1000.0 : nil)
    @timers[timer.id] = [timer, block, context, milliseconds, repeat, __task_owner].freeze
    __track_registration(timer) if track
    timer
  end

  def self.__timer_active?(id); @timers.key?(id); end
  def self.__cancel_timer(id); !@timers.delete(id).nil?; end
  def self.__timer_count
    @timer_rows = @timers.keys.sort.map { |id| [id, @timers[id][3], @timers[id][4]] }
    @timer_rows.length
  end
  def self.__timer_field(index, field); @timer_rows[index][field]; end

  # Consumption precedes the host transaction: a failed one-shot is still spent.
  def self.__prepare_timer(id)
    @prepared_timer = @timers[id]
    @timers.delete(id) if @prepared_timer && !@prepared_timer[4]
  end
  def self.__invoke_timer
    entry, @prepared_timer = @prepared_timer, nil
    return false unless entry
    timer, callback, context, _delay, _repeat, owner = entry
    return false unless __timer_context_valid?(context)
    checkpoint = __callback_checkpoint
    begin
      __with_task_owner(owner) { callback.call(timer, context) }
    rescue => error
      __rollback_callback(checkpoint)
      raise error
    end
    true
  end

  def self.__prepare_async(id)
    @prepared_async = [id, @async_tasks.delete(id), @async_callbacks.delete(id), @async_owners.delete(id)]
  end

  def self.__invoke_async_callback(id, stdout, stderr, exit_status, launch_error = false)
    __prepare_async(id) unless @prepared_async && @prepared_async[0] == id
    _id, task, callback_entry, owner = @prepared_async
    @prepared_async = nil
    return true unless task
    result = ProcessResult.new(stdout, stderr, exit_status, launch_error ? :launch : nil)
    task.__complete(result)
    return true unless callback_entry && callback_entry[0] && !task.cancelled?
    checkpoint = __callback_checkpoint
    begin
      __with_task_owner(owner) { callback_entry[0].call(result, callback_entry[1]) }
    rescue => error
      __rollback_callback(checkpoint)
      raise error
    end
    true
  end

  class << self
    alias __before_task_spawn async_spawn
    alias __before_task_checkpoint __registration_checkpoint
    alias __before_task_rollback __rollback_registrations
    alias __before_task_cancel __cancel_async
    alias __before_task_request_rollback __rollback_async_requests

    def async_spawn(program, *args, cwd: nil, &block)
      task = __before_task_spawn(program, *args, cwd: cwd, &block)
      @async_owners[task.id] = __task_owner
      __track_registration(task)
      task
    end

    def __cancel_async(id)
      @async_owners.delete(id)
      __before_task_cancel(id)
    end

    def __rollback_async_requests(checkpoint)
      (@async_requests[checkpoint..-1] || []).each do |request|
        task = @async_tasks[request[0]]
        task.__restore([nil, true]) if task
        @async_owners.delete(request[0])
      end
      __before_task_request_rollback(checkpoint)
    end

    def __registration_checkpoint
      [__before_task_checkpoint, @timers.dup, @async_tasks.dup, @async_callbacks.dup,
       @async_requests.dup, @async_cancellations.dup, @async_owners.dup,
       @async_tasks.values.map { |task| [task, task.__checkpoint] }]
    end

    def __rollback_registrations(checkpoint)
      __before_task_rollback(checkpoint[0])
      @async_tasks.each { |id, task| task.__restore([nil, true]) unless checkpoint[2].key?(id) }
      @timers, @async_tasks, @async_callbacks = checkpoint[1], checkpoint[2], checkpoint[3]
      @async_requests, @async_cancellations, @async_owners = checkpoint[4], checkpoint[5], checkpoint[6]
      checkpoint[7].each { |task, state| task.__restore(state) }
    end
  end
end
