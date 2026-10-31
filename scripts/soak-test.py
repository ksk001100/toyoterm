#!/usr/bin/env python3
"""Opt-in headless/real-GUI soak and bounded PTY fixture (stdlib only)."""
import argparse
import base64
import csv
import ctypes
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import threading
import time
import uuid

ROOT = Path(__file__).resolve().parent.parent


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def positive(value):
    value = int(value)
    if value < 1:
        raise argparse.ArgumentTypeError("must be positive")
    return value


def collect_log_errors(path):
    count, examples = 0, []
    with path.open(encoding="utf-8", errors="replace") as gui_log:
        for line in gui_log:
            if line.startswith("ERROR "):
                count += 1
                if len(examples) < 100:
                    examples.append(line.rstrip())
    return count, examples


def memory(pid):
    """Bytes, never mix working set/private bytes/footprint into one metric."""
    if sys.platform == "win32":
        from ctypes import wintypes

        class Counters(ctypes.Structure):
            _fields_ = [("cb", wintypes.DWORD), ("faults", wintypes.DWORD)] + [
                (name, ctypes.c_size_t) for name in
                ("peak_ws", "ws", "peak_paged", "paged", "peak_nonpaged",
                 "nonpaged", "pagefile", "peak_pagefile", "private")]

        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        psapi = ctypes.WinDLL("psapi", use_last_error=True)
        kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel.OpenProcess.restype = wintypes.HANDLE
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        psapi.GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.c_void_p, wintypes.DWORD]
        handle = kernel.OpenProcess(0x1000, False, pid)
        if not handle:
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            counters = Counters()
            counters.cb = ctypes.sizeof(counters)
            if not psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
                raise ctypes.WinError(ctypes.get_last_error())
            return {"rss_bytes": counters.ws, "private_bytes": counters.private}
        finally:
            kernel.CloseHandle(handle)
    if sys.platform.startswith("linux"):
        result = {}
        for line in Path(f"/proc/{pid}/status").read_text().splitlines():
            if line.startswith(("VmRSS:", "Threads:")):
                key, value, *_ = line.split()
                result["rss_bytes" if key == "VmRSS:" else "threads"] = int(value) * (1024 if key == "VmRSS:" else 1)
        rollup = Path(f"/proc/{pid}/smaps_rollup")
        if rollup.exists():
            private = 0
            for line in rollup.read_text().splitlines():
                if line.startswith("Pss:"):
                    result["pss_bytes"] = int(line.split()[1]) * 1024
                if line.startswith(("Private_Clean:", "Private_Dirty:")):
                    private += int(line.split()[1]) * 1024
            result["private_bytes"] = private
        return result
    rss = subprocess.check_output(["ps", "-o", "rss=", "-p", str(pid)], text=True, timeout=10)
    return {"rss_bytes": int(rss.strip()) * 1024}


def summarize(rows, warmup, axis="elapsed_s"):
    rows = [row for row in rows if row[axis] >= warmup]
    result = {"warmup_s" if axis == "elapsed_s" else "warmup_iterations": warmup,
              "samples": len(rows), "metrics": {}}
    if len(rows) < 6:
        result["status"] = "insufficient post-warmup samples; no stability verdict"
        return result
    x = [row[axis] / (60 if axis == "elapsed_s" else 100) for row in rows]
    mean_x = statistics.mean(x)
    denominator = sum((value - mean_x) ** 2 for value in x)
    if denominator == 0:
        raise ValueError("samples have no elapsed time")
    for key in ("rss_bytes", "private_bytes", "pss_bytes"):
        if not all(key in row for row in rows):
            continue
        y = [row[key] / 1048576 for row in rows]
        mean_y = statistics.mean(y)
        third = len(y) // 3
        result["metrics"][key] = {
            "slope_mib_per_min" if axis == "elapsed_s" else "slope_mib_per_100_iterations":
                sum((a - mean_x) * (b - mean_y) for a, b in zip(x, y)) / denominator,
            "early_mean_mib": statistics.mean(y[:third]),
            "late_mean_mib": statistics.mean(y[-third:]),
            "min_mib": min(y), "max_mib": max(y),
        }
    result["status"] = "review slopes and phase journal; this is not proof of a plateau"
    return result


