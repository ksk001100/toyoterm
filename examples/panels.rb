# Alt+M opens a live, read-only terminal monitor. Type to filter its lines;
# Enter/Escape/Ctrl+G closes it. The next timer tick releases the mode/resources.
display = proc { |value| value.to_s.each_char.take(512).join.inspect }
lines = proc do |context|
  ["Pane: #{context.pane.id}", "Title: #{display.call(context.pane.title)}",
   "Directory: #{display.call(context.pane.cwd)}",
   "Command running: #{context.pane.command_running?}", "Updated: #{Time.now}"]
end
monitor_keys = Toyoterm.keymap(:terminal_monitor) {}
monitor = Toyoterm.mode(:terminal_monitor, keymap: monitor_keys) do |enabled, context|
  if enabled
    panel = Toyoterm.show_panel(title: 'Terminal monitor', lines: lines.call(context))
    Toyoterm.every(1) do |_timer, tick|
      if panel.active?
        panel.update(lines: lines.call(tick))
      else
        monitor.disable(pane: tick.pane)
      end
    end
  end
end

Toyoterm.command(:terminal_monitor, description: 'Show a live terminal snapshot panel') do |context|
  monitor.disable(pane: context.pane) if monitor.enabled?(pane: context.pane)
  monitor.enable(pane: context.pane)
end
Toyoterm.configure { |config| config.keys.alt('m').command(:terminal_monitor) }
