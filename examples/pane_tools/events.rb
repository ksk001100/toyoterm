# Every prepared/resized Pane shows its actual terminal grid dimensions.
# Alt+D splits the current Pane; resizing the window/splits updates each badge.
raise 'pane lifecycle events unavailable' unless Toyoterm.supports?(:pane_lifecycle_events)

[:pane_ready, :pane_resized].each do |name|
  Toyoterm.on(name) do |event|
    # A short-lived child can close before the asynchronous notification runs.
    next unless event.pane.valid?
    event.pane.badge = "#{event.columns} cols x #{event.rows} rows"
  end
end
Toyoterm.configure { |config| config.keys.alt('d').split(:right) }
