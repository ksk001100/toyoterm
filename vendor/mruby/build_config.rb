# toyoterm's reproducible mruby 4.0.0 amalgamation configuration.
# Upstream source: 831da26b9021de0369d17b71b5667e2941a1a32d

platform = ENV.fetch("TOYOTERM_MRUBY_PLATFORM") do
  RUBY_PLATFORM.match?(/mswin|mingw/) ? "windows" : "posix"
end
unless %w[posix windows].include?(platform)
  raise "TOYOTERM_MRUBY_PLATFORM must be posix or windows"
end

MRuby::Build.new do |conf|
  conf.toolchain(platform == "windows" ? :visualcpp : :gcc)

  conf.gem core: "mruby-error"
  conf.gembox "stdlib"
  conf.gembox "stdlib-ext"
  conf.gembox "math"
  conf.gembox "metaprog"

  # Official mruby filesystem APIs and the platform HAL selected for the
  # generated amalgamation. Socket, task, sleep, exit, binary, and test gems
  # remain intentionally excluded.
  conf.gem core: "mruby-errno"
  conf.gem core: platform == "windows" ? "hal-win-io" : "hal-posix-io"
  conf.gem core: platform == "windows" ? "hal-win-dir" : "hal-posix-dir"

  # mruby 4.0.0 lacks File.write and Dir.glob class APIs. This Ruby-only gem
  # composes the official File/Dir primitives without adding a host bridge.
  conf.gem File.expand_path("filesystem-ext", __dir__)
end
