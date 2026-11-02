module Toyoterm
  @work_id = 0
  @work_tasks = {}
  @work_delivery = nil

  def self.monotonic_time; __host_monotonic_time; end

  class CooperativeTask
    attr_reader :id, :name, :context, :batch_size, :budget_ms, :interval, :error

    def initialize(id, name, context, iterator, callback, batch_size, budget_ms, interval)
      @id, @name, @context, @iterator, @callback = id, name, context, iterator, callback
      @batch_size, @budget_ms, @interval = batch_size, budget_ms, interval
      @processed, @slices, @overruns = 0, 0, 0
      @total_ms, @last_ms = 0.0, 0.0
      @terminal, @error, @running = nil, nil, false
    end

    def active?; Toyoterm.__work_active?(@id); end
    def cancel; Toyoterm.__cancel_work(@id); end
    alias remove cancel
    def state
      return @terminal if @terminal
      return :cancelled unless active?
      @running ? :running : :pending
    end
    def statistics
      { id: @id, name: @name, state: state, processed: @processed, slices: @slices,
        total_ms: @total_ms, last_ms: @last_ms, overruns: @overruns,
        batch_size: @batch_size, budget_ms: @budget_ms }.freeze
    end

    def __terminal?; !@terminal.nil?; end
    def __running?; @running; end
    def __fail(message)
      @terminal, @error = :failed, message.dup.freeze
      __release
    end
    def __references; [@iterator, @callback]; end
    def __restore_references(state); @iterator, @callback = state; end
    def __release; @iterator, @callback = nil, nil; end
    def __resume(context)
      @running = true
      @slices += 1
      started = Toyoterm.monotonic_time
      count = 0
      begin
        while active? && count < @batch_size
          break if count > 0 && (Toyoterm.monotonic_time - started) * 1000 >= @budget_ms
          begin
            item = @iterator.next
          rescue StopIteration
            @terminal = :completed
            break
          end
          @callback.call(item, context, self)
          @processed += 1
          count += 1
        end
      ensure
        @last_ms = (Toyoterm.monotonic_time - started) * 1000
        @total_ms += @last_ms
        @overruns += 1 if @last_ms > @budget_ms
        @running = false
      end
    end
  end

  def self.work_each(items, name: nil, batch_size: 32, budget_ms: 4, interval: 0.01, &block)
    raise ArgumentError, 'work_each requires a block' unless block
    raise RuntimeError, 'work_each requires a runtime request' unless @task_runtime_ready
    raise RuntimeError, 'work_each cannot be used while loading a Ruby source' if __loading_source?
    raise TypeError, 'work_each requires an object responding to each' unless items.respond_to?(:each)
    raise TypeError, 'batch_size must be an Integer' unless batch_size.is_a?(Integer)
    raise ArgumentError, 'batch_size must be within 1..4096' unless batch_size.between?(1, 4096)
    budget_ms = __number(budget_ms, 'work budget_ms', 0.1, 100)
    interval = __number(interval, 'work interval', 0.01, 60)
    name = __identifier(name, 'work name') unless name.nil?
    __prune_work
    raise ArgumentError, 'at most 256 cooperative tasks may be active' if @work_tasks.length >= 256
    task = CooperativeTask.new(@work_id += 1, name, __task_context, items.to_enum(:each), block, batch_size, budget_ms, interval)
    __schedule_work(task, 0)
    __track_registration(task)
    task
  end

  def self.work_tasks
    __prune_work
    @work_tasks.keys.sort.map { |id| @work_tasks[id][0] }.freeze
  end
  def self.__work_active?(id)
    entry = @work_tasks[id]
    !!(entry && !entry[0].__terminal? && (entry[0].__running? || entry[1].active?))
  end
  def self.__cancel_work(id)
    entry = @work_tasks.delete(id)
    return false unless entry
    entry[1].cancel
    entry[0].__release
    true
  end
  def self.__prune_work
    @work_tasks.keys.each do |id|
      task = @work_tasks[id][0]
      __cancel_work(id) unless __work_active?(id) && task.context.pane.valid?
    end
  end
  def self.__schedule_work(task, delay)
    timer = __schedule_timer(delay, false, proc do |_timer, context|
      next unless @work_tasks.key?(task.id)
      task.__resume(context)
      if task.__terminal?
        @work_tasks.delete(task.id)
        task.__release
      elsif @work_tasks.key?(task.id)
        __schedule_work(task, task.interval)
      end
    end, task.context, false)
    @work_tasks[task.id] = [task, timer].freeze
  end

  # Called after the host request commits or rolls back. Iterator/progress state
  # cannot be rewound; failed deliveries must never retry consumed input.
  def self.__finish_work_request(error)
    task, @work_delivery = @work_delivery, nil
    return false unless task && error
    __cancel_work(task.id)
    task.__fail(error)
    true
  end

  class << self
    alias __before_work_checkpoint __registration_checkpoint
    alias __before_work_rollback __rollback_registrations
    alias __before_work_begin __begin_task_request
    alias __before_work_prepare __prepare_timer
    def __registration_checkpoint
      [__before_work_checkpoint, @work_tasks.dup,
       @work_tasks.values.map { |entry| [entry[0], entry[0].__references] }]
    end
    def __rollback_registrations(checkpoint)
      __before_work_rollback(checkpoint[0])
      @work_tasks.each { |id, entry| entry[0].__release unless checkpoint[1].key?(id) }
      @work_tasks = checkpoint[1]
      checkpoint[2].each { |task, state| task.__restore_references(state) }
    end
    def __begin_task_request
      @work_delivery = nil
      __before_work_begin
      __prune_work
    end
    def __prepare_timer(id)
      @work_delivery = @work_tasks.values.find { |entry| entry[1].id == id }
      @work_delivery = @work_delivery && @work_delivery[0]
      __before_work_prepare(id)
    end
  end
end
