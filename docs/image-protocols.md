# Terminal images

toyoterm accepts images from terminal output by default. This works over a PTY,
including SSH, and does not require Ruby configuration. The image parser, image
storage, and GPU renderer are native Rust components.

Run this inside toyoterm to display the same red/green/blue chart three ways:

```sh
python examples/terminal_images.py
python examples/terminal_images.py --protocol kitty
```

## Supported subset

| Protocol | Supported |
| --- | --- |
| Sixel (DCS `ESC P … q … ESC \\`) | Raster dimensions, repeat, carriage return, next sixel line, 256 color registers, RGB and DEC HLS definitions, transparent background (`P2=1`) |
| Kitty (APC `ESC _ G … ESC \\`) | Direct base64 transmission (`t=d`), RGB/RGBA/PNG (`f=24/32/100`), zlib (`o=z`), chunking (`m`), transmit/query/display (`a=t/q/T/p`), image and placement IDs (`i/p`), cell sizes (`c/r`), cursor preservation (`C=1`), quiet replies (`q`), deletion of visible placements or an image ID (`d=a/A/i/I`) |
| iTerm2 (OSC 1337) | `File=inline=1`, base64 PNG/JPEG, optional byte `size`, `width`/`height` in cells, pixels, percent, or `auto`, `preserveAspectRatio`, BEL or ST termination |

Kitty replies use the normal PTY response channel. Queries decode and validate
the image without retaining or displaying it. Unknown image IDs and unsupported
actions return errors, subject to `q=1/2`. Image-less `a=p` and `a=d` commands
may omit the semicolon. Kitty placement IDs can be replaced independently.
Primary device attributes advertise Sixel, and `CSI 14 t` / `CSI 18 t` report
text-area pixel / cell dimensions.

Images retain their native pixel size unless a display size is requested. They
are clipped to their pane and composited after cell backgrounds and before text.
Placement starts at the current cursor. By default the cursor moves to the last
occupied image row and just beyond its occupied columns, clipped to the screen.
An image taller than the screen reserves at most one screen of cursor movement.
Kitty `C=1` leaves the cursor in place.

Images follow vertical scrolling into history. Primary and alternate screens
have separate placements; exiting the alternate screen discards its placements.
Erasing a screen region or line removes an entire image intersecting those rows.
Resizing the cell grid or changing the physical cell size clears placements;
stored Kitty images can be placed again. Reset clears all images and transfers.
Text selection and copying continue to operate on text only.

## Limits and remaining compatibility work

- Seven-bit ESC introducers are supported; eight-bit C1 introducers and tmux
  passthrough are not implemented.
- Sixel pixel-aspect scaling, persistent palettes across separate images, and
  DEC private Sixel modes are not implemented.
- Kitty file/shared-memory transmission, Unicode placeholders, animation,
  image-number addressing, cropping, offsets, relative placement, and nonzero
  z-index are not implemented. Use direct transmission with supported placement
  keys. Deletion selectors other than `a/A/i/I` return an unsupported error.
- OSC 1337 multipart transfers, file downloads (`inline=0`), and formats other
  than PNG/JPEG are not implemented.
- Each control string and accumulated Kitty transfer is limited to 32 MiB.
  Decoded RGBA is limited to 32 MiB and 4096 pixels per side. Image decoding also
  has a 32 MiB allocation budget. Oversized/malformed strings are discarded and
  normal text parsing resumes at the terminator.
- Each pane holds up to 128 placements / 64 MiB of placed pixels, conservatively
  counting shared images. Oldest placements are evicted. Kitty's retained image
  store separately allows 128 images / 64 MiB and reports `ENOSPC` when full.
  The protocols never cause toyoterm to open a local file or shared-memory path.
- An image command flushes preceding synchronized text to preserve cursor and
  output order; it may end a synchronized-update batch early.

The protocol references are the [Kitty graphics specification](https://sw.kovidgoyal.net/kitty/graphics-protocol/),
[iTerm2 inline-image specification](https://iterm2.com/documentation-images.html),
and [DEC Sixel graphics description](https://vt100.net/docs/vt3xx-gp/chapter14.html).

## Validation

`cargo test -p toyoterm-terminal --locked --test graphics` covers protocol bytes,
fragmentation, cursor placement, deletion, history, screen isolation, and limits.
`cargo test -p toyoterm-render --locked gpu_terminal_image_blends_and_clips_to_pane -- --ignored`
checks actual GPU pixels for alpha blending and pane clipping, including an image
partially above the viewport. Interactive chart checks on Linux/macOS/Windows
remain part of platform validation.
