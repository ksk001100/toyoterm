#!/usr/bin/env python3
"""Opt-in real GUI/PTY baseline. Python stdlib only; no timing assertions."""
import argparse
import base64
import ctypes
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import platform
import re
import statistics
import struct
import subprocess
import sys
import time
import uuid
import zlib

ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location("soak", ROOT / "scripts/soak-test.py")
SOAK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SOAK)
FIELDS = re.compile(r'(\w+)=("[^"]*"|[^ ]+)')
LOG_CACHE = {}


def events(path):
    offset, partial, result = LOG_CACHE.get(path, (0, b"", []))
    with path.open("rb") as log:
        log.seek(offset)
        incoming = log.read()
        offset = log.tell()
    lines = (partial + incoming).split(b"\n")
    partial = lines.pop()
    for raw in lines:
        line = raw.decode("utf-8", errors="replace")
        if "toyoterm::perf:" not in line:
            continue
        row = dict(FIELDS.findall(line))
        for key, value in row.items():
            try:
                row[key] = json.loads(value)
            except (ValueError, TypeError):
                pass
        if "ts_ns" in row:
            result.append(row)
    LOG_CACHE[path] = (offset, partial, result)
    return result


def stats(values):
    values = sorted(values)
    if not values:
        return {"samples": 0, "p50_ms": None, "p95_ms": None, "max_ms": None}
    return {"samples": len(values), "p50_ms": statistics.median(values),
            "p95_ms": values[math.ceil(len(values) * .95) - 1],
            "max_ms": max(values)}


def cpu_seconds(pid):
    """Process user+kernel CPU, separate from elapsed wall time; unavailable is null."""
    if sys.platform.startswith("linux"):
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
        return (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")
    if sys.platform == "win32":
        from ctypes import wintypes
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel.OpenProcess.restype = wintypes.HANDLE
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        kernel.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
        handle = kernel.OpenProcess(0x1000, False, pid)
        if not handle:
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            times = [wintypes.FILETIME() for _ in range(4)]
            if not kernel.GetProcessTimes(handle, *(ctypes.byref(t) for t in times)):
                raise ctypes.WinError(ctypes.get_last_error())
            return sum((t.dwHighDateTime << 32) + t.dwLowDateTime for t in times[2:]) / 1e7
        finally:
            kernel.CloseHandle(handle)
    return None


def summarize(rows, begin, end):
    groups = {}
    for row in rows:
        if begin <= row["ts_ns"] <= end and "stage" in row:
            groups.setdefault(row["stage"], []).append(row["duration_us"] / 1000)
    stages = {key: {**stats(value), "total_ms": sum(value)} for key, value in groups.items()}
    stalls = groups.get("pty_event", []) + groups.get("frame", [])
    cache = [r for r in rows if begin <= r["ts_ns"] <= end and "uploaded_images" in r]
    return {"stages": stages, "frame_count": len(groups.get("present", [])),
            "frame": stats(groups.get("frame", [])),
            "main_thread_work_max_ms": max(stalls, default=None),
            "pty_event_count": len(groups.get("pty_event", [])),
            "uploaded_images": sum(r["uploaded_images"] for r in cache),
            "retained_images_max": max((r["retained_images"] for r in cache), default=0)}


def png(width, height):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    # Fixed checkerboard, no external codecs or image files.
    raw = b"".join(b"\0" + b"".join(
        bytes((220, 40, 80, 255)) if (x // 8 + y // 8) % 2 else bytes((30, 180, 220, 255))
        for x in range(width)) for y in range(height))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))


