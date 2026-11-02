# Alt+P chooses a directory and opens a new terminal Pane there.
# Candidates are recomputed on the script thread as the path changes.
directories = proc do |query, _context|
  query = query.tr('\\', '/') if Toyoterm.platform == :windows
  directory = query.end_with?('/') ? query : File.dirname(query)
  prefix = query.end_with?('/') ? '' : File.basename(query)
  next [] unless File.directory?(directory)
  candidates = []
  total = 0
  Dir.entries(directory).sort.each do |name|
    next if name == '.' || name == '..' || !name.downcase.start_with?(prefix.downcase)
    path = directory == '.' && !query.start_with?('./') ? name : File.join(directory, name)
    next unless File.directory?(path)
    path += '/'
    next if path.bytesize > 4096 || path.unpack('U*').any? { |c| c < 32 || (127..159).include?(c) }
    break if candidates.length >= 4096 || total + path.bytesize > 4 * 1024 * 1024
    candidates << path
    total += path.bytesize
  end
  candidates
end

Toyoterm.command(:terminal_directory, description: 'Open a terminal in a chosen directory') do
  Toyoterm.read_string(title: 'Terminal directory', initial: Dir.pwd + '/',
                      completer: directories, history: :terminal_directories) do |path, context|
    next if path.nil? || path.empty?
    raise ArgumentError, 'choose an existing directory' unless File.directory?(path)
    context.pane.split(:right, cwd: path)
  end
end

Toyoterm.configure do |config|
  config.keys.alt('p').command(:terminal_directory)
end
