#!/usr/bin/env python3
"""Exercise the actual compiled child publication callback and durable recovery.

Existing native peers supply source bytes. Rust owns compilation, original
files, source associations and execution journals; no MCP server is simulated.
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def child(executable, test, phase, case, env, source, output):
    env = env.copy()
    env.update({
        "PROJECT_PUBLICATION_RUNTIME_PHASE": phase,
        "PROJECT_PUBLICATION_RUNTIME_CASE": case,
        "PROJECT_PUBLICATION_RUNTIME_INPUT": str(source),
        "PROJECT_PUBLICATION_RUNTIME_OUTPUT": str(output),
        "PYTHONDONTWRITEBYTECODE": "1",
    })
    result = subprocess.run([executable, "--exact", test, "--nocapture"],
                            env=env, timeout=180, capture_output=True, text=True, check=False)
    assert result.returncode == 0, f"{phase}/{case}:\n{result.stdout}\n{result.stderr}"
    assert f"project-publication-runtime:{phase}:{case}:checked" in result.stdout


def run_case(executable, test, case):
    combined = load("project_publication_production_peer", "combined_production.py")
    stock = load("project_publication_workshop_seed", "combined_stock_target_runtime_peer.py")
    with tempfile.TemporaryDirectory(prefix="dfmcp-project-publication-") as temporary:
        directory = Path(temporary).resolve()
        donor = stock.seed_workshop(executable, combined, directory)
        workshop, custody, donor_before, donor_calls, prerequisite, helper = donor
        production = directory / "production"
        production.mkdir(mode=0o700)
        original_capture = combined.workforce_capture

        def carpenter_capture(ids=(42,), option_count=0):
            # Observe and Prepare must use identical native bytes. The generic
            # peer's observation-only hook would leave Prepare pinned to MINE.
            capture = combined.wf.capture(original_capture(ids, option_count))
            capture["labor_keys"] = ["CARPENTER", "MINE"]
            return combined.wf.encode_capture(capture)

        combined.workforce_capture = carpenter_capture
        peer = combined.Peer(production)
        try:
            peer.inventory_stack = 6 if case == "no_work" else 2
            environment = combined.environment(production, peer.endpoint)
            parent = json.loads(environment["DFMCP_COMBINED_PRODUCTION_CONFIG"])
            parent["workshop_prerequisite"] = prerequisite
            environment["DFMCP_COMBINED_PRODUCTION_CONFIG"] = json.dumps(parent)
            environment["DFMCP_COMBINED_PRODUCTION_WORKSHOP_TOKEN"] = "t" * 32
            retained_path = directory / "publication.json"
            child(executable, test, "control", case, environment, retained_path, retained_path)
            retained = json.loads(retained_path.read_text())
            calls = [operation for _, operation in peer.calls]
            assert calls.count("PrepareAssignment") == int(case == "advanced")
            assert not any(operation.startswith("Commit") for operation in calls)
            assert "PrepareOrder" not in calls
            assert not peer.workforce_applied and not peer.order_created
            if case in ("published", "partial", "advanced"):
                reopened = combined.reopen_environment(
                    environment, None, retained["workforce_journal_id"])
                offline = {key: value for key, value in reopened.items()
                           if not (key.startswith("DFMCP_") and (
                               key.endswith("_TOKEN") or key.endswith("_ALLOW_LABOR")
                               or key.endswith("_ALLOW_PRODUCTION")))}
                calls_before = (list(peer.calls), list(workshop.calls))
                files_before = combined.custody_files(production)
                child(executable, test, "recover", case, offline, retained_path,
                      directory / "recovered.json")
                assert calls_before == (peer.calls, workshop.calls), "recovery contacted a native source"
                assert files_before == combined.custody_files(production), "recovery repaired or rewrote custody"
                if case == "published":
                    for name in ("original.plan", "original.citizens", "original.workshop", "execution.journal"):
                        path = production / "parent" / name
                        hidden = directory / f"saved-{name}"
                        path.rename(hidden)
                        try:
                            absent = combined.custody_files(production)
                            child(executable, test, "recover", f"missing-{name}", offline,
                                  retained_path, directory / "refused.json")
                            assert absent == combined.custody_files(production)
                        finally:
                            hidden.rename(path)
                        original = path.read_bytes()
                        try:
                            damaged = bytearray(original)
                            damaged[-1] ^= 1
                            path.write_bytes(damaged)
                            corrupt = combined.custody_files(production)
                            child(executable, test, "recover", f"corrupt-{name}", offline,
                                  retained_path, directory / "refused.json")
                            assert corrupt == combined.custody_files(production)
                        finally:
                            path.write_bytes(original)
                    foreign = json.loads(retained_path.read_text())
                    foreign["intent"]["recovery_reference"]["recovery"]["object_digest"] = \
                        hashlib.sha256(b"another complete stock-target original").hexdigest()
                    foreign_path = directory / "foreign.json"
                    foreign_path.write_text(json.dumps(foreign))
                    child(executable, test, "recover", "foreign-original", offline,
                          foreign_path, directory / "refused.json")
                    assert files_before == combined.custody_files(production)
                    assert calls_before == (peer.calls, workshop.calls)
            assert donor_before == helper.snapshot(custody)
            assert workshop.writer_count == 1
            assert all(operation not in (2, 3, 5) for operation in workshop.calls[donor_calls:])
            assert not peer.clock_commits and not peer.inventory_clock_commits
        finally:
            try:
                peer.close()
            finally:
                workshop.close()


def main():
    executable, test = sys.argv[1:]
    for case in ("no_work", "ack_refusal", "published", "partial", "advanced"):
        run_case(executable, test, case)
    print("project-publication-campaign:checked")


if __name__ == "__main__":
    main()
