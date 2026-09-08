MRuby::Build.new do |conf|
  conf.toolchain :gcc

  conf.gem core: "mruby-error"
  conf.gembox "stdlib"
  conf.gembox "stdlib-ext"
  conf.gembox "math"
  conf.gembox "metaprog"
end
