Toyoterm.command :choose_theme do
  Toyoterm.select(title: "Select theme", items: Toyoterm.themes) do |theme|
    next if theme.nil?

    Toyoterm.configure { |config| config.theme = theme }
  end
end

Toyoterm.configure do |config|
  config.keys { leader("t").command(:choose_theme) }
end
