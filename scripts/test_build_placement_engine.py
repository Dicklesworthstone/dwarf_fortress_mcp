#!/usr/bin/env python3
"""Compile and execute the actual furniture engine, plus independent wire goldens.

SDK-free C++ tests are not a DFHack build, a live campaign, or production admission.
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
HEADER = Path("bridge/common/build_placement.h")
HELPER = Path("bridge/common/retained_snapshot.h")
TEST = Path("bridge/common/tests/build_placement_test.cpp")
FIXTURE = Path("bridge/common/tests/fixtures/build_placement_v1_19.json")
FLAGS = ["-std=c++17", "-Wall", "-Wextra", "-Werror", "-pedantic", "-g",
         "-fsanitize=undefined", "-fno-sanitize-recover=all"]


def digest(domain: bytes, data: bytes) -> bytes:
    return hashlib.sha256(domain + b"\0" + data).digest()


def u32(*values: int) -> bytes:
    return struct.pack(">" + "I" * len(values), *values)


def field(value: bytes) -> bytes:
    return struct.pack(">H", len(value)) + value


def golden_vectors() -> dict[str, str]:
    """Construct all native fields independently; never parse C++ output as truth."""
    selection = b"\1" + u32(42, 15, 15, 2)

    def tile(*, occupied: bool = False, other: int = 0) -> bytes:
        return (b"\2" + u32(37) + bytes([3, 0, 0]) + u32(other)
                + bytes([occupied, occupied]) + (u32(70) if occupied else b""))

    def capture(after: bool) -> bytes:
        item = (b"\2" + u32(10, 11, 2) + b"\1" + u32(101)
                + struct.pack(">iii", -1, 419, -1) + u32(2, 0, 0, 0)
                + bytes([1, after, after]) + (u32(90) if after else b"") + tile(other=8))
        return (b"DFMBC019" + struct.pack(">QQQ", 41, int(after), 806500)
                + u32(2, 64, 64, 8, 70 + after, 90 + after, 4 + after)
                + field(b"region1") + bytes([1, not after, 1]) + selection
                + b"".join(tile(occupied=after and n == 4) for n in range(9)) + item)

    before, after = capture(False), capture(True)
    plan = digest(b"dfmcp-build-plan/1", selection + hashlib.sha256(before).digest())
    token = digest(b"dfmcp-build-token/1", field(b"golden") + plan)[:16]
    insertion = (b"DFMBI019" + u32(70, 90, 42) + b"\1" + u32(15, 15, 2)
                 + struct.pack(">ii", 419, -1) + u32(0, 1) + bytes([1, 1, 1, 0]))

    def record(phase: int, reason: int) -> bytes:
        raw = (b"DFMBR019" + field(b"golden") + field(before) + plan + token
               + bytes([phase, reason, phase in (1, 2), phase == 2]))
        if phase == 2:
            raw += field(after) + field(insertion)
        return raw + digest(b"dfmcp-build-receipt/1", raw)

    return {name: value.hex() for name, value in {
        "capture": before, "plan": plan, "token": token,
        "prepared": record(0, 0), "placed": record(2, 0),
        "indeterminate": record(1, 5), "expired": record(3, 2), "cancelled": record(4, 4),
    }.items()}


def run(command: list[str], *, cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(command, cwd=cwd, text=True, capture_output=True, timeout=90, check=False)
    if result.returncode:
        raise RuntimeError(f"command failed: {command[0]}\n{result.stdout}\n{result.stderr}")
    return result


def git_blob(path: Path) -> str:
    data = path.read_bytes()
    return hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()


def check(compiler: str, mutations: bool, selected: list[str] | None = None) -> dict:
    expected = golden_vectors()
    retained = json.loads((ROOT / FIXTURE).read_text(encoding="utf-8"))
    if retained != expected:
        raise RuntimeError("checked-in native vectors differ from independent Python construction")
    with tempfile.TemporaryDirectory(prefix="dfmcp-build-engine-") as raw:
        temp = Path(raw)
        executable = temp / "build-placement-test"
        run([compiler, *FLAGS, str(ROOT / TEST), "-o", str(executable)])
        result = json.loads(run([str(executable)]).stdout)
        actual = json.loads(run([str(executable), "--vectors"]).stdout)
        if actual != expected:
            raise RuntimeError("actual C++ bytes differ from independently constructed native vectors")
        killed = []
        changes = {
            "missing_full_revalidation": ("current.encode() != r.before.encode()", "false"),
            "missing_uncertainty_fence": ("require(!unresolved_, 8);", "require(true, 8);"),
            "terminal_replay_not_immutable": ("if (r.phase != Phase::Prepared) return r;",
                "if (r.phase == Phase::Cancelled || r.phase == Phase::Refused) return r;"),
            "missing_final_readback": (" || inspect(r.before.selection, read).encode() != expected", ""),
        }
        if selected and any(name not in changes for name in selected):
            raise ValueError("unknown mutation")
        if mutations or selected:
            for name, (needle, replacement) in changes.items():
                if selected and name not in selected:
                    continue
                tree = temp / name
                for path in (HEADER, HELPER, TEST):
                    destination = tree / path
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(ROOT / path, destination)
                header = tree / HEADER
                text = header.read_text(encoding="utf-8")
                if needle not in text:
                    raise RuntimeError(f"mutation point disappeared: {name}")
                header.write_text(text.replace(needle, replacement), encoding="utf-8")
                binary = tree / "regression"
                run([compiler, *FLAGS, str(tree / TEST), "-o", str(binary)])
                outcome = subprocess.run([str(binary)], text=True, capture_output=True, timeout=30, check=False)
                if outcome.returncode == 0 or "check failed:" not in outcome.stderr:
                    raise RuntimeError(f"mutation did not fail a regression assertion: {name}\n{outcome.stderr}")
                killed.append(name)
        return {
            "schema": "dfmcp.build-engine-evidence/1", "actual_cpp_executed": True,
            "compiler": run([compiler, "--version"]).stdout.splitlines()[0], "flags": FLAGS,
            **result, "independent_vectors": len(expected), "mutations_killed": killed,
            "blob_sha1": {str(p): git_blob(ROOT / p) for p in (HEADER, HELPER, TEST, FIXTURE)},
            "vector_bytes": {key: len(bytes.fromhex(value)) for key, value in expected.items()},
            "real_dfhack_sdk": False, "live_fortress": False,
            "rust_executed": False, "full_repository_qualification": False,
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
