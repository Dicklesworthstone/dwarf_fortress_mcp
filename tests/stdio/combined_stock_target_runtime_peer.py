#!/usr/bin/env python3
"""Run the real stock-target Runtime against the existing native TCP doubles.

The child owns all production code and private custody. This driver supplies
isolated configuration and native peers; it never implements an MCP server.
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
PHASE = "COMBINED_STOCK_TARGET_RUNTIME_PHASE"
CASE = "COMBINED_STOCK_TARGET_RUNTIME_CASE"
INPUT = "COMBINED_STOCK_TARGET_RUNTIME_INPUT"
OUTPUT = "COMBINED_STOCK_TARGET_RUNTIME_OUTPUT"
STAGE = "COMBINED_STOCK_TARGET_RUNTIME_STAGE"


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run_child(executable, test_name, phase, case, environment, source, destination):
    environment = environment.copy()
    environment.update({PHASE: phase, CASE: case, INPUT: str(source),
                        OUTPUT: str(destination), "PYTHONDONTWRITEBYTECODE": "1"})
    result = subprocess.run(
        [executable, "--exact", test_name, "--nocapture"], env=environment,
        timeout=180, capture_output=True, text=True, check=False,
    )
    assert result.returncode == 0, (
        f"stock-target {phase}/{case} failed:\n{result.stdout}\n{result.stderr}"
    )
    assert f"combined-stock-target-runtime:{phase}:{case}:checked" in result.stdout


def creation_states(directory):
    """Read actual v1.10 frames to place a source change after durable dispatch.

    No bytes are written or repaired. Checksums and chain ordering distinguish
    the real journal event from an arbitrary matching byte in fixture custody.
    """
    path = directory / "orders/creation.journal"
    if not path.exists():
        return []
    raw = path.read_bytes()
    assert 80 <= len(raw) <= 64 * 1024 * 1024
    assert raw[:8] == b"DFMWOJ10"
    assert raw[48:80] == hashlib.sha256(raw[:48]).digest()
    identity, head, at, states = raw[8:40], raw[48:80], 80, []
    while at < len(raw):
        prefix = raw[at:at + 52]
        assert len(prefix) == 52 and prefix[:8] == b"DFMWOR10"
        length, number = struct.unpack_from(">IQ", prefix, 8)
        assert 0 < length <= 19 * 1024 and number == len(states) + 1
        assert prefix[20:52] == head
        body = raw[at + 52:at + 52 + length]
        trailer = raw[at + 52 + length:at + 92 + length]
        assert len(body) == length and len(trailer) == 40
        head = hashlib.sha256(
            b"dfmcp-work-order-journal-frame/1\0" + identity + prefix + body
        ).digest()
        assert trailer == head + b"DFMWEND0"
        assert body[0] in range(1, 7)
        states.append(body[0])
        at += 92 + length
    assert at == len(raw)
    return states


def seed_workshop(executable, combined, directory):
    """Reuse the existing actual Workshop test's native seed phase."""
    helper = load("stock_target_workshop_runtime_helper", "combined_workshop_runtime_peer.py")
    workshop = load("stock_target_workshop_native_fixture", "workshop.py")
    workshop.VECTORS = {
        key: bytes.fromhex(value)
        for key, value in json.loads((ROOT / "tests/native/workshop/vectors.json").read_text()).items()
    }
    capture = workshop.VECTORS["capture"]
    tick = int.from_bytes(workshop.VECTORS["progress_complete"][16:24], "big")
    combined.SITE = int.from_bytes(capture[32:36], "big")
    combined.DF, combined.DFH, combined.TICK = b"test-df", b"test-dfhack", tick
    queue_encoder = combined.order_queue

    def joined_queue(created=False, *, tick=tick, sequence=None, retired=False):
        return queue_encoder(created, tick=tick, sequence=sequence, retired=retired)

    combined.order_queue = joined_queue
    donor = directory / "donor"
    donor.mkdir(mode=0o700)
    custody = donor / "custody"
    custody.mkdir(mode=0o700)
    peer = workshop.Peer(donor)
    try:
        source, output = directory / "workshop-seed.json", directory / "workshop-original.json"
        source.write_text(json.dumps({
            "directory": str(custody), "endpoint": peer.endpoint,
            "world_folder": "region1", "site_id": combined.SITE,
            "df_version": "test-df", "dfhack_version": "test-dfhack",
            "generation": int.from_bytes(capture[8:16], "big"),
            "map_dimensions": [int.from_bytes(capture[i:i + 4], "big") for i in (36, 40, 44)],
            "capture_tick": int.from_bytes(capture[24:32], "big"),
        }))
        clean = {key: value for key, value in os.environ.items()
                 if not key.startswith(("DFMCP_", "COMBINED_STOCK_TARGET_RUNTIME_"))}
        helper.run_child(
            executable,
            "live_production_runtime::workshop_prerequisite_tests::"
            "native_workshop_prerequisite_qualifies_and_rechecks_each_order_boundary",
            "seed", "stock-target", clean, source, output,
        )
        before = helper.snapshot(custody)
        assert set(before) == {"workshop.journal", "workshop.ack"}
        assert peer.writer_count == 1
        return peer, custody, before, len(peer.calls), json.loads(output.read_text()), helper
    except BaseException:
        peer.close()
        raise