def fixture(args):
    # Write fixed-size batches; never accumulate output proportional to duration.
    if args.kind == "images":
        png = (ROOT / "crates/toyoterm-script/tests/fixtures/background.png").read_bytes()
        for index in range(args.iterations):
            image_id = index + 1
            sys.stdout.buffer.write(f"\x1b_Ga=T,q=2,f=32,s=1,v=1,i={image_id};/wAA/w==\x1b\\".encode())
            sys.stdout.buffer.flush()
            time.sleep(args.delay)
            sys.stdout.buffer.write(f"\x1b_Ga=d,d=I,i={image_id};\x1b\\".encode())
            sys.stdout.buffer.write(b"\x1bPq#0;2;100;0;0#0~\x1b\\")
            sys.stdout.buffer.write(b"\x1b]1337;File=inline=1:" + base64.b64encode(png) + b"\x07")
            sys.stdout.buffer.flush()
            time.sleep(args.delay)
            # Erase and scroll legacy placements out, then re-display next cycle.
            sys.stdout.buffer.write(b"\x1b[2J\x1b[H" + b"\r\n" * 32)
    else:
        deadline = time.monotonic() + args.seconds
        remaining = args.lines
        while remaining > 0 and (args.seconds == 0 or time.monotonic() < deadline):
            count = min(1000, remaining)
            sys.stdout.buffer.write(b"toyoterm soak output\r\n" * count)
            sys.stdout.buffer.flush()
            remaining -= count
            if args.delay:
                time.sleep(args.delay)
    sys.stdout.buffer.write(b"\r\nSOAK_FIXTURE_DONE\r\n")
    sys.stdout.buffer.flush()
    if args.hold:
        time.sleep(args.hold)


