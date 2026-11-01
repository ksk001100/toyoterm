# Alt+I summarizes a terminal snapshot under an explicit Ruby opcode budget.
# This does not interrupt C functions or blocking I/O. No default bindings change.
Toyoterm.command(:bounded_screen_summary, description: 'Summarize terminal text with an instruction limit') do |context|
  begin
    rows = Toyoterm.with_instruction_budget(instructions: 100_000) do
      counts = {}
      context.pane.screen_text.each_line do |line|
        line.split.each { |word| counts[word] = (counts[word] || 0) + 1 }
      end
      counts.to_a.sort_by { |word, count| [-count, word] }.take(10).map do |word, count|
        "#{word.each_char.take(48).join.inspect}: #{count}"
      end
    end
  rescue Toyoterm::InstructionBudgetError
    rows = ['Summary exceeded its Ruby instruction limit.',
            'Try smaller work units or the cooperative_work.rb example.']
  end
  Toyoterm.show_panel(title: 'Bounded screen summary', lines: rows)
end
Toyoterm.configure { |config| config.keys.alt('i').command(:bounded_screen_summary) }
