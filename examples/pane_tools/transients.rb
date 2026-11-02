# Alt+S: choose one split direction. Alt+R: keep using Pane operations until
# Escape, Ctrl+G, or an unassigned key (that key is consumed).
split_keys = Toyoterm.keymap(:choose_split) do
  key('h').split(:left)
  key('j').split(:down)
  key('k').split(:up)
  key('l').split(:right)
end

pane_keys = Toyoterm.keymap(:pane_tools) do
  key('h').activate_pane(:left)
  key('j').activate_pane(:down)
  key('k').activate_pane(:up)
  key('l').activate_pane(:right)
  key('z').toggle_zoom
end

Toyoterm.command(:choose_split, description: 'Choose one terminal split direction') do
  split_keys.activate_transient
end
Toyoterm.command(:pane_tools, description: 'Repeat terminal Pane operations') do
  # Global scope keeps this map active while focus moves between Panes.
  pane_keys.activate_transient(pane: nil, once: false)
end

Toyoterm.configure do |config|
  config.keys.alt('s').command(:choose_split)
  config.keys.alt('r').command(:pane_tools)
end
