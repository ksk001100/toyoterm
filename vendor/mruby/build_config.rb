MRuby::Build.new do |conf|
  conf.toolchain :gcc

  conf.gem core: "mruby-compiler"
  conf.gem core: "mruby-error"
  conf.gem core: "mruby-eval"
  conf.gem core: "mruby-enum-ext"
  conf.gem core: "mruby-array-ext"
  conf.gem core: "mruby-hash-ext"
  conf.gem core: "mruby-string-ext"
  conf.gem core: "mruby-proc-ext"
  conf.gem core: "mruby-metaprog"
  conf.gem core: "mruby-object-ext"
  conf.gem core: "mruby-class-ext"
  conf.gem core: "mruby-method"
end
