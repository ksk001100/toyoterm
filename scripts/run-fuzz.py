#!/usr/bin/env python3
"""Locked dependency preparation and cargo-fuzz build/replay/campaign runner."""
import argparse
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent
TARGETS = ("raw_terminal", "osc", "kitty_graphics", "sixel", "iterm_image",
           "kitty_file_transfer")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("build", "smoke", "run"))
    parser.add_argument("target", choices=TARGETS)
    parser.add_argument("--seconds", type=int, default=3600)
    parser.add_argument("--max-len", type=int, default=1048576)
    parser.add_argument("--sanitizer", choices=("address", "none"), default="address")
    args = parser.parse_args()
    if args.seconds < 1 or args.max_len < 1:
        parser.error("seconds and max-len must be positive")
    lock = ROOT / "fuzz" / "Cargo.lock"
    original = lock.read_bytes()
    subprocess.run(["cargo", "fetch", "--locked", "--manifest-path", "fuzz/Cargo.toml"],
                   cwd=ROOT, check=True)
    env = dict(os.environ, CARGO_NET_OFFLINE="true")
    command = ["cargo", "+nightly", "fuzz"]
    options = ["--sanitizer", args.sanitizer]

    def invoke(arguments):
        try:
            subprocess.run(command + arguments, cwd=ROOT, env=env, check=True)
        finally:
            if lock.read_bytes() != original:
                raise RuntimeError("cargo-fuzz changed fuzz/Cargo.lock; review dependency resolution")

    if args.mode == "build":
        invoke(["build", *options, args.target])
    elif args.mode == "smoke":
        seeds = sorted((ROOT / "fuzz" / "corpus" / args.target).iterdir())
        if not seeds:
            raise RuntimeError(f"missing corpus for {args.target}")
        for seed in seeds:
            invoke(["run", *options, args.target, str(seed), "--",
                    "-timeout=10", "-rss_limit_mb=1024"])
    else:
        invoke(["run", *options, args.target, "--", "-dict=fuzz/protocol.dict",
                f"-max_total_time={args.seconds}", "-timeout=10", "-rss_limit_mb=1024",
                f"-max_len={args.max_len}"])


if __name__ == "__main__":
    main()
