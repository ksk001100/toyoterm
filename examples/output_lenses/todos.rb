require_relative 'lenses'

# An independent library contributes a lens to the same terminal UI.
ExampleOutputLenses.register('TODOs') do |line|
  ['TODO', 'FIXME', 'HACK'].any? { |word| line.include?(word) } ? [line.strip] : []
end
