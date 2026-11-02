# Each lens is a real generated Ruby method. Additional required libraries can
# publish lenses without editing the chooser or snapshot/selection pipeline.
module ExampleOutputLenses
  def self.register(name, &extract)
    implementation = Module.new
    implementation.define_method(:scan) do |text|
      text.split("\n").flat_map { |line| extract.call(line) }.uniq
    end
    scanner = Object.new.extend(implementation).method(:scan)
    Toyoterm.add_hook(:recipe_output_lenses, label: name) do
      { name: name, scan: scanner }
    end
  end
end

# This mruby build has no Regexp. Plain String operations keep the recipes
# runnable without gems, including multiple URLs on the same visible line.
ExampleOutputLenses.register('Diagnostics') do |line|
  ['error', 'warning', 'failed', 'exception'].any? { |word| line.downcase.include?(word) } ? [line.strip] : []
end
ExampleOutputLenses.register('URLs') do |line|
  line.split.select { |word| word.start_with?('https://', 'http://') }.map do |word|
    word = word[0...-1] while !word.empty? && ['.', ',', ';', ')', ']', '>'].include?(word[-1])
    word
  end
end
