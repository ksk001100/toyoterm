require_relative '../shared/keys'
require_relative 'forge'

# Add one row to generate a Ruby method, command metadata and an Alt binding.
entries = [
  { name: :forge_tools, key: '1', direction: :right, role: 'tools',
    description: 'Open a tools shell beside this Pane' },
  { name: :forge_notes, key: '2', direction: :down, role: 'notes',
    description: 'Open a notes shell below this Pane' },
  { name: :forge_review, key: '3', direction: :left, role: 'review',
    description: 'Open a review shell to the left' }
].map { |entry| entry.freeze }.freeze
receiver, implementation = ExampleCommandForge.compile(entries)

# A separate subscriber decorates every generated operation without changing
# the factory. This observes a queued launch, not completed PTY creation.
Toyoterm.add_hook(:forge_launch_requested, label: :badge) do |role, context|
  context.pane.badge = "queued: #{role}"
end

Toyoterm.command(:forge_inspect, description: 'Inspect generated Ruby methods and native command metadata') do
  lines = implementation.instance_methods(false).sort.map do |name|
    info = Toyoterm.command_info(name)
    "#{receiver.method(name).name} | #{Toyoterm.command_keys(name).join(', ')} | #{info[:description]}"
  end
  Toyoterm.show_panel(title: 'Command Forge: generated methods -> commands -> keys', lines: lines)
end

Toyoterm.configure do |config|
  config.keys.alt('i').command(:forge_inspect)
  config.keys.alt('h').run { Toyoterm.command_help }
  config.keys.alt('o').run { Toyoterm.operation_palette }
  config.window.bar :bottom do |bar|
    bar.section(:left) { |section| section.add("#{entries.length} commands compiled from Ruby data") }
    bar.section(:right) { |section| section.add('Alt+1/2/3: shells | Alt+I: inspect | Alt+H: help') }
  end
end
