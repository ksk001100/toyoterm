# Alt+T toggles a Pane-local clock; Escape exits clock mode and cancels its timer.
# Alt+R sets a one-shot reminder five seconds later. No Ruby runs during rendering.
clock_keys = Toyoterm.keymap(:clock) do
  key('ESCAPE').run { |context| context.pane.disable_mode(:clock) }
end

Toyoterm.mode(:clock, keymap: clock_keys) do |enabled, context|
  if enabled
    now = Time.now
    context.pane.badge = '%02d:%02d:%02d' % [now.hour, now.min, now.sec]
    Toyoterm.every(1) do |_timer, tick|
      now = Time.now
      tick.pane.badge = '%02d:%02d:%02d' % [now.hour, now.min, now.sec]
    end
  else
    context.pane.badge = nil
  end
end

Toyoterm.command(:toggle_clock, description: 'Toggle this Pane clock') do |context|
  mode = Toyoterm.mode(:clock)
  if mode.enabled?(pane: context.pane)
    mode.disable(pane: context.pane)
  else
    mode.enable(pane: context.pane)
  end
end

Toyoterm.command(:reminder, description: 'Show a reminder in five seconds') do
  Toyoterm.after(5) { |_timer, context| context.pane.badge = 'Reminder' }
end

Toyoterm.on(:app_started) { |event| event.pane.enable_mode(:clock) }
Toyoterm.configure do |config|
  config.keys.alt('t').command(:toggle_clock)
  config.keys.alt('r').command(:reminder)
end
