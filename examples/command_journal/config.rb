require_relative '../shared/keys'

# Keep only 50 completed commands in memory, never shell input or screen text.
started = {}
entries = []
Toyoterm.on(:command_started) do |event|
  next unless event.pane.valid?
  started[event.pane.id] = Toyoterm.monotonic_time
end
Toyoterm.on(:pane_closed) { |event| started.delete(event.pane.id) }
Toyoterm.on(:command_finished) do |event|
  next unless event.pane.valid?
  since = started.delete(event.pane.id)
  elapsed = since ? '%.2fs' % (Toyoterm.monotonic_time - since) : 'duration unknown'
  cwd = event.pane.cwd.to_s.each_char.take(160).join.inspect
  status = event.exit_status.nil? ? '?' : event.exit_status.to_s
  now = Time.now
  clock = '%02d:%02d:%02d' % [now.hour, now.min, now.sec]
  entries << "#{clock} | pane #{event.pane.id} | exit #{status} | #{elapsed} | #{cwd}"
  entries.shift while entries.length > 50
end

Toyoterm.command(:recipe_journal, description: 'Review the last 50 command results') do
  lines = entries.empty? ? ['No completed commands. Enable shell integration, then run a command.'] : entries.reverse
  Toyoterm.show_panel(title: 'Command journal (newest first)', lines: lines)
end
Toyoterm.command(:recipe_clear_journal, description: 'Forget completed command results') do
  entries.clear
end
Toyoterm.configure do |config|
  config.keys.alt('j').command(:recipe_journal)
  config.keys.alt('k').command(:recipe_clear_journal)
  config.window.bar :bottom do |bar|
    bar.section(:left) { |section| section.add { "Journal: #{entries.length}/50 results" } }
    bar.section(:right) { |section| section.add('Alt+J: review | Alt+K: clear') }
  end
end
