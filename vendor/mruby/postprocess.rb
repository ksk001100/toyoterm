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

if platform == "posix"
  stat_times = <<~'C'.b
      /* Extract time values FIRST while macros are still defined.
       * On POSIX systems, st_atime may be a macro for st_atim.tv_sec */
      time_t atime_val, mtime_val, ctime_val;
    #if defined(st_atime)
      /* st_atime is a macro - use it to extract from src */
      atime_val = src->st_atime;
      mtime_val = src->st_mtime;
      ctime_val = src->st_ctime;
    #elif defined(__APPLE__) || defined(__FreeBSD__) || \
          defined(__OpenBSD__) || defined(__NetBSD__) || defined(__DragonFly__)
      /* BSD/macOS: st_atime is typically a direct member */
      atime_val = src->st_atime;
      mtime_val = src->st_mtime;
      ctime_val = src->st_ctime;
    #else
      /* POSIX.1-2008: use st_atim.tv_sec directly */
      atime_val = src->st_atim.tv_sec;
      mtime_val = src->st_mtim.tv_sec;
      ctime_val = src->st_ctim.tv_sec;
    #endif
  C
  replacement = <<~'C'.b
      /* An earlier amalgamated source undefines the st_*time compatibility
       * macros, so access the platform's underlying timespec members directly. */
      time_t atime_val, mtime_val, ctime_val;
    #if defined(__APPLE__) || defined(__NetBSD__)
      atime_val = src->st_atimespec.tv_sec;
      mtime_val = src->st_mtimespec.tv_sec;
      ctime_val = src->st_ctimespec.tv_sec;
    #else
      /* POSIX.1-2008: use st_atim.tv_sec directly */
      atime_val = src->st_atim.tv_sec;
      mtime_val = src->st_mtim.tv_sec;
      ctime_val = src->st_ctim.tv_sec;
    #endif
  C
  abort "POSIX mruby stat conversion was not found" unless source.sub!(stat_times, replacement)
elsif platform == "windows"
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
