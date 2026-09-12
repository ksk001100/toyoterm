# Example configuration demonstrating asynchronous widgets in the status bar
# using Toyoterm.async.
#
# Network operations (such as fetching weather or pinging a server) run in the
# background without blocking terminal input, rendering, or Ruby callbacks.
# When a task completes, the status bar is automatically refreshed with the result.

$weather = "Weather: fetching..."
$last_weather_fetch = 0

$ping = "Ping: measuring..."
$last_ping = 0

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
    bar.add(:left) { |context| context.workspace.name }
    bar.add(:left, "toyoterm")

    # Center widget: ping measurement updated every 10 seconds
    bar.add(:center) do
      now = Time.now.to_i
      if now - $last_ping >= 10
        $last_ping = now
        # Windows uses ping -n 1, Unix uses ping -c 1
        ping_flag = Toyoterm.platform == :windows ? "-n" : "-c"
        Toyoterm.async("ping", ping_flag, "1", "1.1.1.1") do |result|
          if result.success?
            if result.stdout =~ /(?:Average = |time=)([\d.]+ ?ms)/i
              $ping = "Ping: #{$1}"
            else
              $ping = "Ping: ok"
            end
          else
            $ping = "Ping: failed"
          end
        end
      end
      $ping
    end

    # Right widget: weather updated every 5 minutes (300 seconds)
    bar.add(:right) do
      now = Time.now.to_i
      if now - $last_weather_fetch >= 300
        $last_weather_fetch = now
        Toyoterm.async("curl", "-s", "https://wttr.in/?format=1") do |result|
          if result.success? && !result.stdout.strip.empty?
            $weather = result.stdout.strip
          else
            $weather = "Weather: unavailable"
          end
        end
      end
      $weather
    end
  end
end
