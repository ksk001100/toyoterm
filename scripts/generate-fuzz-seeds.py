#!/usr/bin/env python3
"""Recreate named baseline seeds; never remove minimized regression inputs."""
from pathlib import Path
import base64
import struct
import zlib

ROOT = Path(__file__).resolve().parent.parent / "fuzz" / "corpus"
ST = b"\x1b\\"


def osc(payload):
    return b"\x1b]" + payload + ST


def kitty(payload):
    return b"\x1b_G" + payload + ST


def png():
    def chunk(kind, data):
        return (struct.pack(">I", len(data)) + kind + data
                + struct.pack(">I", zlib.crc32(kind + data)))
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(b"\0\xff\0\0\xff"))
            + chunk(b"IEND", b""))


def main():
    image = png()
    encoded = base64.b64encode(image)
    size = str(len(image)).encode()
    # Derived from tests/graphics.rs, src/alacritty.rs tests and the image example.
    seeds = {
        "raw_terminal": {
            "ascii": b"hello\r\nworld\t!",
            "sgr": b"\x1b[31;1mred\x1b[0m\x1b[2J\x1b[H",
            "unterminated": b"\x1b]0;unfinished\x1b",
            "c1-utf8-reset": b"\x9d0;title\x9c\xf0\x9f\x98\x80\x1bc",
        },
        "osc": {
            "title": osc(b"0;title") + b"\x1b]2;title\x07",
            "hyperlink": osc(b"8;id=seed;https://example.com") + b"link" + osc(b"8;;"),
            "shell": osc(b"133;A") + b"$ " + osc(b"133;B") + b"echo hi\r\n"
                     + osc(b"133;C") + b"hi\r\n" + osc(b"133;D;0"),
            "notification": osc(b"99;i=seed:d=0:p=title;Hello")
                            + osc(b"99;i=seed:d=1:p=body;World"),
        },
        "kitty_graphics": {
            "rgba": kitty(b"a=T,f=32,s=2,v=1,i=42;/wAA/wD/AIA="),
            "chunk-delete": kitty(b"a=t,f=100,i=9,m=1;" + encoded[:20])
                            + kitty(b"m=0;" + encoded[20:])
                            + kitty(b"a=p,i=9,p=3,c=4,r=2,C=1;")
                            + kitty(b"a=d,d=I,i=9;"),
            "zlib": kitty(b"a=T,f=32,s=1,v=1,i=7,o=z;"
                          + base64.b64encode(zlib.compress(b"\xff\0\0\xff"))),
            "placeholder": kitty(b"a=T,U=1,f=32,s=1,v=1,i=42;/wAA/w==")
                           + "\x1b[38;2;0;0;42m\U0010eeee\u0305\u0305\x1b[39m".encode(),
            "huge-malformed": kitty(b"a=T,f=32,s=4294967295,v=4294967295;???="),
            "payload-session": b"a=t,f=32,s=1,v=1,i=9,m=1;/wAA\nm=0;/w==\na=p,i=9;\na=d,d=I,i=9;",
        },
        "sixel": {
            "palette-repeat": b'\x1bP0;1q"1;1;3;6#1;2;100;0;0!3~$#2;2;0;100;0A' + ST,
            "huge": b'\x1bPq"1;1;4294967295;4294967295!999999999999999999999~' + ST,
            "payload": b'"1;1;3;6#1;2;100;0;0!3~$#2;2;0;100;0A',
            "malformed": b'\x1bPq#999999999999999999;2;-1;NaN;100!0~-' + ST,
        },
        "iterm_image": {
            "png": osc(b"1337;File=inline=1;size=" + size + b":" + encoded),
            "multipart": osc(b"1337;MultipartFile=inline=1;size=" + size)
                         + osc(b"1337;FilePart=" + encoded[:20])
                         + osc(b"1337;FilePart=" + encoded[20:]) + osc(b"1337;FileEnd"),
            "download": osc(b"1337;File=inline=0;name=dGVzdC50eHQ=;size=3:YWJj"),
            "missing-end": osc(b"1337;MultipartFile=inline=1;size=999999999")
                           + osc(b"1337;FilePart=AAAA"),
            "malformed": osc(b"1337;File=inline=1;size=999999999:???="),
            "long-metadata": osc(b"1337;File=inline=1;name=" + b"A" * 8192 + b":AAAA"),
            "payload-session": b"MultipartFile=inline=0;name=dGVzdC50eHQ=;size=3\nFilePart=YWJj\nFileEnd",
        },
        "kitty_file_transfer": {
            "parent-links": b"ac=send;id=s\nac=file;id=s;fid=d;ft=directory;n=ZGly\n"
                            b"ac=file;id=s;fid=f;pr=d;n=ZGlyL2E=;sz=1\n"
                            b"ac=end_data;id=s;fid=f;d=YQ==\n"
                            b"ac=file;id=s;fid=l;ft=symlink;pr=d;n=ZGlyL2xpbms=\n"
                            b"ac=end_data;id=s;fid=l;d=YQ==\nac=finish;id=s",
            "zlib": b"ac=send;id=z\nac=file;id=z;fid=f;n=YQ==;sz=3;zip=zlib\n"
                    b"ac=end_data;id=z;fid=f;d=" + base64.b64encode(zlib.compress(b"abc"))
                    + b"\nac=finish;id=z",
            "regular": b"".join(osc(b"5113;" + part) for part in [
                b"ac=send;id=test", b"ac=file;id=test;fid=f1;n=L3RtcC9maWxlLnR4dA==;sz=3",
                b"ac=data;id=test;fid=f1;d=YQ==", b"ac=end_data;id=test;fid=f1;d=YmM=",
                b"ac=finish;id=test"]),
            "payload-session": b"ac=send;id=s\nac=file;id=s;fid=f;n=YS50eHQ=;sz=3\n"
                               b"ac=end_data;id=s;fid=f;d=YWJj\nac=finish;id=s",
            "traversal-parent-symlink": b"ac=send;id=s\nac=file;id=s;fid=f;pr=f;n=Li4vLi4vZXNjYXBl;sz=3\n"
                                       b"ac=end_data;id=s;fid=f;d=YWJj\n"
                                       b"ac=file;id=s;fid=l;ft=symlink;n=bGluaw==;sz=4\n"
                                       b"ac=end_data;id=s;fid=l;d=Li4vYQ==\nac=finish;id=s",
            "oversized": b"ac=send;id=s\nac=file;id=s;fid=f;n=YQ==;sz=18446744073709551615\n"
                         b"ac=data;id=s;fid=f;d=???=\nac=cancel;id=s",
            "upload": b"ac=receive;id=u;sz=1\nac=file;id=u;fid=f;n=Li4vZXNjYXBl\nac=finished;id=u",
        },
    }
    for target, files in seeds.items():
        directory = ROOT / target
        directory.mkdir(parents=True, exist_ok=True)
        for name, data in files.items():
            (directory / name).write_bytes(data)
    # Raw reaches each protocol directly, without a framing adapter.
    for target, files in seeds.items():
        if target != "raw_terminal":
            wire_seed = files["regular"] if target == "kitty_file_transfer" else next(iter(files.values()))
            (ROOT / "raw_terminal" / target).write_bytes(wire_seed)
    # Every requested OSC selector has a directly reachable seed.
    for code, payload in {
        4: b"1;rgb:ff/00/00", 7: b"file://localhost/tmp", 9: b"hello",
        21: b"?", 50: b"?", 52: b"c;YWJj", 66: b"s=2;large",
        21337: b"0;seed",
    }.items():
        (ROOT / "osc" / f"osc-{code}").write_bytes(osc(str(code).encode() + b";" + payload))
    (ROOT / "osc" / "osc-1337").write_bytes(seeds["iterm_image"]["png"])


if __name__ == "__main__":
    main()
