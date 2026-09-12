# Example configuration demonstrating asynchronous widgets in the status bar
# using Toyoterm.async.
#
# Network operations (such as fetching weather or pinging a server) run in the
# background without blocking terminal input, rendering, or Ruby callbacks.
# When a task completes, the status bar is automatically refreshed with the result.

Toyoterm.configure do |config|
  config.font.family = "monospace"
  config.font.size = 14

  config.keys do
    ctrl_shift("t").new_tab
    ctrl_shift("\\").split(:right)
    ctrl_shift("r").reload_config
  end

  # Configure the bottom status bar with a 1-second update interval.
  config.window.bar :bottom, interval: 1.0 do |bar|
    weather_task = nil
    weather_text = "Weather: fetching..."
    last_weather_fetch = 0
    ping_task = nil
    ping_text = "Ping: measuring..."
    last_ping_fetch = 0

    bar.add(:left) { |context| context.workspace.name }
    bar.add(:left, "toyoterm")

    # Center widget: ping measurement updated every 10 seconds
    bar.add(:center) do
      now = Time.now.to_i
      if ping_task.nil? || (ping_task.complete? && now - last_ping_fetch >= 10)
        last_ping_fetch = now
        # Windows uses ping -n 1, Unix uses ping -c 1
        ping_flag = Toyoterm.platform == :windows ? "-n" : "-c"
        ping_task = Toyoterm.async("ping", ping_flag, "1", "1.1.1.1")
      end
      if ping_task.complete? && ping_task.success?
        if ping_task.result.stdout =~ /(?:Average = |time=)([\d.]+ ?ms)/i
          ping_text = "Ping: #{$1}"
        else
          ping_text = "Ping: ok"
        end
      elsif ping_task.complete?
        ping_text = "Ping: failed"
      end
      ping_text
    end

    # Right widget: weather updated every 5 minutes (300 seconds)
    bar.add(:right) do
      now = Time.now.to_i
      if weather_task.nil? || (weather_task.complete? && now - last_weather_fetch >= 300)
        last_weather_fetch = now
        weather_task = Toyoterm.async("curl", "-s", "https://wttr.in/?format=1")
      end
      if weather_task.complete? && weather_task.success? && !weather_task.result.stdout.strip.empty?
        weather_text = weather_task.result.stdout.strip
      elsif weather_task.complete?
        weather_text = "Weather: unavailable"
      end
      weather_text
    end
  end
end
