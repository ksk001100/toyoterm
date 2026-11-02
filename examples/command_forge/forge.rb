# This library deliberately creates methods on an anonymous Module, rather than
# adding helpers to Object or monkey-patching Toyoterm.
module ExampleCommandForge
  def self.compile(entries)
    implementation = Module.new
    entries.each do |entry|
      implementation.define_method(entry[:name]) do |context|
        context.pane.split(entry[:direction], cwd: context.pane.cwd,
                           env: { 'TOYOTERM_ROLE' => entry[:role] })
        Toyoterm.run_hook(:forge_launch_requested, entry[:role], context)
      end
    end
    receiver = Object.new.extend(implementation)
    entries.each do |entry|
      # Method objects become command callbacks. There is no source-string eval.
      callback = receiver.method(entry[:name])
      Toyoterm.command(entry[:name], description: entry[:description], &callback)
      Toyoterm.configure do |config|
        config.keys.alt(entry[:key]).command(entry[:name])
      end
    end
    [receiver, implementation]
  end
end
