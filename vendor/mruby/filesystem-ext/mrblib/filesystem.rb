class File
  def self.write(path, data, offset = nil, mode: "w")
    file = open(path, mode)
    begin
      file.seek(offset) unless offset.nil?
      file.write(data)
    ensure
      file.close unless file.closed?
    end
  end

  def self.mtime(path)
    open(path) { |file| file.mtime }
  end
end

class Dir
  module Glob
    extend self

    def normalize(pattern)
      pattern.tr("\\", "/")
    end

    def split_root(pattern)
      if pattern.bytesize >= 3 && pattern.getbyte(1) == 58 && pattern.getbyte(2) == 47
        [pattern[0, 3], pattern[3..-1]]
      elsif pattern.start_with?("/")
        ["/", pattern[1..-1]]
      else
        [nil, pattern]
      end
    end

    def join(parent, child)
      return child if parent.nil? || parent.empty?
      return "#{parent}#{child}" if parent.end_with?("/")
      "#{parent}/#{child}"
    end

    def meta?(segment)
      segment.include?("*") || segment.include?("?") || segment.include?("[")
    end

    def class_match?(pattern, pattern_index, byte)
      negate = false
      if pattern.getbyte(pattern_index) == 33 || pattern.getbyte(pattern_index) == 94
        negate = true
        pattern_index += 1
      end
      matched = false
      previous = nil
      while pattern_index < pattern.bytesize
        current = pattern.getbyte(pattern_index)
        return [negate ? !matched : matched, pattern_index + 1] if current == 93
        if current == 45 && !previous.nil? && pattern.getbyte(pattern_index + 1) != 93
          range_end = pattern.getbyte(pattern_index + 1)
          matched = true if previous <= byte && byte <= range_end
          previous = range_end
          pattern_index += 2
        else
          matched = true if current == byte
          previous = current
          pattern_index += 1
        end
      end
      [false, pattern_index]
    end

    def segment_match?(pattern, name, pattern_index = 0, name_index = 0)
      if name.getbyte(0) == 46 && pattern.getbyte(0) != 46
        return false
      end
      while pattern_index < pattern.bytesize
        token = pattern.getbyte(pattern_index)
        if token == 42
          pattern_index += 1 while pattern.getbyte(pattern_index + 1) == 42
          pattern_index += 1
          return true if pattern_index >= pattern.bytesize
          while name_index <= name.bytesize
            return true if segment_match?(pattern, name, pattern_index, name_index)
            name_index += 1
          end
          return false
        end
        return false if name_index >= name.bytesize
        if token == 63
          pattern_index += 1
          name_index += 1
        elsif token == 91
          matched, next_index = class_match?(pattern, pattern_index + 1, name.getbyte(name_index))
          return false unless matched
          pattern_index = next_index
          name_index += 1
        else
          return false unless token == name.getbyte(name_index)
          pattern_index += 1
          name_index += 1
        end
      end
      name_index == name.bytesize
    end

    def entries(path)
      Dir.children(path.nil? || path.empty? ? "." : path)
    rescue SystemCallError
      []
    end

    def walk(parent, segments, index, matches, visited)
      if index >= segments.length
        candidate = parent.nil? ? "." : parent
        matches << candidate if File.exist?(candidate)
        return
      end

      segment = segments[index]
      if segment == "**"
        walk(parent, segments, index + 1, matches, visited)
        entries(parent).each do |entry|
          child = join(parent, entry)
          if File.directory?(child)
            canonical = File.realpath(child)
            unless visited[canonical]
              visited[canonical] = true
              walk(child, segments, index, matches, visited)
            end
          end
        end
        return
      end

      if meta?(segment)
        entries(parent).each do |entry|
          next unless segment_match?(segment, entry)
          child = join(parent, entry)
          if index == segments.length - 1
            matches << child
          elsif File.directory?(child)
            walk(child, segments, index + 1, matches, visited)
          end
        end
      else
        child = join(parent, segment)
        if index == segments.length - 1
          matches << child if File.exist?(child)
        elsif File.directory?(child)
          walk(child, segments, index + 1, matches, visited)
        end
      end
    end

    def expand(pattern)
      normalized = normalize(pattern)
      root, rest = split_root(normalized)
      segments = rest.split("/").reject(&:empty?)
      matches = []
      start = root.nil? ? "." : root
      visited = { File.realpath(start) => true }
      walk(root, segments, 0, matches, visited)
      matches.uniq.sort
    end
  end

  def self.glob(pattern, flags = 0, &block)
    raise ArgumentError, "unsupported glob flags" unless flags == 0
    patterns = pattern.is_a?(Array) ? pattern : [pattern]
    matches = patterns.flat_map { |item| Glob.expand(item) }.uniq.sort
    if block
      matches.each(&block)
      nil
    else
      matches
    end
  end

  class << self
    alias [] glob
  end
end
