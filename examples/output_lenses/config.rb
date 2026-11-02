require_relative '../shared/keys'
require_relative 'lenses'
require_relative 'todos'

Toyoterm.command(:recipe_lenses, description: 'Inspect visible terminal output through a Ruby lens') do |context|
  lenses = Toyoterm.run_hook(:recipe_output_lenses)
  by_name = {}
  lenses.each { |lens| by_name[lens[:name]] = lens }
  # Capture once: later PTY output must not silently change the selection data.
  snapshot = context.pane.screen_text
  Toyoterm.select(title: 'Output lens', items: by_name.keys) do |name, origin|
    next unless name && origin.pane.valid?
    matches = by_name[name][:scan].call(snapshot).reject(&:empty?).take(200)
    # Keep raw matches for searching/copying; escape and bound only UI labels.
    targets = {}
    matches.each_with_index do |text, index|
      label = "#{index + 1}: #{text.each_char.take(180).join.inspect}"
      targets[label] = text
    end
    if targets.empty?
      Toyoterm.show_panel(title: name, lines: ['No matches in the captured visible screen.'])
      next
    end
    Toyoterm.select(title: "#{name}: choose a match", items: targets.keys) do |choice, selected|
      next unless choice && selected.pane.valid?
      text = targets[choice]
      Toyoterm.select(title: 'Use captured text', items: ['Find in scrollback', 'Copy to clipboard']) do |action, target|
        next unless action && target.pane.valid?
        if action == 'Copy to clipboard'
          Toyoterm.clipboard.write(text)
        else
          target.pane.search(text, direction: :previous)
        end
      end
    end
  end
end
Toyoterm.configure do |config|
  config.keys.alt('l').command(:recipe_lenses)
  config.window.bar :bottom do |bar|
    bar.section(:left) { |section| section.add('Ruby lenses: Diagnostics / URLs / TODOs') }
    bar.section(:right) { |section| section.add('Alt+L: capture -> extract -> find or copy') }
  end
end
