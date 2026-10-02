# Stable, non-networked fixture used by both the GUI and headless soak.
Toyoterm.configure do |config|
  config.font.family = "monospace"
  config.font.fallback = ["Noto Color Emoji"]
  config.font.size = 14
  config.colors.background = "#112233"
  config.colors.foreground = "#ddeeff"
  config.scrollback_lines = 1000
  config.default_shell = Toyoterm.platform == :windows ? "cmd.exe" : "/bin/sh"
  config.window.image do |image|
    image.path = "../crates/toyoterm-script/tests/fixtures/background.png"
    image.opacity = 0.2
  end
  config.window.bar(:bottom) do |bar|
    bar.section(:left) { |section| section.add { |context| context.workspace.name } }
  end
end
Toyoterm.command(:soak_command) { $soak_commands = ($soak_commands || 0) + 1 }
Toyoterm.on(:bell) { $soak_events = ($soak_events || 0) + 1 }
