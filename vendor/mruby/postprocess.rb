# frozen_string_literal: true

platform, source_c, source_h, destination_c, destination_h = ARGV
unless %w[posix windows].include?(platform) && destination_h
  abort "usage: ruby postprocess.rb PLATFORM SOURCE_C SOURCE_H DESTINATION_C DESTINATION_H"
end

normalize = lambda do |path|
  File.binread(path).gsub("\r\r\n".b, "\n".b).gsub("\r\n".b, "\n".b)
end

source = normalize.call(source_c)
header = normalize.call(source_h)

if platform == "windows"
  include_line = "#include \"mruby.h\"\n".b
  replacement = <<~'C'.b
    #include "mruby-windows.h"
    #include <winsock2.h>
    #include <io.h>
  C
  abort "Windows mruby include was not found" unless source.sub!(include_line, replacement)

  timeval = <<~'C'.b
    struct timeval {
      time_t tv_sec;
      suseconds_t tv_usec;
    };
  C
  replacement = <<~'C'.b
    #if 0 /* winsock2.h supplies struct timeval */
    struct timeval {
      time_t tv_sec;
      suseconds_t tv_usec;
    };
    #endif
  C
  abort "Windows mruby timeval definition was not found" unless source.sub!(timeval, replacement)
end

File.binwrite(destination_c, source)
File.binwrite(destination_h, header)
