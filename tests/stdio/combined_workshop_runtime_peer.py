#!/usr/bin/env python3
"""Actual Rust Runtime against existing Workshop and production native peers.

The Rust child seeds real private donor custody through WorkshopRpc, then opens
the production Runtime in separate children. This driver supplies native TCP
doubles and isolated operator environments. It never implements an MCP server.
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
ROOT = Path(__file__).resolve().parents[2]
PHASE = "COMBINED_WORKSHOP_RUNTIME_PHASE"
CASE = "COMBINED_WORKSHOP_RUNTIME_CASE"
INPUT = "COMBINED_WORKSHOP_RUNTIME_INPUT"
OUTPUT = "COMBINED_WORKSHOP_RUNTIME_OUTPUT"


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def snapshot(directory):
    return {
        path.name: (path.stat().st_mode & 0o7777, hashlib.sha256(path.read_bytes()).hexdigest())
        for path in sorted(directory.iterdir())
    }


def run_child(executable, test_name, phase, case, environment, source, destination):
    environment = environment.copy()
    environment.update({
        PHASE: phase, CASE: case, INPUT: str(source), OUTPUT: str(destination),
        "PYTHONDONTWRITEBYTECODE": "1",
    })
    result = subprocess.run(
        [executable, "--exact", test_name, "--nocapture"],
        env=environment, timeout=180, capture_output=True, text=True, check=False,
    )
    assert result.returncode == 0, (
        f"{phase}/{case} failed:\n{result.stdout}\n{result.stderr}"
    )
    assert f"combined-workshop-runtime:{phase}:{case}:checked" in result.stdout


def run_case(executable, test_name, case):
    workshop = load("combined_workshop_native_fixture", "workshop.py")
    combined = load("combined_workshop_production_fixture", "combined_production.py")
    workshop.VECTORS = {
        key: bytes.fromhex(value)
        for key, value in json.loads((ROOT / "tests/native/workshop/vectors.json").read_text()).items()
    }
    original_progress = workshop.VECTORS["progress_complete"]
    tick = int.from_bytes(original_progress[16:24], "big")
    capture = workshop.VECTORS["capture"]
    combined.SITE = int.from_bytes(capture[32:36], "big")
    combined.DF, combined.DFH, combined.TICK = b"test-df", b"test-dfhack", tick
    # The existing encoder captured its old module clock as a keyword default.
    # Keep its exact wire encoding, but use this joined fixture's paused tick.
    queue_encoder = combined.order_queue

    def joined_order_queue(created=False, *, tick=tick, sequence=None, retired=False):
        return queue_encoder(created, tick=tick, sequence=sequence, retired=retired)

    combined.order_queue = joined_order_queue

    def change_progress(vector):
        current = bytearray(workshop.VECTORS[vector])
        current[16:24] = tick.to_bytes(8, "big")
        workshop.VECTORS["progress_complete"] = bytes(current)

    class WorkshopPeer(workshop.Peer):
        generation_drift = False

        def reply(self, stream, raw):
            fields = workshop.fields(raw)
            if self.generation_drift and 6 in fields:
                fields[6] += 1
                raw = b"".join(
                    workshop.number(tag, value) if isinstance(value, int)
                    else workshop.blob(tag, value)
                    for tag, value in fields.items()
                )
            super().reply(stream, raw)

    class ProductionPeer(combined.Peer):
        def reply(self, plugin, operation, request):
            try:
                raw = super().reply(plugin, operation, request)
            except AssertionError as cause:
                raise AssertionError(f"{plugin}/{operation} native fixture assertion") from cause
            if case == "before_prepare" and operation == "CommitAssignment":
                change_progress("progress_absent")
            if case == "after_prepare" and operation == "PrepareOrder":
                change_progress("progress_absent")
            if case == "lost" and operation == "CommitOrder":
                raise ConnectionResetError("lost reply after the original order insertion")
            return raw

    with tempfile.TemporaryDirectory(prefix="dfmcp-workshop-production-") as temporary:
        directory = Path(temporary).resolve()
        donor = directory / "donor"
        donor.mkdir(mode=0o700)
        custody = donor / "custody"
        custody.mkdir(mode=0o700)
        production = directory / "production"
        production.mkdir(mode=0o700)
        workshop_peer = WorkshopPeer(donor)
        production_peer = ProductionPeer(production)
        try:
            seed = {
                "directory": str(custody), "endpoint": workshop_peer.endpoint,
                "world_folder": "region1", "site_id": combined.SITE,
                "df_version": "test-df", "dfhack_version": "test-dfhack",
                "generation": int.from_bytes(capture[8:16], "big"),
                "map_dimensions": [int.from_bytes(capture[i:i + 4], "big") for i in (36, 40, 44)],
                "capture_tick": int.from_bytes(capture[24:32], "big"),
            }
            source, prerequisite = directory / "seed.json", directory / "prerequisite.json"
            source.write_text(json.dumps(seed))
            clean_environment = {key: value for key, value in os.environ.items()
                                 if not key.startswith("DFMCP_")}
            run_child(executable, test_name, "seed", case, clean_environment, source, prerequisite)
            donor_before = snapshot(custody)
            assert set(donor_before) == {"workshop.journal", "workshop.ack"}
            assert all(mode == 0o600 for mode, _ in donor_before.values())
            assert workshop_peer.writer_count == 1
            donor_calls = len(workshop_peer.calls)
            environment = combined.environment(production, production_peer.endpoint)
            parent = json.loads(environment["DFMCP_COMBINED_PRODUCTION_CONFIG"])
            parent["workshop_prerequisite"] = json.loads(prerequisite.read_text())
            environment["DFMCP_COMBINED_PRODUCTION_CONFIG"] = json.dumps(parent)
            environment["DFMCP_COMBINED_PRODUCTION_WORKSHOP_TOKEN"] = "t" * 32
            if case == "unfinished":
                change_progress("progress_stage0")
            elif case == "removed":
                change_progress("progress_absent")
            elif case == "generation":
                workshop_peer.generation_drift = True
            retained_path = directory / "retained.json"
            run_child(executable, test_name, "control", case, environment,
                      prerequisite, retained_path)
            calls = [operation for _, operation in production_peer.calls]
            if case in ("unfinished", "removed", "generation"):
                assert not any(operation.startswith(("Prepare", "Commit")) for operation in calls)
                assert not production_peer.order_created and not production_peer.workforce_applied
            else:
                assert calls.count("PrepareAssignment") == calls.count("CommitAssignment") == 1
                assert production_peer.workforce_applied
                if case == "before_prepare":
                    assert "PrepareOrder" not in calls and "CommitOrder" not in calls
                elif case == "after_prepare":
                    assert calls.count("PrepareOrder") == 1 and "CommitOrder" not in calls
                else:
                    assert calls.count("PrepareOrder") == calls.count("CommitOrder") == 1
                    assert production_peer.order_created
            if case == "removed":
                retained = json.loads(retained_path.read_text())
                checkpoint = retained["workshop_prerequisite"]["checkpoint"]
                assert checkpoint["fenced"] is True
                checkpoint_environment = combined.reopen_environment(
                    environment, None, retained["native_custody"]["workforce_journal_id"],
                )
                parent = json.loads(checkpoint_environment["DFMCP_COMBINED_PRODUCTION_CONFIG"])
                parent["workshop_prerequisite_checkpoint"] = checkpoint
                checkpoint_environment["DFMCP_COMBINED_PRODUCTION_CONFIG"] = json.dumps(parent)
                checkpoint_environment.pop("DFMCP_COMBINED_PRODUCTION_WORKSHOP_TOKEN")
                # A now-complete native source cannot erase the consumed fence.
                workshop.VECTORS["progress_complete"] = original_progress
                workshop_calls = list(workshop_peer.calls)
                run_child(executable, test_name, "checkpoint", case, checkpoint_environment,
                          retained_path, directory / "checkpoint.json")
                assert workshop_calls == workshop_peer.calls
                assert not any(operation.startswith(("Prepare", "Commit"))
                               for _, operation in production_peer.calls)
                parent["workshop_prerequisite_checkpoint"] = {
                    **checkpoint, "origin_witness": hashlib.sha256(b"another original donor").hexdigest(),
                }
                checkpoint_environment["DFMCP_COMBINED_PRODUCTION_CONFIG"] = json.dumps(parent)
                before_calls = (list(production_peer.calls), list(workshop_peer.calls))
                run_child(executable, test_name, "reject", "checkpoint-donor",
                          checkpoint_environment, retained_path, directory / "wrong-checkpoint.json")
                assert before_calls == (production_peer.calls, workshop_peer.calls)
            if case == "lost":
                retained = json.loads(retained_path.read_text())
                reopened = combined.reopen_environment(
                    environment, retained["recovery"],
                    retained["native_custody"]["workforce_journal_id"],
                )
                offline = {key: value for key, value in reopened.items()
                           if not (key.startswith("DFMCP_") and (
                               key.endswith("_TOKEN") or key.endswith("_ALLOW_LABOR")
                               or key.endswith("_ALLOW_PRODUCTION")))}
                before_calls = (list(production_peer.calls), list(workshop_peer.calls))
                before_files = combined.custody_files(production)
                run_child(executable, test_name, "offline", case, offline, retained_path,
                          directory / "offline.json")
                assert before_calls == (production_peer.calls, workshop_peer.calls)
                assert before_files == combined.custody_files(production)
                for alteration in ("drop", "swap", "drop-all"):
                    invalid = offline.copy()
                    parent = json.loads(invalid["DFMCP_COMBINED_PRODUCTION_CONFIG"])
                    if alteration in ("drop", "drop-all"):
                        parent.pop("workshop_prerequisite")
                    else:
                        parent["workshop_prerequisite"]["original_key"] = "substituted-original"
                    association = production / "parent/original.workshop"
                    hidden = directory / "saved-original-workshop"
                    if alteration == "drop-all":
                        parent["recovery"].pop("workshop_prerequisite_digest")
                        parent["recovery"].pop("workshop_prerequisite_floor")
                        association.rename(hidden)
                    invalid["DFMCP_COMBINED_PRODUCTION_CONFIG"] = json.dumps(parent)
                    try:
                        run_child(executable, test_name, "reject", alteration, invalid,
                                  retained_path, directory / f"rejected-{alteration}.json")
                    finally:
                        if alteration == "drop-all":
                            hidden.rename(association)
                ack = custody / "workshop.ack"
                saved = ack.read_bytes()
                damaged = bytearray(saved)
                damaged[-1] ^= 1
                try:
                    ack.write_bytes(damaged)
                    run_child(executable, test_name, "reject", "custody", offline,
                              retained_path, directory / "rejected-custody.json")
                finally:
                    ack.write_bytes(saved)
                assert before_calls == (production_peer.calls, workshop_peer.calls)
                assert before_files == combined.custody_files(production)
                run_child(executable, test_name, "reconcile", case, reopened, retained_path,
                          directory / "reconciled.json")
                final_calls = [operation for _, operation in production_peer.calls]
                assert final_calls.count("PrepareOrder") == final_calls.count("CommitOrder") == 1
                assert final_calls.count("CommitAssignment") == 1
                assert "QueryOrder" in final_calls[len(calls):]
                assert production_peer.order_plan["key"].decode() == retained["original_plan"]["steps"][1]["key"]
            assert donor_before == snapshot(custody), "production changed the donor journal or ACK"
            assert workshop_peer.writer_count == 1
            assert all(operation not in (2, 3, 5) for operation in workshop_peer.calls[donor_calls:])
            assert not production_peer.clock_commits and not production_peer.inventory_clock_commits
        finally:
            try:
                production_peer.close()
            finally:
                workshop_peer.close()


def main():
    executable, test_name, kind = sys.argv[1:]
    assert kind in ("gates", "recovery")
    cases = ("success", "unfinished", "removed", "generation", "before_prepare", "after_prepare") \
        if kind == "gates" else ("lost",)
    for case in cases:
        run_case(executable, test_name, case)
    print(f"combined-workshop-campaign:{kind}:checked")


if __name__ == "__main__":
    main()
