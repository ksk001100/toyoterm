# Alt+W counts words from a terminal snapshot in cooperative batches and
# displays progress/results in a read-only panel. Close the panel to stop.
keys = Toyoterm.keymap(:screen_word_counts) {}
mode = Toyoterm.mode(:screen_word_counts, keymap: keys) do |enabled, context|
  next unless enabled
  counts = {}
  panel = Toyoterm.show_panel(title: 'Screen word counts', lines: ['Waiting…'])
  task = Toyoterm.work_each(context.pane.screen_text.each_line, name: :screen_words,
                           batch_size: 8, budget_ms: 3) do |line, _step, work|
    unless panel.active?
      work.cancel
      next
    end
    line.split.each do |word|
      counts[word] = (counts[word] || 0) + 1 unless word.empty?
    end
  end
  Toyoterm.every(0.25) do |_timer, tick|
    unless panel.active?
      mode.disable(pane: tick.pane)
      next
    end
    statistics = task.statistics
    rows = ["State: #{statistics[:state]}", "Lines processed: #{statistics[:processed]}",
            "Slices: #{statistics[:slices]}", "Budget overruns: #{statistics[:overruns]}"]
    rows << task.error.each_char.take(512).join.inspect if task.error
    counts.to_a.sort_by { |word, count| [-count, word] }.take(10).each do |word, count|
      rows << "#{word.each_char.take(48).join.inspect}: #{count}"
    end
    panel.update(lines: rows)
  end
end

Toyoterm.command(:screen_word_counts, description: 'Count terminal snapshot words cooperatively') do |context|
  mode.disable(pane: context.pane) if mode.enabled?(pane: context.pane)
  mode.enable(pane: context.pane)
end
Toyoterm.configure { |config| config.keys.alt('w').command(:screen_word_counts) }
