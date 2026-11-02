# Load with: toyoterm --config /path/to/examples/command_palette.rb
Toyoterm.command(:git_status, description: "Show Git status in this pane") do |context|
  context.pane.send_text("git status\n")
end

Toyoterm.command(:project_status, description: "Run the project's status command") do
  Toyoterm.call_command(:git_status)
end

# Libraries can subscribe without replacing the command implementation.
Toyoterm.add_hook(:project_status_requested) do |context|
  Toyoterm.log(:info, "Project status requested for pane #{context.pane.id}")
end
Toyoterm.advise_command(:project_status, kind: :before) do |context|
  Toyoterm.run_hook(:project_status_requested, context)
end

Toyoterm.command(:split_right, description: "Split the current pane to the right") do |context|
  context.actions.split(:right)
end

Toyoterm.configure do |config|
  config.keys.alt("x").run { Toyoterm.command_palette }
  config.key_sequence_timeout_ms = nil
  config.keys.sequence("CTRL+X", "CTRL+S").command(:project_status)
  config.keys.sequence("CTRL+X", "?").run { Toyoterm.key_help(prefix: "CTRL+X") }
  config.keys.sequence("CTRL+H", "B").run { Toyoterm.key_help }
  config.keys.ctrl_shift("r").reload_config
end
