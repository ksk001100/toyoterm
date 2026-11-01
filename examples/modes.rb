# Load with: toyoterm --config /path/to/examples/modes.rb
# Alt+N toggles global navigation, which stays active across Pane switches.
# Escape leaves navigation mode.
Toyoterm.keymap(:navigation_base) do
  key("h").activate_pane(:left)
  key("j").activate_pane(:down)
  key("k").activate_pane(:up)
  key("l").activate_pane(:right)
end

navigation = Toyoterm.keymap(:navigation, parent: :navigation_base) do
  key("ESCAPE").run { Toyoterm.mode(:navigation).disable(pane: nil) }
end

navigation_mode = Toyoterm.mode(:navigation, keymap: navigation) do |enabled, _context|
  Toyoterm.log(:info, enabled ? "Navigation enabled" : "Navigation disabled")
end

Toyoterm.configure do |config|
  config.keys.alt("n").run do |context|
    if navigation_mode.enabled?(pane: nil)
      navigation_mode.disable(pane: nil)
    else
      navigation_mode.enable(pane: nil)
    end
  end
  config.keys.ctrl_shift("t").new_tab
  config.keys.ctrl_shift("e").split(:right)
  config.keys.sequence("CTRL+H", "B").run { Toyoterm.key_help }
  config.keys.ctrl_shift("r").reload_config
end
