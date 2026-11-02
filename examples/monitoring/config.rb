require_relative '../shared/keys'
require_relative 'panels'
require_relative 'timers'

Toyoterm.configure do |config|
  config.window.bar :bottom do |bar|
    bar.section(:left) { |section| section.add('Live terminal monitor and Pane-owned clock') }
    bar.section(:right) { |section| section.add('Alt+M: monitor | Alt+T: clock | Alt+R: reminder') }
  end
end
