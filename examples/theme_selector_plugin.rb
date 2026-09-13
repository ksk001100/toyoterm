Toyoterm::Plugin.define "theme-selector" do |plugin|
  plugin.version = "0.1.0"
  plugin.api_requirement = ">= 0.1.0, < 0.2.0"

  plugin.command :choose_theme do
    Toyoterm.select(title: "Select theme", items: Toyoterm.themes) do |theme|
      next if theme.nil?

      Toyoterm.configure { |config| config.theme = theme }
    end
  end

  plugin.keys { leader("t").command(:choose_theme) }
end
