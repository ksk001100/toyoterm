require_relative '../shared/keys'

# The same key means different things in different Panes. These are explicit
# argv launches, never text typed into a potentially busy interactive shell.
profiles = {
  rust: { marker: 'Cargo.toml', command: ['cargo', 'test'], label: 'Rust tests' },
  node: { marker: 'package.json', command: ['npm', 'test'], label: 'Node tests' }
}
modes = {}
profiles.each do |kind, profile|
  name = "recipe.project.#{kind}"
  keys = Toyoterm.keymap(name) do |keys|
    keys.alt('ENTER').run do |context|
      context.pane.split(:down, command: profile[:command], cwd: context.pane.cwd)
    end
  end
  modes[kind] = Toyoterm.mode(name, keymap: keys, kind: :major) do |enabled, context|
    context.pane.badge = enabled ? profile[:label] : nil
    if enabled
      pane_id = context.pane.id
      # This subscription belongs to this specific mode instance. Changing
      # directory removes it, so per-Pane handlers do not accumulate.
      Toyoterm.on(:command_finished) do |event|
        next unless event.pane.valid? && event.pane.id == pane_id
        status = event.exit_status.nil? ? '?' : event.exit_status
        event.pane.badge = "#{profile[:label]} | last exit #{status}"
      end
    end
  end
  Toyoterm.add_hook(:recipe_project_kind, priority: kind == :rust ? -20 : -10,
                    label: kind) do |cwd|
    kind if File.file?(File.join(cwd, profile[:marker]))
  end
end

reconcile = proc do |pane|
  next unless pane.valid?
  # Inspect just the reported directory, not an unbounded parent/tree walk.
  kind = pane.cwd && Toyoterm.run_hook_until_success(:recipe_project_kind, pane.cwd)
  selected = modes[kind]
  modes.each_value do |mode|
    mode.disable(pane: pane) if mode != selected && mode.enabled?(pane: pane)
  end
  selected.enable(pane: pane) if selected && !selected.enabled?(pane: pane)
end
[:pane_ready, :cwd_changed].each do |name|
  Toyoterm.on(name) { |event| reconcile.call(event.pane) }
end
[:app_started, :config_reloaded].each do |name|
  Toyoterm.on(name) do
    Toyoterm.workspaces.each do |workspace|
      workspace.windows.each do |window|
        window.tabs.each { |tab| tab.panes.each { |pane| reconcile.call(pane) } }
      end
    end
  end
end

Toyoterm.command(:recipe_detect_project, description: 'Recheck this Pane project markers') do |context|
  reconcile.call(context.pane)
end
Toyoterm.command(:recipe_project_help, description: 'Explain this Pane contextual keys') do |context|
  lines = ["cwd: #{context.pane.cwd.to_s.each_char.take(200).join.inspect}",
           "Modes: #{context.pane.modes.join(', ')}",
           'Alt+Enter: cargo test in a Rust root; npm test in a Node root.',
           'The task opens a lower Pane; exiting the task closes that Pane.',
           'Only direct Cargo.toml/package.json markers are checked; Rust wins ties.',
           'Alt+R: recheck markers after creating/removing a project file.']
  Toyoterm.show_panel(title: 'Project-sensitive terminal', lines: lines)
end
Toyoterm.configure do |config|
  config.keys.alt('r').command(:recipe_detect_project)
  config.keys.alt('h').command(:recipe_project_help)
  config.window.bar :bottom do |bar|
    bar.section(:left) { |section| section.add { |ctx| ctx.pane.modes.join(' / ') } }
    bar.section(:right) { |section| section.add('Alt+Enter: project task | Alt+H: explain | Alt+R: detect') }
  end
end
