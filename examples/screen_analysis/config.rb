require_relative '../shared/keys'
require_relative 'cooperative'
require_relative 'bounded'

Toyoterm.configure do |config|
  config.window.bar :bottom do |bar|
    bar.section(:left) { |section| section.add('Compare cooperative slices with an explicit Ruby instruction budget') }
    bar.section(:right) { |section| section.add('Alt+W: cooperative word count | Alt+I: bounded summary') }
  end
end