def headless(args):
    command = ["cargo", "test", "-p", "toyoterm-cli", "-p", "toyoterm-terminal",
               "-p", "toyoterm-script", "--locked"]
    if args.release:
        command.append("--release")
    command += ["soak_", "--", "--ignored", "--nocapture", "--test-threads=1"]
    with (args.output / "headless.log").open("w", encoding="utf-8") as log:
        process = subprocess.Popen(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        try:
            for line in process.stdout:
                print(line, end="", flush=True)
                log.write(line)
            if process.wait() != 0:
                raise RuntimeError("headless soak failed; see headless.log")
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait()


def gui(args):
    executable = str(args.executable.resolve())
    metadata_path = args.output / "metadata.json"
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    metadata["binary_sha256"] = sha256(Path(executable))
    metadata["version"] = subprocess.check_output([executable, "version"], text=True, timeout=30).strip()
    metadata_path.write_text(json.dumps(metadata, indent=2), encoding="utf-8")
    env = os.environ.copy()
    env["TOYOTERM_INSTANCE"] = "soak-" + uuid.uuid4().hex[:12]
    env["TOYOTERM_RUNTIME_DIR"] = str(args.output / "runtime")
    env["TOYOTERM_HISTORY_FILE"] = str(args.output / "console-history")
    env["TOYOTERM_LOG"] = "warn,toyoterm::soak=trace"
    rows, errors = [], []
    iteration_rows = []
    stop = threading.Event()
    started = time.monotonic()
    phase = "startup"
    with (args.output / "gui.log").open("w", encoding="utf-8") as log, (args.output / "journal.jsonl").open("w", encoding="utf-8") as journal:
        process = subprocess.Popen([executable, "--config", str(args.config)], cwd=ROOT, env=env, stdout=log, stderr=log)

        def record(event, **fields):
            if "iteration" in fields:
                fields.update(memory(process.pid))
                iteration_rows.append({"phase": phase, **fields})
            journal.write(json.dumps({"elapsed_s": time.monotonic() - started, "utc_s": time.time(), "phase": phase, "event": event, **fields}) + "\n")
            journal.flush()

        def sample():
            with (args.output / "memory.csv").open("w", newline="", encoding="utf-8") as output:
                writer = csv.DictWriter(output, fieldnames=["elapsed_s", "utc_s", "phase", "rss_bytes", "private_bytes", "pss_bytes", "threads"])
                writer.writeheader()
                while not stop.is_set():
                    try:
                        row = {"elapsed_s": time.monotonic() - started, "utc_s": time.time(), "phase": phase, **memory(process.pid)}
                        rows.append(row)
                        writer.writerow(row)
                        output.flush()
                    except Exception as error:
                        errors.append(str(error))
                        break
                    stop.wait(args.interval)

        def run(*command, source=None):
            result = subprocess.run([executable, *command], cwd=ROOT, env=env, input=source,
                                    capture_output=True, text=True, timeout=30)
            if result.returncode or result.stderr.strip():
                raise RuntimeError(f"IPC failed: {result.stderr.strip()} {result.stdout.strip()}")
            return result.stdout

        def steel(source):
            # Console reports Scheme errors on stderr even when its exit code is zero.
            output = run("steel", "console", source=source + "\n:q\n")
            if "=> " not in output:
                raise RuntimeError(f"missing Scheme response: {output}")
            return output.split("=> ", 1)[1].splitlines()[0]

        def wait_for(source, expected, timeout=30):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                if steel(source) == expected:
                    return
                time.sleep(0.1)
            raise RuntimeError(f"timed out waiting for {source} == {expected}")

        worker = threading.Thread(target=sample, name="soak-memory")
        worker.start()
        record("spawn", pid=process.pid, instance=env["TOYOTERM_INSTANCE"])
        print(f"GUI pid={process.pid}; results={args.output}", flush=True)
        try:
            deadline = time.monotonic() + 30
            while True:
                if process.poll() is not None:
                    raise RuntimeError("GUI exited during startup; see gui.log")
                try:
                    run("list")
                    break
                except RuntimeError:
                    if time.monotonic() >= deadline:
                        raise
                    time.sleep(0.2)
            steel("(define soak-base-pane (current-pane)) (define soak-base-window (current-window)) (define soak-base-workspace (current-workspace))")
            baseline = steel("(list (length (workspaces)) (length (windows)) (apply + (map (lambda (w) (length (handle-ref w 'tabs))) (windows)))")

            def settle(seconds):
                if stop.wait(seconds):
                    raise RuntimeError("sampler stopped")
                if process.poll() is not None or errors:
                    raise RuntimeError(f"GUI/sampler failed: {errors}")

            phase = "idle"
            record("begin")
            settle(args.idle_seconds)
            if args.scenario == "idle":
                record("end")
                return

            launch = [sys.executable, str(Path(__file__).resolve()), "fixture", "--hold", "600"]

            def pane_fixture(kind, **options):
                command = launch + ["--kind", kind]
                for key, value in options.items():
                    command += ["--" + key.replace("_", "-"), str(value)]
                argv = scheme_list(command)
                steel(f"(split! soak-base-pane 'right (command-options {argv}))")
                wait_for("(not (= (handle-id (current-pane)) (handle-id soak-base-pane)))", "#true")
                return argv

            phase = "continuous-output"
            record("begin", seconds=args.output_seconds)
            pane_fixture("output", lines=10**9, seconds=args.output_seconds, delay=0.01)
            wait_for("(string-contains? (handle-ref (current-pane) 'screen-text) \"SOAK_FIXTURE_DONE\")", "#true", args.output_seconds + 30)
            steel("(close! (current-pane))")
            settle(1)

            phase = "scrollback"
            record("begin", lines=100000)
            pane_fixture("output", lines=100000, delay=0)
            wait_for("(string-contains? (handle-ref (current-pane) 'screen-text) \"SOAK_FIXTURE_DONE\")", "#true")
            for _ in range(30):
                steel("(pane-search! (current-pane) \"soak\") (action! 'toggle-visual-mode) (action! 'move-visual-selection 'right) (action! 'toggle-visual-mode)")
            steel("(close! (current-pane))")
            settle(1)

            for kind in ("pane", "tab", "workspace"):
                phase = kind + "-churn"
                record("begin", iterations=args.iterations)
                for iteration in range(1, args.iterations + 1):
                    if kind == "workspace":
                        steel(f"(open-workspace! \"soak-{iteration}\")")
                        wait_for("(not (= (handle-id (current-workspace)) (handle-id soak-base-workspace)))", "#true")
                        steel("(define soak-stale (current-pane)) (pane-send-text! (current-pane) \"exit\\r\\n\")")
                        wait_for("(handle-valid? soak-stale)", "#false")
                    else:
                        command = scheme_list(launch + ["--lines", "100", "--delay", "0"])
                        if kind == "pane":
                            steel(f"(split! soak-base-pane 'right (command-options {command}))")
                        else:
                            steel(f"(new-tab! soak-base-window (command-options {command}))")
                        wait_for("(not (= (handle-id (current-pane)) (handle-id soak-base-pane)))", "#true")
                        wait_for("(string-contains? (handle-ref (current-pane) 'screen-text) \"SOAK_FIXTURE_DONE\")", "#true")
                        steel("(define soak-stale (current-pane)) " + ("(close! (current-pane))" if kind == "pane" else "(close! (current-tab))"))
                        wait_for("(handle-valid? soak-stale)", "#false")
                    steel("(activate! soak-base-workspace) (activate! soak-base-pane)")
                    if iteration % 10 == 0:
                        record("iteration", iteration=iteration)
                    if steel("(list (length (workspaces)) (length (windows)) (apply + (map (lambda (w) (length (handle-ref w 'tabs))) (windows)))") != baseline:
                        raise RuntimeError(f"{kind} counts did not return to baseline")
                settle(2)

            phase = "images"
            record("begin", iterations=args.iterations)
            pane_fixture("images", iterations=args.iterations, delay=0.05)
            wait_for("(string-contains? (handle-ref (current-pane) 'screen-text) \"SOAK_FIXTURE_DONE\")", "#true", args.iterations * 0.1 + 30)
            steel("(close! (current-pane))")
            settle(2)

            phase = "steel-async"
            record("begin", iterations=args.async_iterations)
            command = scheme_list([sys.executable, "-c", "print('ready')"])
            for iteration in range(1, args.async_iterations + 1):
                slow_command = scheme_list([sys.executable, "-c", "import time; time.sleep(2)"])
                steel(f"(define soak-task (async! (car {command}) (cdr {command}) #f)) (define soak-cancelled (async! (car {slow_command}) (cdr {slow_command}) (lambda (r ctx) (error \"cancelled callback ran\"))))")
                steel("(unless (task-pending? soak-cancelled) (error \"cancel raced with completion\")) (task-cancel! soak-cancelled)")
                wait_for("(task-complete? soak-task)", "#true")
                wait_for("(length (__ref 'tasks '()))", "0")
                if iteration % 100 == 0:
                    record("iteration", iteration=iteration)
            steel("(set! soak-task #f)")

            phase = "config-reload"
            record("begin", iterations=args.reloads)
            for iteration in range(1, args.reloads + 1):
                run("reload")
                # Eval is a completion barrier behind reload; verify the fixture.
                steel("(unless (member \"soak_command\" (map cadr (__ref 'commands '()))) (error \"fixture command missing\"))")
                if iteration % 100 == 0:
                    record("iteration", iteration=iteration)
            phase = "recovery-idle"
            record("begin")
            settle(args.recovery_seconds)
            record("end")
        finally:
            stop.set()
            worker.join(timeout=15)
            if process.poll() is None:
                try:
                    # Exit the isolated fixture shell through the ordinary pane lifecycle.
                    steel("(pane-send-text! (current-pane) \"exit\\r\\n\")")
                    process.wait(timeout=5)
                except (RuntimeError, subprocess.TimeoutExpired):
                    process.terminate()
            try:
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            log.flush()
            log_error_count, log_error_examples = collect_log_errors(args.output / "gui.log")
            report = {"pid": process.pid, "sample_errors": errors,
                      "log_error_count": log_error_count,
                      "log_error_examples": log_error_examples,
                      "whole_run": summarize(rows, args.warmup),
                      "phases": {name: summarize([row for row in rows if row["phase"] == name], 0)
                                 for name in sorted({row["phase"] for row in rows})},
                      "iterations": {name: summarize([row for row in iteration_rows if row["phase"] == name], 0, "iteration")
                                     for name in sorted({row["phase"] for row in iteration_rows})}}
            (args.output / "summary.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
            if errors:
                raise RuntimeError(f"memory sampling failed: {errors}")
            if log_error_count:
                raise RuntimeError(f"GUI logged {log_error_count} errors; inspect summary.json and gui.log")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="mode", required=True)
    fixture_parser = sub.add_parser("fixture", help="run INSIDE a terminal; bounded stdout producer")
    fixture_parser.add_argument("--kind", choices=["output", "images"], default="output")
    fixture_parser.add_argument("--lines", type=positive, default=100000)
    fixture_parser.add_argument("--iterations", type=positive, default=100)
    fixture_parser.add_argument("--seconds", type=int, default=0)
    fixture_parser.add_argument("--delay", type=float, default=0.01)
    fixture_parser.add_argument("--hold", type=int, default=0)
    for mode in ("headless", "gui"):
        command = sub.add_parser(mode)
        command.add_argument("--output", type=Path, required=True, help="new results directory")
        if mode == "headless":
            command.add_argument("--release", action="store_true")
        else:
            command.add_argument("--executable", type=Path, required=True)
            command.add_argument("--scenario", choices=["all", "idle"], default="all")
            command.add_argument("--config", type=Path, help="configuration for --scenario idle; defaults to soak-config.scm")
            command.add_argument("--iterations", type=positive, default=100)
            command.add_argument("--reloads", type=positive, default=1000)
            command.add_argument("--async-iterations", type=positive, default=300)
            command.add_argument("--idle-seconds", type=positive, default=30)
            command.add_argument("--output-seconds", type=positive, default=30)
            command.add_argument("--recovery-seconds", type=positive, default=60)
            command.add_argument("--interval", type=positive, default=5)
            command.add_argument("--warmup", type=positive, default=30)
    args = parser.parse_args()
    if args.mode == "fixture":
        if args.delay < 0 or args.seconds < 0 or args.hold < 0:
            parser.error("delay, seconds, and hold must be non-negative")
        fixture(args)
        return
    if args.mode == "gui":
        if args.config is not None and args.scenario != "idle":
            parser.error("--config requires --scenario idle")
        args.config = (args.config or ROOT / "scripts/soak-config.scm").resolve()
        if not args.config.is_file():
            parser.error(f"configuration is not a file: {args.config}")
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    metadata = {"platform": platform.platform(), "python": sys.version, "argv": sys.argv,
                "utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                "dirty": subprocess.check_output(["git", "status", "--short"], cwd=ROOT, text=True)}
    metadata["fixture_sha256"] = sha256(args.config if args.mode == "gui" else ROOT / "scripts/soak-config.scm")
    if args.mode == "gui":
        metadata["config_path"] = str(args.config)
        metadata["config_sha256"] = metadata["fixture_sha256"]
    metadata["harness_sha256"] = sha256(Path(__file__))
    (args.output / "metadata.json").write_text(json.dumps(metadata, indent=2), encoding="utf-8")
    try:
        (headless if args.mode == "headless" else gui)(args)
    except BaseException as error:
        (args.output / "FAILED.txt").write_text(f"{type(error).__name__}: {error}", encoding="utf-8")
        raise
    (args.output / "COMPLETED.txt").write_text("Scenarios completed. Review trends; long soak remains required.\n", encoding="utf-8")


def scheme_list(values):
    return "(list " + " ".join(json.dumps(v, ensure_ascii=False) for v in values) + ")"


if __name__ == "__main__":
    main()
