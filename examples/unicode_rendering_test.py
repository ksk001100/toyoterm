#!/usr/bin/env python3
"""Developer fixture. Run inside toyoterm; no third-party Python packages."""
import argparse
import sys


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cursor", choices=("block", "beam", "underline"), default="block")
    parser.add_argument("--target", choices=("ascii", "cjk", "combining", "sized"), default="cjk")
    parser.add_argument("--plain", action="store_true", help="print without escapes or waiting")
    args = parser.parse_args()
    rows = [
        "ASCII       | ABC fi fl ffi -> => == === != !==",
        "CJK         | 日本語 界 漢字ABC ABC日本語XYZ",
        "Combining   | e\u0301 a\u0308",
        "Emoji       | 😀 👍🏻 🇯🇵 👨‍👩‍👧‍👦",
        "Mixed       | A界e\u0301😀Z",
        "PUA         | \ue0b0 \ue0b2 \uf120 \uf005",
        "Grid        | 012345678901234567890123456789",
        "Copy        | A界e\u0301😀Z (drag both ways, including wide trailing cells)",
        "Multiline   | 日本語",
        "            | e\u0301😀Z",
    ]
    if args.plain:
        print("\n".join(rows))
        return
    if not sys.stdout.isatty():
        parser.error("run in a terminal, or use --plain")
    # Clear the viewport so the cursor targets use reproducible screen rows.
    sys.stdout.write("\x1b[2J\x1b[H" + "\r\n".join(rows) + "\r\n")
    sys.stdout.write("Sized       | \x1b]66;s=2;界e\u0301😀\x07\r\n\r\n")
    sys.stdout.write("Colors      | \x1b[31mred \x1b[32mgreen \x1b[34mblue \x1b[0mdefault\r\n")
    sys.stdout.write("Drag/copy the rows; check glyphs, marks, colors and cell boundaries.\r\n")
    sys.stdout.write("Type Japanese with your IME and press Enter to finish: ")
    sys.stdout.flush()
    shape = {"block": 2, "underline": 4, "beam": 6}[args.cursor]
    row = {"ascii": 1, "cjk": 2, "combining": 3, "sized": 11}[args.target]
    try:
        sys.stdout.write(f"\x1b[{shape} q\x1b[{row};15H")
        sys.stdout.flush()
        input()
    finally:
        sys.stdout.write("\x1b[0m\x1b[0 q\x1b[18;1H\r\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
