# Alt+L collects a label and optional zoom action for the originating terminal Pane.
# The same command can be invoked from either palette or from another Ruby library.
# Long Japanese/emoji labels keep their cursor visible as the input scrolls.
# IME candidate anchoring follows that input cursor, including preedit.
Toyoterm.command(:label_pane, description: 'Label this terminal Pane', arguments: [
  { name: :label, prompt: 'Pane label', history: :pane_labels },
  { name: :zoom, type: :boolean, prompt: 'Toggle Pane zoom?', default: false }
]) do |context, values|
  context.pane.badge = values['label']
  context.actions.toggle_zoom if values['zoom']
end

Toyoterm.configure do |config|
  config.keys.alt('l').command(:label_pane)
  config.keys.alt('o').run { Toyoterm.operation_palette }
end

# Programmatic invocation validates values and does not open prompts:
# Toyoterm.call_command(:label_pane, arguments: { label: 'Build', zoom: false })
