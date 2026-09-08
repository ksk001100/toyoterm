#!/usr/bin/env python3
"""Display a generated color chart using Sixel, Kitty, or OSC 1337 (stdlib only)."""
import argparse
import base64
import struct
import sys
import zlib


def png_chart():
    def chunk(kind, data):
        return (struct.pack(">I", len(data)) + kind + data
                + struct.pack(">I", zlib.crc32(kind + data)))

    row = b"\0" + b"\xff\0\0\xff" * 32 + b"\0\xff\0\xff" * 32 + b"\0\0\xff\xff" * 32
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", 96, 48, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(row * 48)) + chunk(b"IEND", b""))


def sequence(protocol):
    png = png_chart()
    encoded = base64.b64encode(png).decode("ascii")
    if protocol == "kitty":
        return f"\x1b_Ga=T,f=100,q=2;{encoded}\x1b\\"
    if protocol == "iterm2":
        return f"\x1b]1337;File=inline=1;size={len(png)}:{encoded}\x07"
    colors = "#1;2;100;0;0#2;2;0;100;0#3;2;0;0;100"
    bands = "-".join(["#1!32~#2!32~#3!32~"] * 8)
    return f'\x1bP0;1q"1;1;96;48{colors}{bands}\x1b\\'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--protocol", choices=["all", "sixel", "kitty", "iterm2"], default="all")
    args = parser.parse_args()
    protocols = ["sixel", "kitty", "iterm2"] if args.protocol == "all" else [args.protocol]
    for protocol in protocols:
        sys.stdout.write(f"{protocol}\n{sequence(protocol)}\n\n")
    sys.stdout.flush()


if __name__ == "__main__":
    main()
