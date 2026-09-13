# Example configuration demonstrating asynchronous status bar sections using
# BarSection#add_async.
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
    bar.section(:left, separator: " ") do |section|
      section.add { |context| context.workspace.name }
      section.add("toyoterm")
    end

    ping_flag = Toyoterm.platform == :windows ? "-n" : "-c"
    bar.section(:center, separator: " | ") do |section|
      section.add_async("ping", ping_flag, "1", "1.1.1.1",
                        interval: 10.0, initial: "Ping: measuring...") do |result|
        if result.success? && result.stdout =~ /(?:Average = |time=)([\d.]+ ?ms)/i
          "Ping: #{$1}"
        elsif result.success?
          "Ping: ok"
        else
          "Ping: failed"
        end
      end

      section.add_async("curl", "-s", "https://wttr.in/?format=1",
                        interval: 300.0, initial: "Weather: fetching...") do |result|
        if result.success? && !result.stdout.strip.empty?
          result.stdout.strip
        else
          "Weather: unavailable"
        end
      end

      # Synchronous values can share the same separator-delimited section.
      section.add { "Host: #{Toyoterm.platform}" }
    end
  end
end
