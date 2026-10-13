#!/usr/bin/env python3
"""Run the actual furniture recovery engine with injected native boundaries.

No DFHack SDK, protobuf serialization, Rust runtime or live game is exercised.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
ENGINE = "bridge/common/build_placement.h"
TEST = "bridge/common/tests/build_placement_recovery_test.cpp"
INPUTS = [ENGINE, "bridge/common/retained_snapshot.h", TEST, "scripts/test_build_placement_recovery.py"]
FLAGS = ["-std=c++17", "-Wall", "-Wextra", "-Werror", "-pedantic", "-O1", "-g",
         "-fsanitize=undefined", "-fno-sanitize-recover=all"]


def execute(compiler: str, root: Path, binary: Path) -> subprocess.CompletedProcess[str]:
    built = subprocess.run([compiler, *FLAGS, str(root / TEST), "-o", str(binary)],
                           capture_output=True, text=True, timeout=90, check=False)
    if built.returncode:
        raise RuntimeError("C++ compilation failed:\n" + built.stderr)
    return subprocess.run([str(binary)], capture_output=True, text=True, timeout=30, check=False)


def check(compiler: str, mutations: bool) -> dict:
    compiler_path = shutil.which(compiler)
    if not compiler_path:
        raise RuntimeError("C++ compiler unavailable: " + compiler)
    hashes = {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in INPUTS}
    changes = {
        "recovery_is_only_cached_query": (
            "            verify_after(r, expected, read, verify);",
            "            (void)expected; (void)read; (void)verify;"),
        "missing_final_capture": (
            " || inspect(r.before.selection, read).encode() != expected", ""),
        "uncertainty_not_released": (
            "r = std::move(next); unresolved_ = false;", "r = std::move(next);"),
    }
    with tempfile.TemporaryDirectory(prefix="dfmcp-build-recovery-") as directory:
        work = Path(directory)
        result = execute(compiler_path, ROOT, work / "test")
        if result.returncode:
            raise RuntimeError(result.stderr or result.stdout)
        actual = json.loads(result.stdout)
        killed = []
        if mutations:
            for label, (before, after) in changes.items():
                copy = work / label
                for relative in INPUTS:
                    target = copy / relative
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(ROOT / relative, target)
                target = copy / ENGINE
                text = target.read_text(encoding="utf-8")
                if text.count(before) != 1:
                    raise RuntimeError("mutation does not identify exactly one boundary: " + label)
                target.write_text(text.replace(before, after), encoding="utf-8")
                mutant = execute(compiler_path, copy, copy / "test")
                if mutant.returncode != 1 or "check failed:" not in mutant.stderr:
                    raise RuntimeError("mutant did not fail an assertion: " + label + "\n" + mutant.stderr)
                killed.append(label)
        if hashes != {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in INPUTS}:
            raise RuntimeError("source changed during tests")
    version = subprocess.run([compiler_path, "--version"], capture_output=True, text=True,
                             timeout=10, check=True).stdout.splitlines()[0]
    return {"schema": "dfmcp.build-recovery-evidence/1", "status": "passed_injected_boundaries_only",
            "compiler": version, "flags": FLAGS, "actual_engine": actual,
            "rejected_compiled_mutants": killed, "source_sha256": hashes,
            "real_dfhack_sdk": False, "protobuf_serialization": False, "rust_executed": False,
            "live_fortress": False, "full_repository_qualification": False}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default="g++")
    parser.add_argument("--mutations", action="store_true")
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    text = json.dumps(check(args.compiler, args.mutations), indent=2, sort_keys=True) + "\n"
    if args.evidence:
        args.evidence.write_text(text, encoding="utf-8")
    print(text, end="")


if __name__ == "__main__":
    main()
