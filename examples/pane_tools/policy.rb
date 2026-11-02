# Alt+M toggles this Pane's policy; Alt+D requests a right split.
# With the policy enabled, splits in this Pane go down instead.
# Alt+O lists native actions and Ruby commands together.
policy_keys = Toyoterm.keymap(:split_policy) do
  key('ESCAPE').run { |context| context.pane.disable_mode(:split_policy) }
end

Toyoterm.mode(:split_policy, keymap: policy_keys) do |enabled, context|
  if enabled
    pane_id = context.pane.id
    Toyoterm.advise_action(:split, kind: :around) do |proceed, call|
      proceed.call(call.context.pane.id == pane_id ? :down : call.argument)
    end
    Toyoterm.advise_action(:close_pane, kind: :around) do |proceed, call|
      if call.context.pane.id == pane_id && call.context.pane.command_running?
        Toyoterm.log(:info, 'Pane close suppressed by split_policy while a command is running')
      else
        proceed.call
      end
    end
  end
end

Toyoterm.command(:toggle_split_policy, description: 'Toggle this Pane action policy') do |context|
  mode = Toyoterm.mode(:split_policy)
  if mode.enabled?(pane: context.pane)
    mode.disable(pane: context.pane)
  else
    mode.enable(pane: context.pane)
  end
end

Toyoterm.configure do |config|
  config.keys.alt('m').command(:toggle_split_policy)
  config.keys.alt('d').split(:right)
  config.keys.alt('o').run { Toyoterm.operation_palette }
end
