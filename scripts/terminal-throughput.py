#!/usr/bin/env python3
"""Run inside any terminal: output-to-DSR acknowledgement, not frame latency.

Example: wezterm start --always-new-process -- python scripts/terminal-throughput.py
--output results.json --label wezterm --version <version>
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import select
import statistics
import sys
import time


class RawInput:
    def __enter__(self):
        if os.name == "nt":
            self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
            self.kernel.GetStdHandle.restype = ctypes.c_void_p
            self.handle = self.kernel.GetStdHandle(-10)
            self.kernel.GetConsoleMode.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_ulong)]
            self.kernel.SetConsoleMode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
            self.mode = ctypes.c_ulong()
            if not self.kernel.GetConsoleMode(self.handle, ctypes.byref(self.mode)):
                raise ctypes.WinError(ctypes.get_last_error())
            # VT input with line buffering, echo and processed input disabled.
            if not self.kernel.SetConsoleMode(self.handle, (self.mode.value & ~7) | 0x200):
                raise ctypes.WinError(ctypes.get_last_error())
        else:
            import termios
            import tty
            self.mode = termios.tcgetattr(sys.stdin.fileno())
            tty.setraw(sys.stdin.fileno())
        return self

    def __exit__(self, *args):
        if os.name == "nt":
            self.kernel.SetConsoleMode(self.handle, self.mode)
        else:
            import termios
            termios.tcsetattr(sys.stdin.fileno(), termios.TCSANOW, self.mode)

    def read(self):
        if os.name == "nt":
            import msvcrt
            if msvcrt.kbhit():
                return msvcrt.getwch()
            time.sleep(.0005)
            return ""
        if select.select([sys.stdin], [], [], .01)[0]:
            return os.read(sys.stdin.fileno(), 1024).decode("ascii", errors="replace")
        return ""


def write(data):
    # Complete UTF-8 console writes on Windows; raw bytes on POSIX.
    offset = 0
    while offset < len(data):
        end = min(offset + 8192, len(data))
        while end < len(data) and data[end] & 0xc0 == 0x80:
            end -= 1
        sys.stdout.buffer.write(data[offset:end])
        sys.stdout.buffer.flush()
        offset = end


def acknowledge(raw):
    write(b"\x1b[6n")
    response = ""
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        response += raw.read()
        match = re.search(r"\x1b\[(\d+);(\d+)R", response)
        if match:
            return [int(match[2]), int(match[1])]
    raise RuntimeError("terminal did not acknowledge DSR within 15 seconds")


def workloads(mib):
    for name, line in [
        ("ascii", "plain terminal output 0123456789 " * 2 + "\r\n"),
        ("ansi", "\x1b[1;32mcolored\x1b[0m output 0123456789\r\n"),
        ("unicode", "\x1b[1;32mcolored\x1b[0m output 界 😀 0123456789\r\n"),
    ]:
        line = line.encode()
        yield name, line * max(1, mib * 1024 * 1024 // len(line)), False
    # Fixed screen workload, independent of rendering of intermediate frames.
    frame = b"\x1b[H\x1b[2J" + b"\x1b[32mredraw payload\x1b[0m\r\n" * 20
    yield "redraw", frame * 1000, True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--label", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--mib", type=int, default=10)
    args = parser.parse_args()
    if args.samples < 1 or args.mib < 1:
        parser.error("samples and mib must be positive")
    # Fail before measuring if the evidence path already exists.
    with args.output.open("x", encoding="utf-8") as output:
        record = {"schema_version": 1, "metric": "output_to_dsr_ack",
                  "label": args.label, "version": args.version,
                  "platform": platform.platform(), "cpu": platform.processor(),
                  "python": sys.version, "harness_sha256": hashlib.sha256(
                      Path(__file__).read_bytes()).hexdigest(), "cases": []}
        try:
            with RawInput() as raw:
                # Move to the clamped bottom-right corner to obtain actual grid.
                write(b"\x1b[9999;9999H")
                record["grid"] = acknowledge(raw)
                for name, data, alternate in workloads(args.mib):
                    samples = []
                    write(b"\x1b[?1049h" if alternate else b"\x1b[?1049l")
                    try:
                        for sample in range(args.samples + 1):
                            write(b"\x1b[H\x1b[2J")
                            acknowledge(raw)
                            time.sleep(.1)
                            started = time.perf_counter_ns()
                            write(data)
                            acknowledge(raw)
                            duration_ms = (time.perf_counter_ns() - started) / 1e6
                            if sample:
                                samples.append(duration_ms)
                    finally:
                        write(b"\x1b[?1049l")
                    median = statistics.median(samples)
                    record["cases"].append({"name": name, "bytes": len(data),
                        "fixture_sha256": hashlib.sha256(data).hexdigest(),
                        "samples_ms": samples, "median_ms": median,
                        "mib_s": len(data) / (1024 * 1024) / (median / 1000)})
            record["status"] = "complete"
        except Exception as error:
            record.update(status="failed", error=str(error))
            raise
        finally:
            json.dump(record, output, indent=2)
            output.write("\n")


if __name__ == "__main__":
    main()
