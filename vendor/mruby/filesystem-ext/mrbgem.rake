MRuby::Gem::Specification.new("toyoterm-filesystem-ext") do |spec|
  spec.license = "MIT"
  spec.author = "toyoterm contributors"
  spec.summary = "Missing standard File.write and Dir.glob APIs for mruby 4.0"

  spec.add_dependency "mruby-io", core: "mruby-io"
  spec.add_dependency "mruby-dir", core: "mruby-dir"
end
