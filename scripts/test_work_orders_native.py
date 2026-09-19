#!/usr/bin/env python3
"""Execute the work-orders engine and real handler with explicit SDK/wire doubles.

This is NOT a real DFHack/protobuf build, a live campaign, Rust qualification or
admission. No game is contacted. Outputs include independently verified vectors.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
ENGINE = Path("bridge/common/work_order_creation.h")
HANDLER = Path("bridge/dfhack-work-orders-v1_10/dfmcp_work_orders_v1_10.cpp")
TESTS = Path("tests/native/work_orders")
FIXTURES = Path("crates/dfmcp-adapter/tests/fixtures")
WRAPPERS = ["Core.h", "Export.h", "PluginManager.h", "RemoteServer.h", "VersionInfo.h",
            "modules/World.h", "df/global_objects.h", "df/item_type.h", "df/job_type.h",
            "df/manager_order.h", "df/workquota_frequency_type.h", "df/world.h",
            "DfmcpWorkOrdersV1_10.pb.h"]


def run(args: list[str], timeout: int = 60) -> str:
    result = subprocess.run(args, capture_output=True, text=True, timeout=timeout, check=False)
    if result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}): {args!r}\n{result.stdout}\n{result.stderr}")
    return result.stdout


def pairs(output: str) -> dict[str, str]:
    return dict(line.split("=", 1) for line in output.splitlines() if "=" in line)


def sha(value: bytes) -> bytes:
    return hashlib.sha256(value).digest()


def text(value: bytes) -> bytes:
    return struct.pack(">H", len(value)) + value


def reference_vectors() -> dict[str, bytes]:
    # Independent stdlib reconstruction: do not copy the producer's hashes.
    key = b"order-001"
    observation = (b"DFMWO010" + struct.pack(">QQQIIB", 7, 0, 12345, 10, 1, 1)
                   + text(b"region1") + struct.pack(">III", 2, 2, 6))
    spec = struct.pack(">BI", 1, 5)
    witness = sha(observation)
    plan = sha(b"dfmcp-work-order-plan/1\0" + spec + witness)
    token = sha(b"dfmcp-work-order-token/1\0" + struct.pack(">Q", 7) + text(key) + plan)[:16]
    after = (b"DFMWO010" + struct.pack(">QQQIIB", 7, 1, 12345, 11, 1, 1)
             + text(b"region1") + struct.pack(">IIII", 3, 2, 6, 10))
    configuration = b"DFMWOC10" + struct.pack(">I", 10) + spec
    result = {"observation": observation}
    for name, state in [("prepared", 0), ("unknown", 1), ("created", 2), ("refused", 4)]:
        known = state == 2
        outcome = (struct.pack(">BBQ", state, int(known), 12345 if known else 0)
                   + (sha(after) if known else bytes(32))
                   + (sha(configuration) if known else bytes(32)))
        receipt = (sha(b"dfmcp-work-order-receipt/1\0" + struct.pack(">Q", 7)
                       + text(key) + plan + token + outcome) if state in (2, 4) else bytes(32))
        result[name] = (b"DFMWOE10" + struct.pack(">QQQI", 7, 0, 12345, 10)
                        + spec + witness + plan + token + outcome + receipt + text(key))
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default="g++")
    parser.add_argument("--ubsan", action="store_true")
    parser.add_argument("--mutations", action="store_true")
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    compiler = shutil.which(args.compiler)
    if not compiler:
        raise SystemExit(f"compiler not available: {args.compiler}")
    flags = ["-std=c++17", "-Wall", "-Wextra", "-Werror", "-pedantic", "-O1"]
    if args.ubsan:
        flags += ["-fsanitize=undefined", "-fno-sanitize-recover=all"]
    expected = reference_vectors()
    sources = [ENGINE, HANDLER, Path("bridge/common/retained_snapshot.h"),
               TESTS / "engine.cpp", TESTS / "handler.cpp", TESTS / "stubs.h",
               Path("scripts/test_work_orders_native.py"),
               Path("bridge/dfhack-work-orders-v1_10/DfmcpWorkOrdersV1_10.proto")]
    fingerprints = {str(p): hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in sources}
    killed: list[str] = []
    with tempfile.TemporaryDirectory(prefix="dfmcp-work-orders-") as directory:
        work = Path(directory)
        include = work / "include"
        include.mkdir()
        shutil.copyfile(ROOT / TESTS / "stubs.h", include / "work_orders_stubs.h")
        for name in WRAPPERS:
            target = include / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text('#include "work_orders_stubs.h"\n', encoding="utf-8")
        engine_binary, handler_binary = work / "engine", work / "handler"
        run([compiler, *flags, "-I", str(ROOT / "bridge/common"), str(ROOT / TESTS / "engine.cpp"), "-o", str(engine_binary)])
        engine = pairs(run([str(engine_binary)], timeout=15))
        run([compiler, *flags, "-I", str(include), f'-DWORK_ORDERS_HANDLER="{ROOT / HANDLER}"',
             str(ROOT / TESTS / "handler.cpp"), "-o", str(handler_binary)])
        handler = pairs(run([str(handler_binary)], timeout=15))
        for name, value in expected.items():
            if engine.get(name) != value.hex():
                raise RuntimeError(f"independent native vector mismatch: {name}")
            fixture = ROOT / FIXTURES / f"work_order_{name}_v1_10.hex"
            if fixture.read_text(encoding="ascii").strip() != value.hex():
                raise RuntimeError(f"checked-in native fixture differs: {fixture}")
        if args.mutations:
            source = (ROOT / ENGINE).read_text(encoding="utf-8")
            mutations = {
                "missing_commit_witness": ("valid = current.eligible() && current.witness() == r.witness;", "valid = current.eligible();"),
                "missing_replay_fence": ("if (r.state != State::Prepared) return r;", "/* replay fence deliberately removed */"),
                "unchecked_created_queue": ("after.encode() != expected || verify(r.before.next_order, r.spec) != config", "verify(r.before.next_order, r.spec) != config"),
                "missing_uncertainty_fence": ("require(!unresolved_, 8);", "/* uncertainty fence deliberately removed */"),
            }
            for name, (old, new) in mutations.items():
                if old not in source:
                    raise RuntimeError(f"mutation target disappeared: {name}")
                mutant = work / name
                mutant.mkdir()
                (mutant / ENGINE.name).write_text(source.replace(old, new), encoding="utf-8")
                shutil.copyfile(ROOT / "bridge/common/retained_snapshot.h", mutant / "retained_snapshot.h")
                binary = mutant / "engine"
                run([compiler, *flags, "-I", str(mutant), str(ROOT / TESTS / "engine.cpp"), "-o", str(binary)])
                result = subprocess.run([str(binary)], capture_output=True, text=True, timeout=15, check=False)
                if result.returncode == 0:
                    raise RuntimeError(f"mutation survived: {name}")
                killed.append(name)
    # Do not retain a source fingerprint for bytes that changed during execution.
    if any(hashlib.sha256((ROOT / path).read_bytes()).hexdigest() != value for path, value in fingerprints.items()):
        raise RuntimeError("source changed during native test execution")
    report = {
        "schema": "dfmcp.work-orders-native-double-evidence/1", "status": "passed",
        "compiler": run([compiler, "--version"]).splitlines()[0], "flags": flags,
        "engine_groups": int(engine["engine_groups"]), "engine_assertions": int(engine["engine_assertions"]),
        "handler_groups": int(handler["handler_groups"]), "handler_assertions": int(handler["handler_assertions"]),
        "independent_vectors": len(expected), "rejected_compiled_mutants": killed,
        "source_sha256": fingerprints,
        "evidence_scope": "actual C++ engine/handler with explicit DFHack and protobuf doubles",
        "not_established": ["generated protobuf serialization", "real DFHack field access and suspension",
                            "Rust execution", "live fortress behavior", "durable restart coordination", "admission"],
    }
    encoded = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(encoded, encoding="utf-8")
    print(encoded, end="")


if __name__ == "__main__":
    main()
