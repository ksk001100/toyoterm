require_relative '../shared/keys'
require_relative 'themes'

# The selected policy lives in this VM; reload resets it to automatic.
automatic = true
scheduled_theme = proc { |hour| hour >= 7 && hour < 18 ? 'recipe.day' : 'recipe.night' }
apply_theme = proc do |name|
  Toyoterm.configure { |config| config.theme = name }
end
apply_theme.call(scheduled_theme.call(Time.now.hour))

Toyoterm.command(:recipe_theme, description: 'Choose a theme or follow local time') do
  Toyoterm.select(title: 'Theme policy', items: ['Automatic', 'recipe.day', 'recipe.night']) do |choice|
    next unless choice
    name = choice == 'Automatic' ? scheduled_theme.call(Time.now.hour) : choice
    apply_theme.call(name)
    automatic = choice == 'Automatic'
  end
end

# Start after loading, and restart after reload. A global scope keeps the timer
# alive when the initial Pane closes. The guard prevents duplicate schedules.
[:app_started, :config_reloaded].each do |name|
  Toyoterm.on(name) do
    unless Toyoterm.resource_scopes.any? { |scope| scope.name == 'recipe.theme.clock' }
      Toyoterm.resource_scope(name: 'recipe.theme.clock') do
        Toyoterm.every(60) do
          target = scheduled_theme.call(Time.now.hour)
          apply_theme.call(target) if automatic && Toyoterm.config[:theme] != target
        end
      end
    end
  end
end

Toyoterm.configure do |config|
  config.keys.alt('t').command(:recipe_theme)
  config.window.bar :bottom do |bar|
    bar.section(:left) do |section|
      section.add { "#{Toyoterm.config[:theme]} / #{automatic ? 'automatic' : 'manual'}" }
    end
    bar.section(:right) { |section| section.add('Alt+T: theme policy') }
  end
end
