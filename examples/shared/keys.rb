# Shared navigation for the standalone extension recipes. No shell commands run.
Toyoterm.configure do |config|
  config.font.family = 'monospace'
  config.font.size = 14
  config.keys do
    ctrl_shift('r').reload_config
    ctrl_shift('t').new_tab
    ctrl('TAB').next_tab
    ctrl_shift('TAB').previous_tab
    ctrl_shift('e').split(:right)
    ctrl_shift('d').split(:down)
    ctrl_shift('left').activate_pane(:left)
    ctrl_shift('right').activate_pane(:right)
    ctrl_shift('up').activate_pane(:up)
    ctrl_shift('down').activate_pane(:down)
    ctrl_shift('z').toggle_zoom
    primary_shift('c').copy_selection
    primary_shift('v').paste_clipboard
  end
end
