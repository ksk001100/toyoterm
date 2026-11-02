# 0009: Read Kitty image files on the native file worker

- Status: Accepted
- Date: 2026-11-03

## Context

Terminal graphics previously accepted only in-band image bytes and deliberately
performed no local file reads. Local Kitty clients commonly use `t=f`, including
Snacks' normal local image sender. Forcing every local client through chunked
base64 transmission incurs unnecessary encoding, PTY traffic, and client-side
pacing. This is a terminal protocol capability, independent of any editor.

## Decision

Support Kitty `t=f` for absolute UTF-8 paths to regular files, following symlinks.
The terminal parser emits an immutable `ImageFileRequest`. The existing bounded
native file worker performs path checks, file I/O, bounded decompression, and
RGB/RGBA/PNG decoding. An immutable completion returns via the app event loop;
the terminal, image storage, mux and renderer remain owned by the main thread.
The parser, including fuzz harnesses, never calls the file loader itself.

Preserve the stream semantics of `a=t/T/q`, including default cursor movement:
defer subsequent tokens and shell markers for that pane until completion. Limit
deferred output to 1 MiB; overload fails the image and resumes output. A resize
or cell-size change cancels pending images before replay and clearing placements.
Process-wide request IDs reject stale completions, including replaced panes.
The GUI and other panes continue processing while a file is being loaded.

Check paths and regular-file type before opening, then check the open handle
again. Reject Unix `/proc`, `/sys`, `/dev`, and Windows non-local-disk paths.
On Unix use nonblocking/no-controlling-terminal/no-follow-final-symlink open flags
after canonicalization to avoid a substituted FIFO or final symlink. File ranges,
decompression and decoded pixels retain the existing 32 MiB/4096-side limits.
All read/decode failures return `EBADF:Failed to read image file`, respecting
quiet modes; only local diagnostics contain the underlying failure reason.

## Consequences

This deliberately extends the previous in-band-only graphics policy. Terminal
output can request image reads from the terminal host's filesystem by default,
as specified by Kitty; it is not an OSC 5113 upload and does not use that
feature's configured root. There is no new Ruby API or dependency between crates.
The Unix loader uses the shared `libc` dependency for portable open flags.
Temporary-file deletion (`t=t`) and shared memory (`t=s`) remain unsupported.
Remote clients and different filesystem namespaces still need direct data.

No synchronous filesystem operation runs on the main event loop. A slow file
can delay that pane's following output and other jobs on the shared file worker;
queue and output limits prevent unbounded accumulation. Worker jobs are not
forcibly interrupted when a pane closes or its result becomes stale.
