require_relative '../shared/keys'
# Preserve the components' standalone controls; resolve their shared bindings
# at the composition boundary instead of registering a chord twice.
Toyoterm.configure do |config|
  ['CTRL+SHIFT+T', 'CTRL+SHIFT+E', 'CTRL+SHIFT+R'].each { |chord| config.keys.unbind(chord) }
end
require_relative 'navigation'
require_relative 'transients'
require_relative 'policy'
Toyoterm.configure { |config| config.keys.unbind('ALT+D') }
require_relative 'events'

Toyoterm.configure do |config|
  config.window.bar :bottom do |bar|
    bar.section(:left) { |section| section.add('Layered keymaps, transient operations, advice and lifecycle events') }
    bar.section(:right) { |section| section.add('Alt+N: navigate | Alt+S: split menu | Alt+R: Pane tools | Alt+M: policy') }
  end
end
