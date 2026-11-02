# Optional manual key-to-frame companion to scripts/performance-baseline.py.
# Run with TOYOTERM_LOG=warn,toyoterm::perf=trace. Use the same action for both keys.
Toyoterm.configure do |config|
  config.font.family = "monospace"
  config.font.size = 14
  config.scrollback_lines = 100000
  config.keys.key("CTRL+SHIFT+F11").toggle_zoom
  config.keys.key("CTRL+SHIFT+F12").run do |context|
    context.actions.toggle_zoom
  end
end
Toyoterm.on(:bell) { |event| nil }
