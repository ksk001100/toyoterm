# An independently removable observer. A real library can keep its handle in
# its own module rather than introducing an implicit plugin loader.
module ExampleLaunchObserver
  REGISTRATION = Toyoterm.add_hook(:terminal_launch_requested, label: :pane_badge) do |path, context|
    context.pane.badge = 'Requested: ' + File.basename(path)
  end
end
