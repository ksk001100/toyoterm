# Application policy, evaluated by the command before queuing its launch.
module ExampleLaunchPolicy
  REGISTRATION = Toyoterm.add_hook(:allow_terminal_directory, label: :existing_directory) do |path, _context|
    path.is_a?(String) && File.directory?(path)
  end
end
