require_relative '../shared/keys'
require_relative 'composition'
require_relative 'scopes'

Toyoterm.configure do |config|
  config.window.bar :bottom do |bar|
    bar.section(:left) { |section| section.add('Independent hooks and unloadable scope-owned resources') }
    bar.section(:right) { |section| section.add('Alt+P: compose launch | Alt+S: status | Alt+M: unload/reload library') }
  end
end
