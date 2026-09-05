# GPUI uses logical key bindings; physical bindings and always_on_top = true are unsupported.
# Window dimensions, decorations and resizability take effect on the next launch.
Toyoterm.configure do |config|
  config.font do |font|
    font.family = "monospace"
    # Tried in order before the platform's standard fallback fonts.
    font.fallback = ["Noto Sans Mono CJK JP", "Noto Color Emoji"]
    font.size = 14
    font.weight = 400
  end

  config.colors do |colors|
    colors.background = "#090b0e"
    colors.foreground = "#dce1e8"
    colors.cursor = "#f5f7fa"
    colors.selection = "#375891"
    colors.ansi[1] = "#ff5f56"
  end

  config.window.opacity = 1.0
  config.window.bar :bottom, interval: 1.0 do |bar|
    bar.add(:left) { |context| context.workspace.name }
    bar.add(:right) { |context| context.pane.cwd }
  end
  config.ui.padding_x = 8
  config.ui.padding_y = 8
  config.ui.line_height = 1.2857143
  config.behavior.scroll_lines = 3
  config.scrollback_lines = 10_000
  config.leader key: "b", mods: "CTRL", timeout: 1000

  config.bind "CTRL+SHIFT+H" do |context|
    context.pane.send_text("echo hello from toyoterm\n")
  end

  config.keys do
    leader("v").split(:right)
    ctrl_shift("e").split(:right)
    ctrl_shift("o").activate_pane(:right)
    ctrl_shift("t").new_tab
    ctrl_shift("r").reload_config
    leader("g").command(:git_status)
  end
end

Toyoterm.command :git_status do |context|
  context.pane.send_text("git status\n")
end
