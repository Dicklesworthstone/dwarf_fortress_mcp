#!/usr/bin/env python3
"""Compile the real bounded-run engine with deterministic callback doubles.

No DFHack SDK, game, admission, or release qualification is inferred from this.
Build products are retained in a unique directory; no repository files are removed.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import shutil
import subprocess
import tempfile


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", action="append", help="C++ compiler; repeat to test several")
    parser.add_argument("--build-dir", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    build = args.build_dir or Path(tempfile.mkdtemp(prefix="dfmcp-bounded-run-"))
    build.mkdir(parents=True, exist_ok=True)
    compilers = args.compiler or [name for name in ("g++", "clang++") if shutil.which(name)]
    if not compilers:
        parser.error("no C++ compiler found")
    for index, compiler in enumerate(compilers):
        binary = build / f"engine-{index}"
        subprocess.run([
            compiler, "-std=c++17", "-Wall", "-Wextra", "-Werror", "-pedantic",
            "-fsanitize=undefined", "-fno-sanitize-recover=all", "-g", "-I", str(root),
            str(root / "tests/native/bounded_run/engine.cpp"), "-o", str(binary),
        ], check=True, timeout=90)
        result = subprocess.run([str(binary)], check=True, timeout=15, text=True, capture_output=True)
        print(f"{compiler}: {result.stdout.strip()}")
    print(f"Retained native test artifacts: {build}")


if __name__ == "__main__":
    main()
