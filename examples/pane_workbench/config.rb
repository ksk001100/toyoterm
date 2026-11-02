require_relative '../shared/keys'

Toyoterm.command(:recipe_workbench, description: 'Add a right shell and a lower shell in this directory') do |context|
  # Both splits target the original Pane. Newly created handles are visible only
  # in the next callback snapshot, so do not chain calls on split's nil return.
  context.pane.split(:right, cwd: context.pane.cwd, env: { 'TOYOTERM_ROLE' => 'tools' })
  context.pane.split(:down, cwd: context.pane.cwd, env: { 'TOYOTERM_ROLE' => 'scratch' })
end

Toyoterm.command(:recipe_project_tab, description: 'Open a shell tab in the current directory') do |context|
  context.window.new_tab(cwd: context.pane.cwd, env: { 'TOYOTERM_ROLE' => 'project' })
end

Toyoterm.command(:recipe_find_pane, description: 'Find a Pane across all workspaces') do
  targets = {}
  Toyoterm.workspaces.each do |workspace|
    workspace.windows.each do |window|
      window.tabs.each do |tab|
        tab.panes.each do |pane|
          # inspect escapes metadata controls; cap length for the native selector.
          detail = (pane.cwd || pane.title).to_s.each_char.take(120).join.inspect
          label = "#{workspace.name.to_s.each_char.take(80).join.inspect} / tab #{tab.id} / pane #{pane.id}: #{detail}"
          targets[label] = [workspace, window, tab, pane]
        end
      end
    end
  end
  next if targets.empty?
  Toyoterm.select(title: 'Jump to Pane', items: targets.keys.take(4096)) do |choice|
    next unless choice
    path = targets[choice]
    # The Pane may have closed while the selector was open.
    path.each { |handle| handle.activate } if path.all? { |handle| handle.valid? }
  end
end

Toyoterm.configure do |config|
  config.keys.alt('w').command(:recipe_workbench)
  config.keys.alt('n').command(:recipe_project_tab)
  config.keys.alt('p').command(:recipe_find_pane)
  config.window.bar :bottom do |bar|
    bar.section(:left) do |section|
      section.add { |ctx| "#{ctx.workspace.name} / pane #{ctx.pane.id}#{ctx.pane.zoomed? ? ' / ZOOM' : ''}" }
    end
    bar.section(:right) { |section| section.add('Alt+W: layout | Alt+N: tab | Alt+P: jump') }
  end
end
