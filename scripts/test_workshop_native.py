#!/usr/bin/env python3
"""Execute the actual workshop/1.20 handler with explicit SDK/protobuf doubles.

This is component evidence for exact source bytes only. It does not establish a
real DFHack SDK build, generated protobuf behavior, ABI compatibility, a live
fortress, completed construction in a real game, or any admission.
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

ROOT = Path(__file__).resolve().parents[1]
ENGINE = "bridge/common/workshop_construction.h"
HANDLER = "bridge/dfhack-workshop-v1_20/dfmcp_workshop_v1_20.cpp"
TEST = "tests/native/workshop/handler.cpp"
VECTORS = "tests/native/workshop/vectors.json"
INPUTS = [
    "architecture/workshop_construction_v1_20.json",
    ENGINE, "bridge/common/retained_snapshot.h", HANDLER,
    "bridge/dfhack-workshop-v1_20/DfmcpWorkshopV1_20.proto",
    "bridge/dfhack-workshop-v1_20/CMakeLists.txt",
    TEST, "tests/native/workshop/stubs.h", VECTORS,
    "scripts/test_workshop_native.py",
]
FLAGS = ["-std=c++17", "-Wall", "-Wextra", "-Werror", "-pedantic", "-g",
         "-fsanitize=undefined", "-fno-sanitize-recover=all"]
MUTATIONS = {
    "missing_full_revalidation": (ENGINE, "current.encode() != r.before.encode()", "false"),
    "missing_uncertainty_fence": (ENGINE,
        "require(!unresolved_, 8); require(records_.size() < MAX_RECORDS, 5);",
        "require(records_.size() < MAX_RECORDS, 5);"),
    "terminal_replay_not_immutable": (ENGINE, "if (r.phase != Phase::Prepared) return r;",
        "if (r.phase == Phase::Cancelled || r.phase == Phase::Refused) return r;"),
    "missing_final_readback": (ENGINE, " || inspect(r.before.selection, read).encode() != expected", ""),
    "missing_planned_occupancy_oracle": (ENGINE, "&& setting_occupancy && planned_tiles == ALL_TILES", ""),
    "missing_worker_labor": (ENGINE, "return visible && adult && sane && labor && labor_valid && job_available && reachable;",
        "return visible && adult && sane && labor_valid && job_available && reachable;"),
    "missing_worker_availability": (ENGINE, "&& labor_valid && job_available && reachable; }",
        "&& reachable; }"),
    "missing_footprint_material_exclusion": (ENGINE, " || item_inside_footprint()) return false;", ") return false;"),
    "missing_material_reachability": (ENGINE, "&& material >= 0 && reachable && ground.dry()",
        "&& material >= 0 && ground.dry()"),
    "missing_writer_credential_recheck": (HANDLER, "authorize_credential(credential);", "(void)credential;"),
    "missing_zone_extent_census": (HANDLER, "out.zone = in_zone(p, all_zones);", "out.zone = false && in_zone(p, all_zones);"),
}


def run(command: list[str], *, success: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(command, text=True, capture_output=True, timeout=180, check=False)
    if success and result.returncode:
        raise RuntimeError(f"command failed: {command[0]}\n{result.stdout[:2000]}\n{result.stderr[:8000]}")
    return result


def compile_and_run(compiler: str, root: Path, work: Path) -> tuple[subprocess.CompletedProcess[str], Path]:
    include = work / "include"
    include.mkdir(parents=True, exist_ok=True)
    # The actual translation unit is compiled unmodified. Only SDK and generated
    # message includes resolve to empty headers; stubs.h supplies non-ABI doubles.
    for name in re.findall(r'#include "([^"\n]+)"', (root / HANDLER).read_text(encoding="utf-8")):
        if name.startswith(".."):
            continue
        target = include / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text("#pragma once\n", encoding="utf-8")
    binary = work / "workshop-native-test"
    run([compiler, *FLAGS, f"-I{include}", str(root / TEST), "-o", str(binary)])
    return run([str(binary)], success=False), binary


def field(data: bytes, offset: int) -> tuple[bytes, int]:
    n = int.from_bytes(data[offset:offset + 2], "big")
    return data[offset + 2:offset + 2 + n], offset + 2 + n


def independent_commitments(vectors: dict[str, bytes]) -> None:
    """Recompute plan/token/receipt commitments with hashlib, not the C++ code."""
    capture = vectors["capture"]
    if capture[:8] != b"DFMWC020":
        raise AssertionError("capture profile differs")
    folder, offset = field(capture, 64)
    if folder != b"region1":
        raise AssertionError("capture fortress folder differs from the specified fixture")
    offset += 4
    selection = capture[offset:offset + 17]
    if selection != b"\x01" + (42).to_bytes(4, "big") + b"".join(n.to_bytes(4, "big") for n in (15, 15, 2)):
        raise AssertionError("selection bytes differ from independently specified fixture")
    witness = hashlib.sha256(capture).digest()
    plan = hashlib.sha256(b"dfmcp-workshop-plan/1\0" + selection + witness).digest()
    key = b"workshop-001"
    token = hashlib.sha256(b"dfmcp-workshop-token/1\0" + len(key).to_bytes(2, "big") + key + plan).digest()[:16]
    if vectors["plan"] != plan or vectors["token"] != token:
        raise AssertionError("native plan/token differ from hashlib commitments")
    phases = {"prepared": 0, "indeterminate": 1, "placed": 2, "refused": 3, "cancelled": 4}
    for name, phase in phases.items():
        record = vectors[name]
        body, digest = record[:-32], record[-32:]
        if record[:8] != b"DFMWR020" or hashlib.sha256(b"dfmcp-workshop-receipt/1\0" + body).digest() != digest:
            raise AssertionError(f"{name} receipt integrity differs")
        stored_key, at = field(record, 8)
        stored_capture, at = field(record, at)
        if stored_key != key or stored_capture != capture or record[at:at + 32] != plan or record[at + 32:at + 48] != token:
            raise AssertionError(f"{name} record is not bound to the original plan")
        if record[at + 48] != phase:
            raise AssertionError(f"{name} phase byte differs")
    for name, present in (("progress_stage0", 1), ("progress_complete", 1), ("progress_absent", 0)):
        progress = vectors[name]
        if progress[:8] != b"DFMWP020":
            raise AssertionError(f"{name} progress profile differs")
        _, at = field(progress, 8 + 16 + 4 + 12)
        if progress[at + 12] != present:
            raise AssertionError(f"{name} presence differs")


def check(compiler: str, mutations: bool, selected: list[str] | None, emit: Path | None) -> dict:
    compiler_path = shutil.which(compiler)
    if compiler_path is None:
        raise RuntimeError(f"requested C++ compiler unavailable: {compiler}")
    if selected and any(name not in MUTATIONS for name in selected):
        raise ValueError("unknown mutation; available: " + ", ".join(MUTATIONS))
    source_hashes = {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest()
                     for name in INPUTS if (ROOT / name).exists()}
    contract = json.loads((ROOT / "architecture/workshop_construction_v1_20.json").read_text(encoding="utf-8"))
    if contract["status"] != "implemented_unadmitted_development_only" or contract["production_admitted"] is not False:
        raise RuntimeError("workshop contract must remain unadmitted")
    if contract["methods"][-1] != "ReadWorkshopProgress" or contract["native_writer_calls_per_key"] != 1:
        raise RuntimeError("workshop contract method set or writer bound changed")
    with tempfile.TemporaryDirectory(prefix="dfmcp-workshop-native-") as directory:
        work = Path(directory)
        result, binary = compile_and_run(compiler_path, ROOT, work)
        if result.returncode:
            raise RuntimeError(f"actual native handler regression failed:\n{result.stdout[:2000]}\n{result.stderr[:8000]}")
        report = json.loads(result.stdout)
        emitted = json.loads(run([str(binary), "--vectors"]).stdout)
        vectors = {name: bytes.fromhex(value) for name, value in emitted.items()}
        independent_commitments(vectors)
        if emit is not None:
            emit.write_text(json.dumps(emitted, indent=1, sort_keys=True) + "\n", encoding="utf-8")
        else:
            retained = json.loads((ROOT / VECTORS).read_text(encoding="utf-8"))
            if retained != emitted:
                raise AssertionError("checked-in workshop vectors differ from actual handler bytes")
        killed = []
        if mutations or selected:
            for label, (path, before, after) in MUTATIONS.items():
                if selected and label not in selected:
                    continue
                copy = work / label
                for relative in INPUTS:
                    if not (ROOT / relative).exists():
                        continue
                    target = copy / relative
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(ROOT / relative, target)
                target = copy / path
                source = target.read_text(encoding="utf-8")
                if source.count(before) != 1:
                    raise RuntimeError(f"mutation no longer applies exactly once: {label}")
                target.write_text(source.replace(before, after), encoding="utf-8")
                mutant, _ = compile_and_run(compiler_path, copy, copy)
                if mutant.returncode == 0 or "check failed:" not in mutant.stderr:
                    raise RuntimeError(f"mutant did not fail a regression assertion: {label}\n{mutant.stderr[:4000]}")
                killed.append(label)
        observed = {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest()
                    for name in INPUTS if (ROOT / name).exists()}
        if observed != source_hashes:
            raise RuntimeError("source changed while native evidence was being captured")
        return {
            "schema": "dfmcp.workshop-native-evidence/1",
            "status": "passed_sdk_doubles_only",
            "actual_cpp_handler_executed": True,
            "compiler": run([compiler_path, "--version"]).stdout.splitlines()[0],
            "flags": FLAGS,
            "actual_handler": report,
            "hashlib_verified_vectors": len(vectors),
            "rejected_compiled_mutants": killed,
            "source_sha256": source_hashes,
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
    parser.add_argument("--mutation", action="append", help="run one named mutation; repeatable")
    parser.add_argument("--emit-vectors", type=Path, help="write actual handler vectors instead of comparing")
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    result = check(args.compiler, args.mutations, args.mutation, args.emit_vectors)
    text = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.evidence:
        args.evidence.write_text(text, encoding="utf-8")
    print(text, end="")


if __name__ == "__main__":
    main()