def image_fixture(protocol, width, height):
    if protocol == "sixel":
        stripes = (f"#0!{width}~-".encode()) * (height // 6)
        first = (f'\x1bPq"1;1;{width};{height}#0;2;80;20;50'.encode() + stripes + b"\x1b\\")
        return first, b"\x1b[H" + first
    encoded = base64.b64encode(png(width, height))
    if protocol == "iterm2":
        first = b"\x1b]1337;File=inline=1:" + encoded + b"\x07"
        return first, b"\x1b[H" + first
    # Use supported direct PNG chunks. Repeated placement retains the decoded image.
    chunks = [encoded[i:i + 4096] for i in range(0, len(encoded), 4096)]
    first = b"".join((b"\x1b_G" + (b"a=T,f=100,i=7,q=2," if i == 0 else b"q=2,")
                      + f"m={int(i < len(chunks)-1)};".encode() + part + b"\x1b\\")
                     for i, part in enumerate(chunks))
    return first, b"\x1b[H\x1b_Ga=p,i=7,p=2,q=2;\x1b\\"


def payload(case):
    kind = case["scenario"]
    if kind in ("bulk", "panes"):
        units = {"ascii": b"toyoterm baseline output 0123456789\r\n",
                 "ansi": b"\x1b[31mred\x1b[32mgreen\x1b[0m plain\r\n",
                 "unicode": "日本語 café e\u0301 🙂\r\n".encode(),
                 "mixed": "plain \x1b[36m日本語 🙂\x1b[0m end\r\n".encode()}
        unit = units[case.get("content", "mixed")]
        size = case["bytes"]
        # Pad with ASCII rather than cutting UTF-8 or a control string in half.
        return unit * (size // len(unit)) + b" " * (size % len(unit)), None
    if kind in ("scroll", "search"):
        return b"baseline error needle 0123456789\r\n" * case["lines"], None
    if kind == "redraw":
        data = bytearray(b"\x1b[?1049h")
        for frame in range(case["redraws"]):
            data.extend(b"\x1b[?2026h\x1b[H\x1b[2J")
            for row in range(1, case["rows"] + 1):
                text = f"{frame:04}:{row:03} " + "x" * (case["columns"] - 10)
                data.extend(f"\x1b[{row};1H\x1b[{31 + frame % 7}m\x1b[2K{text}\x1b[0m".encode())
            data.extend(b"\x1b[?2026l")
        return bytes(data), None
    if kind == "images":
        return image_fixture(case["protocol"], case["width"], case["height"])
    return b"baseline ready\r\n", None


def cases():
    result = [{"scenario": "bulk", "content": content, "bytes": mib * 1048576}
              for content in ("ascii", "ansi", "unicode", "mixed") for mib in (1, 10)]
    result += [{"scenario": "redraw", "columns": cols, "rows": rows, "redraws": 200}
               for cols, rows in ((80, 24), (120, 40), (200, 60))]
    result += [{"scenario": "scroll", "lines": lines} for lines in (1000, 10000, 100000)]
    result += [{"scenario": "panes", "panes": panes, "bytes": 1048576, "batch_delay_ms": 1}
               for panes in (1, 2, 4, 8)]
    result += [{"scenario": "search", "lines": 100000}]
    result += [{"scenario": "images", "protocol": protocol, "width": size, "height": size,
                "image_size": label} for protocol in ("sixel", "kitty", "iterm2")
               for label, size in (("small", 60), ("medium", 300), ("large", 1200))]
    result += [{"scenario": "reload", "config_size": size} for size in ("small", "large")]
    return result


def wait_for(check, process=None, timeout=120):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        if process and process.poll() is not None:
            raise RuntimeError("GUI exited; see gui.log")
        time.sleep(.01)
    raise RuntimeError("benchmark did not complete (operational timeout, not a performance verdict)")


def fixture(directory, token, delay):
    """Child process under the real PTY; file gates exclude startup/preparation."""
    directory = Path(directory)
    parts = [(directory / "first.bin").read_bytes()]
    if (directory / "repeat.bin").exists():
        parts.append((directory / "repeat.bin").read_bytes())
    if (directory / "cached.bin").exists():
        parts.append((directory / "cached.bin").read_bytes())
    sys.stdout.buffer.write(f"\x1b]2;toyoterm-perf:ready-{token}\x07".encode())
    sys.stdout.buffer.write(f"\r\nTOYOTERM_PERF_ready-{token}\r\n".encode())
    sys.stdout.buffer.flush()
    for phase, data in enumerate(parts):
        wait_for(lambda: (directory / f"gate-{phase}").exists())
        begin = time.time_ns()
        (directory / f"start-{token}-{phase}.json").write_text(json.dumps({"ts_ns": begin}))
        # Python's Windows console transport requires complete UTF-8 writes.
        # Terminal parser fragmentation remains covered by its existing tests.
        for chunk in utf8_chunks(data):
            sys.stdout.buffer.write(chunk)
            sys.stdout.buffer.flush()
            if delay:
                time.sleep(delay / 1000)
        sys.stdout.buffer.write(f"\x1b]2;toyoterm-perf:done-{token}-{phase}\x07".encode())
        # Text written inside a large image's covered cells removes its placement.
        # Image phases use only OSC title markers, preserving the image for upload.
        if len(parts) == 1:
            sys.stdout.buffer.write(f"\r\nTOYOTERM_PERF_done-{token}-{phase}\r\n".encode())
        sys.stdout.buffer.flush()
        (directory / f"producer-done-{token}-{phase}").touch()
    # The main app owns child termination; keep the final viewport alive.
    while True:
        time.sleep(60)


def utf8_chunks(data, size=8192):
    offset = 0
    while offset < len(data):
        end = min(len(data), offset + size)
        while end < len(data) and data[end] & 0xc0 == 0x80:
            end -= 1
        if end == offset:
            raise ValueError("chunk size cannot hold one UTF-8 character")
        yield data[offset:end]
        offset = end


def config(case, window_size=None):
    # Redraw grids are calibrated before opening the producer gate.
    cols, rows = case.get("columns", 120), case.get("rows", 40)
    width, height = window_size or (cols * 9 + 24, rows * 18 + 100)
    source = f'''Toyoterm.configure do |c|
  c.font.family = "monospace"
  c.font.size = 14
  c.window.width = {width}
  c.window.height = {height}
  c.scrollback_lines = 100000
end
'''
    if case.get("config_size") == "large":
        source += "\n".join(f'Toyoterm.command(:baseline_{i}) {{ {i} }}' for i in range(500))
    return source


def run_case(executable, directory, case):
    directory.mkdir(parents=True)
    first, repeat = payload(case)
    (directory / "first.bin").write_bytes(first)
    if repeat is not None:
        (directory / "repeat.bin").write_bytes(repeat)
        (directory / "cached.bin").write_bytes(b"\x1b[H")
    config_path = directory / "config.rb"
    config_path.write_text(config(case), encoding="utf-8")
    env = os.environ.copy()
    env.update(TOYOTERM_INSTANCE="perf-" + uuid.uuid4().hex[:12],
               TOYOTERM_RUNTIME_DIR=str(directory / "runtime"),
               TOYOTERM_HISTORY_FILE=str(directory / "history"),
               TOYOTERM_LOG="warn,toyoterm::perf=trace")
    tokens = [f"p{i}" for i in range(case.get("panes", 1))]

    def command(token):
        return [sys.executable, str(Path(__file__).resolve()), "fixture", str(directory), token,
                str(case.get("batch_delay_ms", 0))]

    def ipc(*args, source=None):
        output = subprocess.run([executable, *args], env=env, input=source, capture_output=True,
                                text=True, encoding="utf-8", timeout=120)
        if output.returncode or output.stderr.strip():
            raise RuntimeError(f"IPC failed: {output.stderr} {output.stdout}")
        return output.stdout

    def ruby(source):
        output = ipc("ruby", "console", source=source + "\n:q\n")
        if "=> " not in output:
            raise RuntimeError(f"missing Ruby result: {output}")
        return output

    log_path = directory / "gui.log"
    with log_path.open("w", encoding="utf-8") as log:
        spawn_ns = time.time_ns()
        process = subprocess.Popen([executable, "--config", str(config_path), "-e", *command(tokens[0])],
                                   cwd=ROOT, env=env, stdout=log, stderr=log)
        try:
            window_size = (case.get("columns", 120) * 9 + 24, case.get("rows", 40) * 18 + 100)
            for attempt in range(8):
                ready = wait_for(lambda: next((r for r in events(log_path) if r.get("marker") == "ready-p0"
                                               and r["ts_ns"] >= spawn_ns), None), process)
                if case["scenario"] != "redraw" or (ready["columns"], ready["rows"]) == (case["columns"], case["rows"]):
                    break
                if attempt == 7:
                    raise RuntimeError("display/font constraints prevented requested redraw grid")
                # Outside measurement: restart at a corrected logical window size.
                # A small bounded search accounts for font metrics, chrome and DPI.
                window_size = (max(1, window_size[0] + (case["columns"] - ready["columns"]) * 8),
                               max(1, window_size[1] + (case["rows"] - ready["rows"]) * 18))
                process.terminate()
                process.wait(timeout=10)
                config_path.write_text(config(case, window_size), encoding="utf-8")
                spawn_ns = time.time_ns()
                process = subprocess.Popen([executable, "--config", str(config_path), "-e", *command(tokens[0])],
                                           cwd=ROOT, env=env, stdout=log, stderr=log)
            for index, token in enumerate(tokens[1:], 1):
                # Split breadth first, alternating direction, in a single tab/window.
                ruby(f'Toyoterm.current_tab.panes[{(index - 1) // 2}].split('
                     f'{":right" if index % 2 else ":down"}, command: {json.dumps(command(token))})')
                wait_for(lambda: any(r.get("marker") == f"ready-{token}" for r in events(log_path)), process)
            time.sleep(.2)
            phases = []
            phase_payloads = [first] if repeat is None else [first, repeat, b"\x1b[H"]
            for phase, phase_payload in enumerate(phase_payloads):
                memory_before = SOAK.memory(process.pid)
                cpu_before = cpu_seconds(process.pid)
                (directory / f"gate-{phase}").touch()
                def finished():
                    rows = events(log_path)
                    markers = {r.get("marker") for r in rows}
                    return rows if all(f"done-{t}-{phase}" in markers for t in tokens) else None
                rows = wait_for(finished, process)
                begin = min(json.loads((directory / f"start-{t}-{phase}.json").read_text())["ts_ns"] for t in tokens)
                ends = [next(r for r in rows if r.get("marker") == f"done-{t}-{phase}") for t in tokens]
                end = max(r["ts_ns"] for r in ends)
                # Include the outer frame's stage, emitted just after its marker.
                time.sleep(.02)
                rows = events(log_path)
                frame_end = next((r["ts_ns"] for r in rows if r.get("stage") == "frame" and r["ts_ns"] >= end), end)
                duration = (end - begin) / 1e6
                if duration <= 0:
                    raise RuntimeError("wall clock changed during benchmark")
                size = len(phase_payload) * len(tokens)
                cpu_after = cpu_seconds(process.pid)
                phases.append({"phase": ("first", "repeat", "cached_redraw")[phase], "duration_ms": duration,
                               "bytes": size, "mib_s": size / 1048576 / (duration / 1000),
                               "actual_grids": [[r["columns"], r["rows"]] for r in ends],
                               "memory_before": memory_before, "memory_after": SOAK.memory(process.pid),
                               "process_cpu_ms": (cpu_after - cpu_before) * 1000 if cpu_before is not None else None,
                               **summarize(rows, begin, frame_end)})
                if case["scenario"] == "images" and phases[-1]["retained_images_max"] == 0:
                    raise RuntimeError("image did not reach GPU renderer; PTY may have filtered the protocol")
                if case["scenario"] == "images" and phase == 0 and phases[-1]["uploaded_images"] == 0:
                    raise RuntimeError("first image frame had no texture upload")
            if case["scenario"] in ("search", "reload"):
                begin = time.time_ns()
                if case["scenario"] == "search":
                    ruby("Toyoterm.current_pane.search('needle')")
                else:
                    ipc("reload")
                    # Ordered script barrier: wait for reload/application before recording completion.
                    ruby("Toyoterm.current_pane.id")
                def next_frame():
                    rows = events(log_path)
                    stage = "search_apply" if case["scenario"] == "search" else "config_apply"
                    applied = next((r["ts_ns"] for r in rows if r.get("stage") == stage and r["ts_ns"] > begin), None)
                    # Require successful present after the measured operation, not an idle redraw.
                    presented = next((r["ts_ns"] for r in rows if applied and r.get("stage") == "present" and r["ts_ns"] > applied), None)
                    return next((r for r in rows if presented and r.get("stage") == "frame" and r["ts_ns"] >= presented), None)
                frame = wait_for(next_frame, process)
                phases.append({"phase": case["scenario"], "duration_ms": (frame["ts_ns"] - begin) / 1e6,
                               **summarize(events(log_path), begin, frame["ts_ns"])})
            errors, examples = SOAK.collect_log_errors(log_path)
            if errors:
                raise RuntimeError(f"GUI errors: {examples}")
            adapter = next((line for line in log_path.read_text(encoding="utf-8", errors="replace").splitlines()
                            if "toyoterm::perf: adapter" in line), None)
            return {"case": case, "adapter": adapter, "fixture_sha256": hashlib.sha256(first).hexdigest(),
                    "repeat_sha256": hashlib.sha256(repeat).hexdigest() if repeat is not None else None,
                    "phases": phases, "status": "complete"}
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            LOG_CACHE.pop(log_path, None)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="mode", required=True)
    child = sub.add_parser("fixture")
    child.add_argument("directory")
    child.add_argument("token")
    child.add_argument("delay", type=float)
    run = sub.add_parser("run")
    run.add_argument("--executable", type=Path, required=True)
    run.add_argument("--output", type=Path, required=True)
    run.add_argument("--scenario", choices=["all", "bulk", "redraw", "scroll", "panes", "search", "images", "reload"], default="all")
    run.add_argument("--samples", type=int, default=3)
    run.add_argument("--label", default="v0.2.0")
    run.add_argument("--build-profile", default="release", help="record the actual binary profile")
    run.add_argument("--notes", default="", help="hardware, font versions, display, power settings")
    run.add_argument("--ruby", action="store_true", help="also save native resolver and script-thread overhead")
    run.add_argument("--quick", action="store_true", help="one small case per scenario; diagnostic only")
    compare = sub.add_parser("compare")
    compare.add_argument("before", type=Path)
    compare.add_argument("after", type=Path)
    args = parser.parse_args()
    if args.mode == "fixture":
        fixture(args.directory, args.token, args.delay)
        return
    if args.mode == "compare":
        def durations(path):
            groups = {}
            for row in map(json.loads, path.read_text(encoding="utf-8").splitlines()):
                if row.get("status") != "complete":
                    continue
                for phase in row["phases"]:
                    key = json.dumps([row["case"], row["fixture_sha256"], row["repeat_sha256"],
                                      row["phases"][0].get("actual_grids"), phase["phase"]], sort_keys=True)
                    groups.setdefault(key, []).append(phase["duration_ms"])
            return {k: statistics.median(v) for k, v in groups.items()}
        before, after = durations(args.before), durations(args.after)
        for key in sorted(before.keys() & after.keys()):
            print(json.dumps({"case_phase": json.loads(key), "before_ms": before[key], "after_ms": after[key],
                              "change_percent": (after[key] / before[key] - 1) * 100}))
        for key in sorted(before.keys() ^ after.keys()):
            print(json.dumps({"case_phase": json.loads(key), "status": "unmatched",
                              "present_in": "before" if key in before else "after"}))
        return
    if args.samples < 1:
        parser.error("--samples must be positive")
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    executable = str(args.executable.resolve())
    selected = [c for c in cases() if args.scenario in ("all", c["scenario"])]
    if args.quick:
        seen = set()
        selected = [c for c in selected if not (c["scenario"] in seen or seen.add(c["scenario"]))]
    metadata = {"schema_version": 1, "label": args.label, "platform": platform.platform(),
                "build_profile": args.build_profile, "notes": args.notes, "cpu": platform.processor(),
                "python": sys.version, "utc_ns": time.time_ns(), "quick": args.quick,
                "binary_sha256": SOAK.sha256(Path(executable)),
                "version": subprocess.check_output([executable, "version"], text=True).strip(),
                "harness_sha256": SOAK.sha256(Path(__file__).resolve()),
                "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                "dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT)),
                "samples": args.samples, "cases": selected}
    (args.output / "metadata.json").write_text(json.dumps(metadata, indent=2), encoding="utf-8")
    if args.ruby:
        command = ["cargo", "test", "-p", "toyoterm-app", "-p", "toyoterm-script", "--locked"]
        if args.build_profile == "release":
            command += ["--release"]
        command += ["performance_", "--", "--ignored", "--nocapture", "--test-threads=1"]
        completed = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, encoding="utf-8")
        (args.output / "ruby.log").write_text(completed.stdout + completed.stderr, encoding="utf-8")
        if completed.returncode:
            raise RuntimeError("Ruby/native baseline failed; see ruby.log")
        records = [json.loads(line[line.index('{'):]) for line in completed.stdout.splitlines()
                   if '{"schema_version":' in line]
        if len(records) != 4:
            raise RuntimeError("missing Ruby/native baseline records")
        (args.output / "ruby.jsonl").write_text("".join(json.dumps(r) + "\n" for r in records), encoding="utf-8")
    with (args.output / "results.jsonl").open("w", encoding="utf-8") as output:
        for index, case in enumerate(selected):
            # One unrecorded warmup per case, with identical fixture/instrumentation.
            for sample in range(-1, args.samples):
                directory = args.output / f"case-{index:02}-sample-{sample}"
                print(f"{case} sample={sample}", file=sys.stderr, flush=True)
                try:
                    row = run_case(executable, directory, case)
                except Exception as error:
                    row = {"case": case, "status": "failed", "error": str(error)}
                    output.write(json.dumps(row) + "\n")
                    output.flush()
                    raise
                if sample >= 0:
                    output.write(json.dumps({"schema_version": 1, "sample": sample, **row}) + "\n")
                    output.flush()


if __name__ == "__main__":
    main()
