require_relative '../shared/keys'

# A native terminal with a Ruby-defined dashboard. No icon font is required.
Toyoterm.configure do |config|
  config.window.bar :bottom do |bar|
    bar.section(:left) do |section|
      section.add { |ctx| "#{ctx.workspace.name} / #{ctx.tab.panes.length} panes" }
      section.add { |ctx| ctx.pane.cwd || 'cwd: enable shell integration' }
    end
    bar.section(:center) do |section|
      section.add do |ctx|
        pane = ctx.pane
        if pane.command_running?
          'RUNNING'
        elsif pane.last_exit_status.nil?
          'READY'
        else
          "exit #{pane.last_exit_status}"
        end
      end
    end
    bar.section(:right) do |section|
      # Without OSC 7, cwd is nil and Git uses the GUI process's directory.
      # Each widget has at most one process in flight. Missing Git is harmless.
      section.add_async('git', 'branch', '--show-current', interval: 5,
                        cwd: ->(ctx) { ctx.pane.cwd }, initial: 'git: ...') do |result|
        branch = result.stdout.strip
        result.success? ? (branch.empty? ? 'git: detached' : "git: #{branch}") : 'git: unavailable'
      end
      section.add(interval: 1) do
        now = Time.now
        '%02d:%02d:%02d' % [now.hour, now.min, now.sec]
      end
    end
  end
end

Toyoterm.on(:command_started) do |event|
  event.pane.badge = nil if event.pane.valid?
end
Toyoterm.on(:command_finished) do |event|
  next unless event.pane.valid?
  status = event.exit_status
  event.pane.badge = status.nil? || status == 0 ? nil : "exit #{status}"
end
