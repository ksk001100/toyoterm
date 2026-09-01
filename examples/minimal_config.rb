Toyoterm.configure do |config|
  config.font do |font|
    font.family = "monospace"
    font.size = 14
    font.weight = 400
  end

  config.colors do |colors|
    colors.background = "#090b0e"
    colors.foreground = "#dce1e8"
    colors.cursor = "#f5f7fa"
    colors.selection = "#375891"
  end

  config.window.opacity = 1.0
  config.scrollback_lines = 10_000
  config.leader key: "b", mods: "CTRL", timeout: 1000

  config.bind "CTRL+SHIFT+H" do |context|
    context.pane.send_text("echo hello from toyoterm\n")
  end

  config.bind "CTRL+SHIFT+R" do
    Toyoterm.reload_config
  end

  config.keys do
    leader("v").split(:right)
    ctrl_shift("e").split(:right)
    ctrl_shift("o").activate_pane(:right)
    ctrl_shift("t").new_tab
  end
end
