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
    ctrl_shift("p").command_palette
    leader("g").command(:git_status)
  end
end

Toyoterm.command :git_status do |context|
  context.pane.send_text("git status\n")
end

Toyoterm.status(interval: 1.0) do |context|
  [context.workspace.name, context.pane.cwd].compact.join(" | ")
end
