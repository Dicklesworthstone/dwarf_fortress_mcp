#!/usr/bin/env python3
"""Compile and execute native recovery engines; no DFHack SDK or Rust required.

These tests exercise production C++ headers with deterministic native callbacks.
They do not qualify a DFHack plugin, the Rust stack, or a live fortress.
"""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
TESTS = (
    "test_job_suspension_recovery.cpp",
    "test_work_order_creation_recovery.cpp",
)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sanitize", action="store_true", help="enable AddressSanitizer and UndefinedBehaviorSanitizer")
    args = parser.parse_args()
    compiler = shlex.split(os.environ.get("CXX", "c++"))
    if not compiler or shutil.which(compiler[0]) is None:
        print("A C++17 compiler is required; set CXX to its command.", file=sys.stderr)
        return 1
    flags = ["-std=c++17", "-Wall", "-Wextra", "-Werror", "-pedantic"]
    flags += (["-g", "-fsanitize=address,undefined", "-fno-omit-frame-pointer"]
              if args.sanitize else ["-O2"])
    try:
        with tempfile.TemporaryDirectory(prefix="dfmcp-native-recovery-") as temporary:
            for name in TESTS:
                source = ROOT / "bridge" / "common" / "tests" / name
                executable = Path(temporary) / (source.stem + (".exe" if os.name == "nt" else ""))
                subprocess.run(compiler + flags + [str(source), "-o", str(executable)], check=True, timeout=60)
                subprocess.run([str(executable)], check=True, timeout=15)
    except (OSError, subprocess.SubprocessError) as exc:
        print(f"Native transaction recovery failed: {exc}", file=sys.stderr)
        return 1
    print("Native transaction recovery: 111 cases passed (not live-game qualification).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
