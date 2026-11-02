module Toyoterm
  def self.with_instruction_budget(instructions:, &block)
    raise TypeError, 'instructions must be an Integer' unless instructions.is_a?(Integer)
    unless instructions >= 1 && instructions <= 1_000_000_000
      raise ArgumentError, 'instructions must be between 1 and 1000000000'
    end
    raise ArgumentError, 'with_instruction_budget requires a block' unless block
    checkpoint = __callback_checkpoint
    begin
      __host_with_instruction_budget(instructions, &block)
    rescue Exception => error
      __rollback_callback(checkpoint)
      raise error
    end
  end
end
