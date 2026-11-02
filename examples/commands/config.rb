require_relative '../shared/keys'
# palette.rb also works standalone and owns this same reload binding.
Toyoterm.configure { |config| config.keys.unbind('CTRL+SHIFT+R') }
require_relative 'palette'
require_relative 'help'
require_relative 'arguments'
require_relative 'completion'

Toyoterm.configure do |config|
  config.window.bar :bottom do |bar|
    bar.section(:left) { |section| section.add('Commands, typed arguments and directory completion') }
    bar.section(:right) { |section| section.add('Alt+X: palette | Alt+H: help | Alt+L: label | Alt+P: directory') }
  end
end
