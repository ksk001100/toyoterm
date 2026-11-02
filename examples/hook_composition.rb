# Alt+P composes independently required directory, policy and observer libraries.
raise 'hook strategies unavailable' unless Toyoterm.supports?(:hook_strategies)
require_relative 'hook_libraries/directory_provider'
require_relative 'hook_libraries/launch_policy'
require_relative 'hook_libraries/launch_observer'

Toyoterm.command(:project_terminal, description: 'Open a terminal using library-provided directory policy') do |context|
  path = Toyoterm.run_hook_until_success(:terminal_directory, context)
  unless path && Toyoterm.run_hook_until_failure(:allow_terminal_directory, path, context)
    Toyoterm.log(:info, 'Terminal directory request declined')
    next
  end
  context.pane.split(:right, cwd: path)
  # This observes a queued request, before native PTY creation completes.
  Toyoterm.run_hook(:terminal_launch_requested, path, context)
end

Toyoterm.configure do |config|
  config.keys.alt('p').command(:project_terminal)
end

# Runtime inspection or removal:
# Toyoterm.hook_handlers(:terminal_directory)
# ExampleLaunchObserver::REGISTRATION.remove