def run_case(executable, test_name, requested_case):
    assignment = requested_case.startswith("assignment_")
    case = requested_case.removeprefix("assignment_")
    combined = load("stock_target_production_native_fixture", "combined_production.py")
    inspection = load("stock_target_operations_native_fixture", "combined_production_inspection.py")
    with tempfile.TemporaryDirectory(prefix="dfmcp-stock-target-runtime-") as temporary:
        directory = Path(temporary).resolve()
        production = directory / "production"
        production.mkdir(mode=0o700)
        stage = directory / "stage.txt"
        donor = seed_workshop(executable, combined, directory) if case == "workshop" else None

        class StockPeer(combined.Peer):
            def __init__(self):
                super().__init__(production)
                self.existing_labor_ready = not assignment
                self.last_stage = None
                self.injected_after_dispatch = False
                self.inventory_stack = {"already": 6, "already_denied": 6, "above": 9}.get(case, 2)
                if case == "fresh":
                    self.after_operations_read = lambda: setattr(self, "inventory_stack", 4)
                if case == "generation":
                    self.operations_generation += 1
                if case == "matching_job":
                    self.operations_capture = inspection.operations_capture
                elif case in ("unpaused", "incomplete"):
                    encoder = self.operations_capture

                    def changed_capture(tick, stack):
                        raw = bytearray(encoder(tick, stack))
                        if case == "unpaused":
                            raw[28] = 0
                        else:
                            raw.pop()
                        return bytes(raw)

                    self.operations_capture = changed_capture

                def enabled_labor(capture):
                    capture["automatic"] = False
                    capture["details"] = []
                    capture["labor_keys"] = ["MINE", "CARPENTRY"]
                    for unit in capture["units"]:
                        unit["labors"] = [1, 0]

                # Assignment cases use the original disabled MINE capture and
                # its real manual detail; the existing-worker path has no setter.
                if not assignment:
                    self.workforce_capture_hook = enabled_labor

            def external_queue(self):
                self.order_created = True
                self.order_sequence += 1

            def apply_stage(self):
                value = stage.read_text() if stage.exists() else ""
                if value == self.last_stage:
                    return
                self.last_stage = value
                if value == "before_prepare":
                    if case == "before_stock":
                        self.inventory_stack += 1
                    elif case == "before_queue":
                        self.external_queue()
                    elif case == "before_job":
                        self.operations_capture = inspection.operations_capture
                elif value in ("goal_first", "goal_second"):
                    self.inventory_stack = 6
                    self.goal_tick(combined.TICK + (5 if value == "goal_first" else 10))

            def reply(self, plugin, operation, request):
                self.apply_stage()
                if case in ("after_stock", "after_queue") and not self.injected_after_dispatch:
                    if 2 in creation_states(production):
                        self.injected_after_dispatch = True
                        if case == "after_stock":
                            self.inventory_stack += 1
                        else:
                            self.external_queue()
                response = super().reply(plugin, operation, request)
                if case == "lost" and operation == "CommitOrder":
                    raise ConnectionResetError("lost reply after original stock-target insertion")
                return response

        peer = None
        try:
            peer = StockPeer()
            environment = combined.environment(production, peer.endpoint)
            environment[STAGE] = str(stage)
            if donor is not None:
                parent = json.loads(environment["DFMCP_COMBINED_PRODUCTION_CONFIG"])
                parent["workshop_prerequisite"] = donor[4]
                environment["DFMCP_COMBINED_PRODUCTION_CONFIG"] = json.dumps(parent)
                environment["DFMCP_COMBINED_PRODUCTION_WORKSHOP_TOKEN"] = "t" * 32
            retained_path = directory / "retained.json"
            run_child(executable, test_name, "control", requested_case, environment,
                      directory / "unused.json", retained_path)
            calls = [operation for _, operation in peer.calls]
            no_assignment = not assignment or case in (
                "already", "already_denied", "above", "denied", "budget")
            if no_assignment:
                assert not peer.workforce_applied and peer.workforce_plan is None
                assert not any(operation in (
                    "PrepareAssignment", "CommitAssignment", "QueryAssignment")
                    for operation in calls)
            else:
                assert peer.workforce_applied and peer.workforce_plan["assigned"]
                assert calls.count("PrepareAssignment") == calls.count("CommitAssignment") == 1
                if "PrepareOrder" in calls:
                    assert calls.index("CommitAssignment") < calls.index("PrepareOrder")
                retained = json.loads(retained_path.read_text())
                assert peer.workforce_plan["key"] == retained["original_plan"]["steps"][0]["key"]
            assert not peer.clock_commits and not peer.inventory_clock_commits
            if case in ("fresh", "workshop", "lost") or (assignment and case == "positive"):
                assert calls.count("PrepareOrder") == calls.count("CommitOrder") == 1
                assert peer.order_plan["amount"] == (2 if case == "fresh" else 4)
                assert peer.order_created
            elif case in ("after_stock", "after_queue"):
                assert peer.injected_after_dispatch, "late test never reached durable dispatch"
                assert 2 in creation_states(production)
                assert calls.count("PrepareOrder") == 1 and "CommitOrder" not in calls
            else:
                assert not any(operation in ("PrepareOrder", "CommitOrder", "CancelOrder")
                               for operation in calls), calls
                if no_assignment:
                    assert not any(operation.startswith(("Prepare", "Commit", "Cancel"))
                                   for operation in calls), calls
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
                # Changed live stock must never rewrite the original deficit.
                peer.inventory_stack = 9
                before_calls, before_files = list(peer.calls), combined.custody_files(production)
                run_child(executable, test_name, "offline", requested_case, offline, retained_path,
                          directory / "offline.json")
                assert before_calls == peer.calls, "Offline contacted a native source"
                assert before_files == combined.custody_files(production), "Offline changed custody"
                run_child(executable, test_name, "reconcile", requested_case, reopened, retained_path,
                          directory / "reconciled.json")
                after = [operation for _, operation in peer.calls]
                assert after.count("PrepareOrder") == after.count("CommitOrder") == 1
                assert "QueryOrder" in after[len(before_calls):]
                order_index = int(assignment)
                assert peer.order_plan["key"].decode() == retained["original_plan"]["steps"][order_index]["key"]
                if assignment:
                    assert after.count("PrepareAssignment") == after.count("CommitAssignment") == 1
                    assert peer.workforce_plan["key"] == retained["original_plan"]["steps"][0]["key"]
            if donor is not None:
                workshop_peer, custody, before, previous_calls, _, helper = donor
                assert helper.snapshot(custody) == before
                assert workshop_peer.writer_count == 1
                assert all(operation not in (2, 3, 5)
                           for operation in workshop_peer.calls[previous_calls:])
        finally:
            try:
                if peer is not None:
                    peer.close()
            finally:
                if donor is not None:
                    donor[0].close()


def main():
    executable, test_name, campaign = sys.argv[1:]
    cases = {
        "planning": ("fresh", "already", "above", "cap", "generation", "unpaused",
                     "incomplete", "matching_job", "workshop"),
        "recovery": ("before_stock", "before_queue", "before_job",
                     "after_stock", "after_queue", "lost"),
        "assignment": ("assignment_positive", "assignment_already", "assignment_above",
                       "assignment_denied", "assignment_already_denied", "assignment_budget"),
        "assignment_recovery": ("assignment_before_stock", "assignment_before_queue",
                                "assignment_after_stock", "assignment_after_queue",
                                "assignment_lost"),
    }[campaign]
    for case in cases:
        run_case(executable, test_name, case)
    print(f"combined-stock-target-campaign:{campaign}:checked")


if __name__ == "__main__":
    main()
