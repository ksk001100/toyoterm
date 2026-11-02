# Alt+H browses command metadata. Selecting a definition copies its full
# location; another trusted library can provide a different navigation handler.
Toyoterm.command(:terminal_status, description: 'Label this terminal with its shell status') do |context|
  context.pane.badge = context.pane.command_running? ? 'Running' : 'Ready'
end

Toyoterm.configure do |config|
  config.keys.alt('s').command(:terminal_status)
  config.keys.alt('h').run do
    Toyoterm.command_help do |definition, context|
      location = definition[:source]
      location += ':' + definition[:line].to_s if definition[:line]
      Toyoterm.clipboard.write(location)
      context.pane.badge = 'Definition location copied'
    end
  end
end
