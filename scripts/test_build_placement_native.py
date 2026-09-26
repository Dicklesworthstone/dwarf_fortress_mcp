#!/usr/bin/env python3
"""Execute the actual furniture/1.19 handler with explicit SDK/protobuf doubles.

These source tests do not establish DFHack ABI compatibility, protobuf wire
serialization, a live fortress, or permission to admit the development protocol.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

from build_placement_wire import Capture, Plan, Record

ROOT = Path(__file__).resolve().parents[1]
ENGINE = "bridge/common/build_placement.h"
HANDLER = "bridge/dfhack-build-v1_19/dfmcp_build_v1_19.cpp"
TEST = "tests/native/build_placement/handler.cpp"
INPUTS = [
    "architecture/build_placement_v1_19.json",
    ENGINE, "bridge/common/retained_snapshot.h", HANDLER,
    "bridge/dfhack-build-v1_19/DfmcpBuildV1_19.proto",
    "bridge/dfhack-build-v1_19/CMakeLists.txt",
    TEST, "tests/native/build_placement/stubs.h",
    "scripts/test_build_placement_native.py", "scripts/build_placement_wire.py",
]
FLAGS = ["-std=c++17", "-Wall", "-Wextra", "-Werror", "-pedantic", "-g",
         "-fsanitize=undefined", "-fno-sanitize-recover=all"]


def run(command: list[str], *, success: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(command, text=True, capture_output=True, timeout=90, check=False)
    if success and result.returncode:
        raise RuntimeError(f"command failed: {command[0]}\n{result.stdout[:2000]}\n{result.stderr[:8000]}")
    return result


def compile_and_run(compiler: str, root: Path, work: Path) -> subprocess.CompletedProcess[str]:
    include = work / "include"
    include.mkdir(parents=True, exist_ok=True)
    # The actual translation unit is compiled without editing or replacing it.
    # Only explicit SDK and generated-message includes resolve to empty headers;
    # the test's stubs.h supplies their intentionally non-ABI API doubles.
    for name in re.findall(r'#include "([^"\n]+)"', (root / HANDLER).read_text(encoding="utf-8")):
        if name.startswith(".."):
            continue
        target = include / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text("#pragma once\n", encoding="utf-8")
    binary = work / "build-native-test"
    run([compiler, *FLAGS, f"-I{include}", str(root / TEST), "-o", str(binary)])
    return run([str(binary)], success=False)


def check(compiler: str, mutations: bool, selected: list[str] | None) -> dict:
    compiler_path = shutil.which(compiler)
    if compiler_path is None:
        raise RuntimeError(f"requested C++ compiler unavailable: {compiler}")
    source_hashes = {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in INPUTS}
    contract = json.loads((ROOT / "architecture/build_placement_v1_19.json").read_text(encoding="utf-8"))
    if contract["operator_gates"].get("writer_rechecks_current_credential") is not True:
        raise RuntimeError("furniture contract must require the current credential at native dispatch")
    changes = {
        "missing_full_revalidation": (ENGINE,
            "current.encode() != r.before.encode()", "false"),
        "missing_uncertainty_fence": (ENGINE,
            "require(!unresolved_, 8);", "require(true, 8);"),
        "terminal_replay_not_immutable": (ENGINE,
            "if (r.phase != Phase::Prepared) return r;",
            "if (r.phase == Phase::Cancelled || r.phase == Phase::Refused) return r;"),
        "missing_final_readback": (ENGINE,
            " || inspect(r.before.selection, read).encode() != expected", ""),
        "missing_writer_credential_recheck": (HANDLER,
            "authorize_credential(credential);", "(void)credential;"),
    }
    if selected and any(name not in changes for name in selected):
        raise ValueError("unknown mutation; available: " + ", ".join(changes))
    with tempfile.TemporaryDirectory(prefix="dfmcp-build-native-") as directory:
        work = Path(directory)
        result = compile_and_run(compiler_path, ROOT, work)
        if result.returncode:
            raise RuntimeError(f"actual native handler regression failed:\n{result.stdout[:2000]}\n{result.stderr[:8000]}")
        report = json.loads(result.stdout)
        emitted = json.loads(run([str(work / "build-native-test"), "--vectors"]).stdout)
        vectors = {name: bytes.fromhex(value) for name, value in emitted.items()}
        before = Capture.decode(vectors["capture"])
        plan = Plan("build-001", before)
        if (before.generation, before.sequence, before.tick, before.selection.item, before.selection.target) != (7, 0, 12345, 42, (15, 15, 2)):
            raise AssertionError("native capture differs from independently specified SDK fixture")
        if vectors["plan"] != plan.digest or vectors["token"] != plan.token:
            raise AssertionError("native plan/token differ from Python hashlib commitments")
        prepared = Record.decode(vectors["prepared"], plan)
        placed = Record.decode(vectors["placed"], plan)
        if prepared.phase != "prepared" or placed.phase != "placed" or placed.after != before.expected_after():
            raise AssertionError("actual native records disagree with Python state/receipt contract")
        if before.raw != vectors["capture"] or prepared.raw != vectors["prepared"] or placed.raw != vectors["placed"]:
            raise AssertionError("native canonical bytes do not roundtrip through Python wire decoders")
        killed = []
        if mutations or selected:
            for label, (path, before, after) in changes.items():
                if selected and label not in selected:
                    continue
                copy = work / label
                for relative in INPUTS:
                    target = copy / relative
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(ROOT / relative, target)
                target = copy / path
                source = target.read_text(encoding="utf-8")
                if before not in source:
                    raise RuntimeError(f"mutation no longer applies: {label}")
                target.write_text(source.replace(before, after), encoding="utf-8")
                mutant = compile_and_run(compiler_path, copy, copy)
                if mutant.returncode == 0 or "check failed:" not in mutant.stderr:
                    raise RuntimeError(f"mutant did not fail a regression assertion: {label}\n{mutant.stderr[:4000]}")
                killed.append(label)
        observed_hashes = {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in INPUTS}
        if observed_hashes != source_hashes:
            raise RuntimeError("source changed while native evidence was being captured")
        return {
            "schema": "dfmcp.build-native-evidence/1",
            "status": "passed_sdk_doubles_only",
            "actual_cpp_handler_executed": True,
            "compiler": run([compiler_path, "--version"]).stdout.splitlines()[0],
            "flags": FLAGS,
            "actual_handler": report,
            "python_wire_verified_native_vectors": len(vectors),
            "rejected_compiled_mutants": killed,
            "source_sha256": source_hashes,
            "warning_denied": True,
            "ubsan": True,
            "real_dfhack_sdk": False,
            "protobuf_serialization": False,
            "live_fortress": False,
            "rust_executed": False,
            "full_repository_qualification": False,
        }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default="g++")
    parser.add_argument("--mutations", action="store_true")
    parser.add_argument("--mutation", action="append", help="run a named mutation separately; repeatable")
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    result = check(args.compiler, args.mutations, args.mutation)
    text = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.evidence:
        args.evidence.write_text(text, encoding="utf-8")
    print(text, end="")


if __name__ == "__main__":
    main()
