# Alt+S opens the library's status panel. Close it, then Alt+M toggles the
# complete library (command, keymap activation, panels and timers).
module ScopedStatusExample
  NAME = 'example.scoped_status'.freeze
  KEYS = Toyoterm.keymap(:scoped_status_library) { key('ALT+S').command(:scoped_status) }

  def self.current
    Toyoterm.resource_scopes.find { |scope| scope.name == NAME }
  end

  def self.enable
    return false if current
    Toyoterm.resource_scope(name: NAME) do |scope|
      Toyoterm.command(:scoped_status, description: 'Show a scope-owned terminal panel', &scope.wrap do |context|
        panel = Toyoterm.show_panel(title: 'Scoped terminal status', lines: ["Pane: #{context.pane.id}"])
        Toyoterm.every(1) do |timer, tick|
          if panel.active?
            title = tick.pane.title.each_char.take(512).join.inspect
            panel.update(lines: ["Pane: #{tick.pane.id}", "Title: #{title}", "Command running: #{tick.pane.command_running?}"])
          else
            timer.cancel
          end
        end
      end)
      KEYS.activate
    end
    true
  end

  def self.disable
    scope = current
    scope ? scope.close : false
  end
end

ScopedStatusExample.enable
Toyoterm.command(:toggle_status_library, description: 'Enable/disable the scoped status library') do
  ScopedStatusExample.current ? ScopedStatusExample.disable : ScopedStatusExample.enable
end
Toyoterm.configure { |config| config.keys.alt('m').command(:toggle_status_library) }
