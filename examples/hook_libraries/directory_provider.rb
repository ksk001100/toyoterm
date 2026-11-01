# Independent provider library: earlier providers may override either choice.
module ExampleDirectoryProvider
  CURRENT = Toyoterm.add_hook(:terminal_directory, priority: 0, label: :shell_directory) do |context|
    path = context.pane.cwd
    path if path && File.directory?(path)
  end

  FALLBACK = Toyoterm.add_hook(:terminal_directory, priority: 100, label: :process_directory) do |_context|
    Dir.pwd
  end
end
