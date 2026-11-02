#!/usr/bin/env python3
"""Run the Manager 1.23 handler with explicit SDK doubles, never live/ABI proof."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

from read_stable_repository_file import read_stable_regular_file

ROOT = Path(__file__).resolve().parents[1]
ENGINE = "bridge/common/manager_appointment.h"
TRANSACTION = "bridge/common/one_attempt_native.h"
HANDLER = "bridge/dfhack-manager-v1_23/dfmcp_manager_v1_23.cpp"
TEST = "tests/native/manager_v1_23/handler.cpp"
VECTORS = "tests/fixtures/manager_v1_23_component.json"
INPUTS = [
    ENGINE, TRANSACTION,
    "bridge/common/workshop_construction.h", "bridge/common/retained_snapshot.h",
    HANDLER, "bridge/dfhack-manager-v1_23/DfmcpManagerV1_23.proto",
    "bridge/dfhack-manager-v1_23/CMakeLists.txt", TEST,
    "tests/native/manager_v1_23/stubs.h", VECTORS, "scripts/test_manager_v1_23_native.py",
    "scripts/read_stable_repository_file.py",
]
FLAGS = ["-std=c++17", "-Wall", "-Wextra", "-Werror", "-pedantic", "-g",
         "-fsanitize=undefined", "-fno-sanitize-recover=all"]
MUTATIONS = {
    "missing_production_cache": (HANDLER, "production.push_back(a);", "(void)production;"),
    "empty_event_flag_storage": (ENGINE, "e.flags=std::string(1,'\\0');", "e.flags=std::string();"),
    "missing_history_link": (HANDLER,
        "df::global::world->history.events.push_back(event.release());", "(void)event;"),
    "terminal_replay_not_immutable": (TRANSACTION,
        "if (r.phase != Phase::Prepared) return r;", "if (r.phase == Phase::Cancelled) return r;"),
}


def inventory(root):
    result = {}
    for name in INPUTS:
        value = read_stable_regular_file(root / name, 4 * 1024 * 1024, name)
        text = value.content.decode("utf-8", errors="strict")
        if "\0" in text:
            raise ValueError("NUL in Manager component input: " + name)
        result[name] = value.sha256
    return result


def commitments(values):
    """Independent hashlib commitments; retained vectors must also match exactly."""
    if set(values) != {"capture", "prepared", "plan", "token", "assigned"}:
        raise ValueError("Manager vector keys differ")
    vectors = {name: bytes.fromhex(value) for name, value in values.items()}
    capture = vectors["capture"]
    if capture[:8] != b"DFMMC023":
        raise ValueError("Manager capture profile differs")
    folder_size = int.from_bytes(capture[56:58], "big")
    selection_at = 58 + folder_size
    selection = capture[selection_at:selection_at + 8]
    if selection != (256).to_bytes(4, "big") + (361).to_bytes(4, "big"):
        raise ValueError("Manager fixture selection differs")
    digest = lambda domain, data: hashlib.sha256(domain + b"\0" + data).digest()
    plan = digest(b"dfmcp-manager-plan/1", selection + hashlib.sha256(capture).digest())
    key = b"manager-001"
    token = digest(b"dfmcp-manager-token/1", len(key).to_bytes(2, "big") + key + plan)[:16]
    if vectors["plan"] != plan or vectors["token"] != token:
        raise ValueError("Manager plan/token commitments differ")
    for name, phase in [("prepared", 0), ("assigned", 2)]:
        record = vectors[name]
        body, checksum = record[:-32], record[-32:]
        if record[:8] != b"DFMMR023" or digest(b"dfmcp-manager-receipt/1", body) != checksum:
            raise ValueError("Manager record integrity differs: " + name)
        at = 8
        n = int.from_bytes(body[at:at + 2], "big"); at += 2
        if body[at:at + n] != key:
            raise ValueError("Manager original key differs")
        at += n
        if body[at:at + 32] != plan or body[at + 32:at + 48] != token:
            raise ValueError("Manager original commitment differs")
        at += 48
        n = int.from_bytes(body[at:at + 4], "big"); at += 4
        if body[at:at + n] != capture or body[at + n] != phase:
            raise ValueError("Manager record capture/phase differs")


def check(compiler, mutations, output):
    compiler = shutil.which(compiler)
    if compiler is None:
        raise ValueError("requested C++ compiler is unavailable")
    before = inventory(ROOT)
    expected = {"scenarios": 13, "assertions": 308, "ignored": 0,
                "sdk_doubles_only": True, "live_game": False}
    sequence = 0

    def command(argv):
        nonlocal sequence
        sequence += 1
        stem = output / f"{sequence:03d}"
        (stem.with_suffix(".argv.json")).write_text(json.dumps(argv) + "\n")
        with stem.with_suffix(".stdout").open("x") as stdout, stem.with_suffix(".stderr").open("x") as stderr:
            result = subprocess.run(argv, stdout=stdout, stderr=stderr, timeout=180, check=False)
        stem.with_suffix(".exit").write_text(str(result.returncode) + "\n")
        return result.returncode, stem.with_suffix(".stdout").read_text(), stem.with_suffix(".stderr").read_text()

    def execute(root, work):
        include = work / "include"
        for name in re.findall(r'#include "([^"\n]+)"', (root / HANDLER).read_text()):
            if name.startswith(".."):
                continue
            header = include / name
            header.parent.mkdir(parents=True, exist_ok=True)
            header.write_text("#pragma once\n")
        binary = work / "manager-native-test"
        code, stdout, stderr = command([compiler, *FLAGS, f"-I{include}", str(root / TEST), "-o", str(binary)])
        if code:
            raise RuntimeError("Manager component compile failed:\n" + stdout + stderr)
        return command([str(binary)]), binary

    killed = []
    with tempfile.TemporaryDirectory(prefix="dfmcp-manager-native-") as temporary:
        work = Path(temporary)
        (code, stdout, stderr), binary = execute(ROOT, work)
        if code:
            raise RuntimeError("Manager component failed:\n" + stdout + stderr)
        report = json.loads(stdout)
        if report != expected:
            raise ValueError("Manager component denominator/proof class differs")
        code, stdout, stderr = command([str(binary), "--vectors"])
        if code:
            raise RuntimeError("Manager vectors failed:\n" + stdout + stderr)
        emitted = json.loads(stdout)
        commitments(emitted)
        if emitted != json.loads((ROOT / VECTORS).read_text()):
            raise ValueError("retained Manager vectors differ from actual handler")
        if mutations:
            for label, (name, old, new) in MUTATIONS.items():
                copy = work / label
                for source in INPUTS:
                    target = copy / source
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(ROOT / source, target)
                target = copy / name
                text = target.read_text()
                if text.count(old) != 1:
                    raise ValueError("Manager mutation does not apply once: " + label)
                target.write_text(text.replace(old, new))
                (code, stdout, stderr), _ = execute(copy, copy)
                if code == 0 or "check failed:" not in stderr:
                    raise RuntimeError("compiled Manager mutant survived: " + label + "\n" + stdout + stderr)
                killed.append(label)
    if inventory(ROOT) != before:
        raise ValueError("Manager inputs changed during component execution")
    return {"status": "passed_sdk_doubles_only", "actual_handler": report,
            "rejected_compiled_mutants": killed, "source_sha256": before,
            "real_dfhack_sdk": False, "live_fortress": False, "rust_executed": False}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default="g++")
    parser.add_argument("--mutations", action="store_true")
    args = parser.parse_args()
    output = ROOT / "target/manager-v1-23-native-component" / f"{time.time_ns()}-{os.getpid()}"
    output.mkdir(mode=0o700, parents=True)
    result = check(args.compiler, args.mutations, output)
    (output / "component.json").write_text(json.dumps(result, sort_keys=True, indent=2) + "\n")
    print(json.dumps(result, sort_keys=True))
