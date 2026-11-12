#!/usr/bin/env python3
"""Actual Rust production Runtime with Manager 1.23 and production TCP doubles.

A Rust child creates the original appointment through the real native client and
private ACK/journal. Manager vectors are explicitly rebased to the workshop
fixture's source; plan, token and receipt commitments are recomputed. These are
protocol doubles, not native bridge, MCP transport or live-game evidence.
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
PHASE = "COMBINED_MANAGER_RUNTIME_PHASE"
CASE = "COMBINED_MANAGER_RUNTIME_CASE"
INPUT = "COMBINED_MANAGER_RUNTIME_INPUT"
OUTPUT = "COMBINED_MANAGER_RUNTIME_OUTPUT"


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def snapshot(directory):
    return {path.name: (path.stat().st_mode & 0o7777,
                        hashlib.sha256(path.read_bytes()).hexdigest())
            for path in sorted(directory.iterdir())}


def run_child(executable, test_name, phase, case, environment, source, destination):
    environment = environment.copy()
    environment.update({PHASE: phase, CASE: case, INPUT: str(source), OUTPUT: str(destination),
                        "PYTHONDONTWRITEBYTECODE": "1"})
    result = subprocess.run([executable, "--exact", test_name, "--nocapture"],
                            env=environment, timeout=180, capture_output=True, text=True,
                            check=False)
    assert result.returncode == 0, f"{phase}/{case} failed:\n{result.stdout}\n{result.stderr}"
    assert f"combined-manager-runtime:{phase}:{case}:checked" in result.stdout


def record_captures(record):
    key_length = int.from_bytes(record[8:10], "big")
    pos = 10 + key_length + 48
    size = int.from_bytes(record[pos:pos + 4], "big")
    before = (pos + 4, size)
    pos += 4 + size
    present = record[pos + 2]
    pos += 3
    after = None
    if present:
        size = int.from_bytes(record[pos:pos + 4], "big")
        after = (pos + 4, size)
    return before, after


def rebase_manager(vectors, site, dimensions):
    """Keep assignment/HF/history/cache facts, rebind the double's source only."""
    def capture(raw):
        data = bytearray(raw)
        assert data[:8] == b"DFMMC023"
        data[32:36] = site.to_bytes(4, "big")
        data[44:56] = b"".join(n.to_bytes(4, "big") for n in dimensions)
        return bytes(data)

    result = {**vectors, "capture": capture(vectors["capture"])}
    plan = hashlib.sha256(b"dfmcp-manager-plan/1\0" + (256).to_bytes(4, "big")
                          + (361).to_bytes(4, "big")
                          + hashlib.sha256(result["capture"]).digest()).digest()
    result["plan"] = plan
    for name in ("prepared", "assigned"):
        data = bytearray(vectors[name])
        key_length = int.from_bytes(data[8:10], "big")
        key = bytes(data[10:10 + key_length])
        pos = 10 + key_length
        token = hashlib.sha256(b"dfmcp-manager-token/1\0" + len(key).to_bytes(2, "big")
                               + key + plan).digest()[:16]
        data[pos:pos + 48] = plan + token
        for span in record_captures(data):
            if span is not None:
                pos, size = span
                data[pos:pos + size] = capture(data[pos:pos + size])
        data[-32:] = hashlib.sha256(b"dfmcp-manager-receipt/1\0" + data[:-32]).digest()
        result[name] = bytes(data)
        result["token"] = token
    return result


def dispatch_markers(directory):
    """Independently inspect checksum-valid parent and native durable markers."""
    raw = (directory / "parent/execution.journal").read_bytes()
    assert raw[:8] == b"DFMLDJ01"
    archive_size = int.from_bytes(raw[104:108], "big")
    pos = 108 + archive_size
    head = hashlib.sha256(b"dfmcp-live-dispatch-journal/1\0" + raw[:pos]).digest()
    assert raw[pos:pos + 32] == head
    pos += 32
    phases, sequence = {}, 0
    while pos < len(raw):
        assert raw[pos:pos + 32] == head
        assert int.from_bytes(raw[pos + 32:pos + 36], "big") == sequence
        kind = raw[pos + 36]
        step = int.from_bytes(raw[pos + 37:pos + 41], "big")
        size = int.from_bytes(raw[pos + 49:pos + 51], "big")
        end = pos + 51 + size
        head = hashlib.sha256(b"dfmcp-live-dispatch-frame/1\0" + raw[pos:end]).digest()
        assert raw[end:end + 32] == head
        if kind != 11:
            phases[step] = kind
        sequence += 1
        pos = end + 32
    assert pos == len(raw)
    native = (directory / "orders/creation.journal").read_bytes()
    assert native[:8] == b"DFMWOJ10"
    head = hashlib.sha256(native[:48]).digest()
    assert native[48:80] == head
    identity = native[8:40]
    pos, sequence, phase = 80, 1, None
    while pos < len(native):
        prefix = native[pos:pos + 52]
        assert prefix[:8] == b"DFMWOR10"
        size = int.from_bytes(prefix[8:12], "big")
        assert int.from_bytes(prefix[12:20], "big") == sequence
        assert prefix[20:52] == head
        body = native[pos + 52:pos + 52 + size]
        head = hashlib.sha256(b"dfmcp-work-order-journal-frame/1\0" + identity + prefix + body).digest()
        end = pos + 52 + size
        assert native[end:end + 32] == head and native[end + 32:end + 40] == b"DFMWEND0"
        phase = body[0]
        pos, sequence = end + 40, sequence + 1
    assert pos == len(native)
    return phases.get(1) == 3, phase == 2


def run_case(executable, test_name, case):
    manager = load("joined_manager_native", "manager_appointment.py")
    workshop = load("joined_manager_workshop", "workshop.py")
    workshop_driver = load("joined_manager_workshop_driver", "combined_workshop_runtime_peer.py")
    combined = load("joined_manager_production", "combined_production.py")
    manager.MINOR, manager.PROFILE = 23, "1.23"
    workshop.VECTORS = {key: bytes.fromhex(value) for key, value in
                        json.loads((ROOT / "tests/native/workshop/vectors.json").read_text()).items()}
    workshop_capture = workshop.VECTORS["capture"]
    site = int.from_bytes(workshop_capture[32:36], "big")
    dimensions = [int.from_bytes(workshop_capture[p:p + 4], "big") for p in (36, 40, 44)]
    original_manager = {key: bytes.fromhex(value) for key, value in
                        json.loads((ROOT / "tests/fixtures/manager_v1_23_component.json").read_text()).items()}
    vectors = rebase_manager(original_manager, site, dimensions)
    _, (after_at, after_size) = record_captures(vectors["assigned"])
    ready = vectors["assigned"][after_at:after_at + after_size]
    tick = int.from_bytes(ready[24:32], "big")
    combined.SITE = site
    combined.DF, combined.DFH, combined.TICK = b"test-df", b"test-dfhack", tick
    queue_encoder = combined.order_queue

    def joined_order_queue(created=False, *, tick=tick, sequence=None, retired=False):
        return queue_encoder(created, tick=tick, sequence=sequence, retired=retired)

    combined.order_queue = joined_order_queue
    citizen_encoder = combined.citizen_reply

    def joined_citizen_reply(operation, request, citizen_ids=(42,)):
        raw = citizen_encoder(operation, request, citizen_ids)
        if operation == "ReadObservation":
            # The source fixture has exactly one citizen, hence no repeated tag.
            values = manager.fields(raw)
            values[11], values[12] = divmod(tick, 403200)
            raw = b"".join(manager.number(tag, value) if isinstance(value, int)
                           else manager.blob(tag, value) for tag, value in values.items())
        return raw

    combined.citizen_reply = joined_citizen_reply

    def change_workshop(vector):
        data = bytearray(workshop.VECTORS[vector])
        data[16:24] = tick.to_bytes(8, "big")
        workshop.VECTORS["progress_complete"] = bytes(data)

    change_workshop("progress_complete")

    class ManagerPeer(manager.Peer):
        readiness = "ready"
        changed_after_markers = False

        def reply(self, stream, raw):
            values = manager.fields(raw)
            if self.writer_count and 9 in values:
                if case == "post_marker" and production_peer.order_plan is not None:
                    parent_started, native_started = dispatch_markers(production)
                    if parent_started and native_started:
                        # The first rejected fact is delivered only after both
                        # original durable dispatch markers are on disk.
                        self.readiness = "identity"
                        self.changed_after_markers = True
                capture = bytearray(ready)
                if self.readiness == "cache":
                    at = manager.cache_bucket_offset(capture, 0)
                    assert capture[at:at + 6] == b"\0\1" + (600).to_bytes(4, "big")
                    capture[at:at + 6] = b"\0\0"
                elif self.readiness == "identity":
                    capture[-6] = 0
                elif self.readiness == "generation":
                    values[6] += 1
                    capture[8:16] = values[6].to_bytes(8, "big")
                elif self.readiness in ("future", "past"):
                    current = tick + (1 if self.readiness == "future" else -1)
                    capture[24:32] = current.to_bytes(8, "big")
                    year, annual = divmod(current, 403200)
                    capture[36:44] = year.to_bytes(4, "big") + annual.to_bytes(4, "big")
                values[9] = bytes(capture)
                raw = b"".join(manager.number(tag, value) if isinstance(value, int)
                               else manager.blob(tag, value) for tag, value in values.items())
            super().reply(stream, raw)

    class ProductionPeer(combined.Peer):
        def reply(self, plugin, operation, request):
            raw = super().reply(plugin, operation, request)
            if operation == "CommitAssignment":
                if case == "before_prepare":
                    manager_peer.readiness = "cache"
                elif case == "dual_late":
                    change_workshop("progress_absent")
                elif case == "custody_late":
                    ack = custody / "manager-appointment-v1-23.ack"
                    data = bytearray(ack.read_bytes())
                    data[-1] ^= 1
                    ack.write_bytes(data)
            if case == "after_prepare" and operation == "PrepareOrder":
                manager_peer.readiness = "identity"
            if case.endswith("lost") and operation == "CommitOrder":
                raise ConnectionResetError("lost original production reply after insertion")
            return raw

    with tempfile.TemporaryDirectory(prefix="dfmcp-manager-production-") as temporary:
        directory = Path(temporary).resolve()
        donor, production = directory / "manager", directory / "production"
        donor.mkdir(mode=0o700)
        production.mkdir(mode=0o700)
        custody = donor / "custody"
        custody.mkdir(mode=0o700)
        manager_peer, production_peer = ManagerPeer(donor, vectors), ProductionPeer(production)
        workshop_peer = None
        try:
            seed = {"directory": str(custody), "endpoint": manager_peer.endpoint,
                    "world_folder": "region1", "site_id": site,
                    "df_version": "test-df", "dfhack_version": "test-dfhack",
                    "generation": int.from_bytes(vectors["capture"][8:16], "big"),
                    "map_dimensions": dimensions, "unit_id": 256, "histfig_id": 361,
                    "capture_tick": tick}
            source, prerequisite = directory / "seed.json", directory / "manager.json"
            source.write_text(json.dumps(seed))
            clean_environment = {key: value for key, value in os.environ.items()
                                 if not key.startswith("DFMCP_")}
            run_child(executable, test_name, "seed", case, clean_environment, source, prerequisite)
            donor_before = snapshot(custody)
            assert set(donor_before) == {"manager-appointment-v1-23.journal", "manager-appointment-v1-23.ack"}
            assert all(mode == 0o600 for mode, _ in donor_before.values())
            assert manager_peer.writer_count == 1
            donor_calls = len(manager_peer.calls)
            environment = combined.environment(production, production_peer.endpoint)
            parent = json.loads(environment["DFMCP_COMBINED_PRODUCTION_CONFIG"])
            parent["manager_prerequisite"] = json.loads(prerequisite.read_text())
            environment["DFMCP_COMBINED_PRODUCTION_MANAGER_TOKEN"] = "t" * 32
            if case == "all_met":
                # Plan remains independently granted through this clock owner;
                # Rust revokes both effect owners with zero available game ticks.
                parent["inventory_clock"] = {"generation": combined.INVENTORY_CLOCK_GENERATION}
                order_config = json.loads(environment["DFMCP_SEMANTIC_WORK_ORDER_CONFIG"])
                order_config["inventory_clock"] = parent["inventory_clock"]
                environment["DFMCP_SEMANTIC_WORK_ORDER_CONFIG"] = json.dumps(order_config)
                environment["DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK"] = "1"
                environment["DFMCP_SEMANTIC_WORK_ORDER_INVENTORY_CLOCK_TOKEN"] = combined.TOKEN.decode()
            workshop_before = None
            workshop_calls = 0
            if case.startswith("dual"):
                workshop_directory = directory / "workshop"
                workshop_directory.mkdir(mode=0o700)
                workshop_custody = workshop_directory / "custody"
                workshop_custody.mkdir(mode=0o700)
                workshop_peer = workshop.Peer(workshop_directory)
                workshop_seed = {"directory": str(workshop_custody), "endpoint": workshop_peer.endpoint,
                                 "world_folder": "region1", "site_id": site,
                                 "df_version": "test-df", "dfhack_version": "test-dfhack",
                                 "generation": int.from_bytes(workshop_capture[8:16], "big"),
                                 "map_dimensions": dimensions,
                                 "capture_tick": int.from_bytes(workshop_capture[24:32], "big")}
                workshop_source, workshop_output = directory / "workshop-seed.json", directory / "workshop.json"
                workshop_source.write_text(json.dumps(workshop_seed))
                workshop_test = test_name.rsplit("::", 2)[0] + "::workshop_prerequisite_tests::native_workshop_prerequisite_qualifies_and_rechecks_each_order_boundary"
                workshop_driver.run_child(executable, workshop_test, "seed", case, clean_environment,
                                          workshop_source, workshop_output)
                parent["workshop_prerequisite"] = json.loads(workshop_output.read_text())
                environment["DFMCP_COMBINED_PRODUCTION_WORKSHOP_TOKEN"] = "t" * 32
                workshop_before, workshop_calls = snapshot(workshop_custody), len(workshop_peer.calls)
            environment["DFMCP_COMBINED_PRODUCTION_CONFIG"] = json.dumps(parent)
            if case in ("cache", "identity", "generation", "future", "past"):
                manager_peer.readiness = case
            elif case == "dual_manager":
                manager_peer.readiness = "cache"
            elif case == "dual_workshop":
                change_workshop("progress_absent")
            elif case == "all_met":
                # An unusable effect prerequisite must not affect an all-met census.
                manager_peer.readiness = "cache"
                environment.pop("DFMCP_COMBINED_PRODUCTION_MANAGER_TOKEN")
            retained_path = directory / "retained.json"
            ack_saved = (custody / "manager-appointment-v1-23.ack").read_bytes()
            try:
                run_child(executable, test_name, "control", case, environment, prerequisite, retained_path)
            finally:
                if case == "custody_late":
                    (custody / "manager-appointment-v1-23.ack").write_bytes(ack_saved)
            calls = [operation for _, operation in production_peer.calls]
            initial_refusals = ("cache", "identity", "generation", "future", "past", "dual_manager", "dual_workshop")
            if case in initial_refusals or case == "all_met":
                assert not any(operation.startswith(("Prepare", "Commit")) for operation in calls)
                assert not production_peer.order_created and not production_peer.workforce_applied
                if case == "all_met":
                    assert manager_peer.calls[donor_calls:] == []
            else:
                assert calls.count("PrepareAssignment") == calls.count("CommitAssignment") == 1
                assert production_peer.workforce_applied
                if case in ("before_prepare", "custody_late", "dual_late"):
                    assert "PrepareOrder" not in calls and "CommitOrder" not in calls
                elif case in ("after_prepare", "post_marker"):
                    assert calls.count("PrepareOrder") == 1 and "CommitOrder" not in calls
                    if case == "post_marker":
                        assert manager_peer.changed_after_markers
                        assert dispatch_markers(production) == (True, True)
                else:
                    assert calls.count("PrepareOrder") == calls.count("CommitOrder") == 1
                    assert production_peer.order_created
                    assert manager_peer.calls[donor_calls:].count(1) >= 3
            if case == "generation":
                checkpoint_reopen(executable, test_name, case, directory, environment,
                                  retained_path, manager_peer, production_peer, combined)
            if case.endswith("lost"):
                recover(executable, test_name, directory, environment, retained_path, custody,
                        manager_peer, production_peer, combined, workshop_peer)
            assert donor_before == snapshot(custody), "production changed donor custody"
            assert manager_peer.writer_count == 1
            assert all(operation not in (2, 3, 5) for operation in manager_peer.calls[donor_calls:])
            if workshop_peer is not None:
                assert workshop_before == snapshot(workshop_custody)
                assert workshop_peer.writer_count == 1
                assert all(operation not in (2, 3, 5) for operation in workshop_peer.calls[workshop_calls:])
            assert not production_peer.clock_commits and not production_peer.inventory_clock_commits
        finally:
            try:
                production_peer.close()
            finally:
                try:
                    manager_peer.close()
                finally:
                    if workshop_peer is not None:
                        workshop_peer.close()


def checkpoint_reopen(executable, test_name, case, directory, environment, retained_path,
                      manager_peer, production_peer, combined):
    retained = json.loads(retained_path.read_text())
    checkpoint = retained["manager_prerequisite"]["checkpoint"]
    assert checkpoint["fenced"] is True
    reopened = combined.reopen_environment(environment, None, retained["native_custody"]["workforce_journal_id"])
    parent = json.loads(reopened["DFMCP_COMBINED_PRODUCTION_CONFIG"])
    parent["manager_prerequisite_checkpoint"] = checkpoint
    reopened["DFMCP_COMBINED_PRODUCTION_CONFIG"] = json.dumps(parent)
    reopened.pop("DFMCP_COMBINED_PRODUCTION_MANAGER_TOKEN")
    manager_peer.readiness = "ready"
    calls = (list(manager_peer.calls), list(production_peer.calls))
    run_child(executable, test_name, "checkpoint", case, reopened, retained_path, directory / "checkpoint.json")
    assert calls[0] == manager_peer.calls
    assert not any(operation.startswith(("Prepare", "Commit"))
                   for _, operation in production_peer.calls)
    parent["manager_prerequisite_checkpoint"] = {
        **checkpoint, "origin_witness": hashlib.sha256(b"a different original manager").hexdigest()}
    reopened["DFMCP_COMBINED_PRODUCTION_CONFIG"] = json.dumps(parent)
    calls = (list(manager_peer.calls), list(production_peer.calls))
    run_child(executable, test_name, "reject", "checkpoint-donor", reopened,
              retained_path, directory / "wrong-checkpoint.json")
    assert calls == (manager_peer.calls, production_peer.calls)


def recover(executable, test_name, directory, environment, retained_path, custody,
            manager_peer, production_peer, combined, workshop_peer=None):
    retained = json.loads(retained_path.read_text())
    reopened = combined.reopen_environment(environment, retained["recovery"],
                                           retained["native_custody"]["workforce_journal_id"])
    offline = {key: value for key, value in reopened.items()
               if not (key.startswith("DFMCP_") and (key.endswith("_TOKEN")
                       or key.endswith("_ALLOW_LABOR") or key.endswith("_ALLOW_PRODUCTION")))}
    def native_calls():
        return (list(production_peer.calls), list(manager_peer.calls),
                list(workshop_peer.calls) if workshop_peer is not None else [])
    calls = native_calls()
    files = combined.custody_files(production_peer.directory)
    run_child(executable, test_name, "offline", "lost", offline, retained_path, directory / "offline.json")
    assert calls == native_calls()
    assert files == combined.custody_files(production_peer.directory)
    for alteration in ("drop", "swap", "drop-all", "floor"):
        invalid = offline.copy()
        parent = json.loads(invalid["DFMCP_COMBINED_PRODUCTION_CONFIG"])
        if alteration in ("drop", "drop-all"):
            parent.pop("manager_prerequisite")
        elif alteration == "swap":
            parent["manager_prerequisite"]["original_key"] = "substituted-original"
        else:
            parent["recovery"].pop("manager_prerequisite_floor")
        association = production_peer.directory / "parent/original.manager"
        hidden = directory / "saved-original-manager"
        if alteration == "drop-all":
            parent["recovery"].pop("manager_prerequisite_digest")
            parent["recovery"].pop("manager_prerequisite_floor")
            association.rename(hidden)
        invalid["DFMCP_COMBINED_PRODUCTION_CONFIG"] = json.dumps(parent)
        try:
            run_child(executable, test_name, "reject", alteration, invalid, retained_path,
                      directory / f"rejected-{alteration}.json")
        finally:
            if alteration == "drop-all":
                hidden.rename(association)
    ack = custody / "manager-appointment-v1-23.ack"
    saved = ack.read_bytes()
    damaged = bytearray(saved)
    damaged[-1] ^= 1
    try:
        ack.write_bytes(damaged)
        run_child(executable, test_name, "reject", "custody", offline,
                  retained_path, directory / "rejected-custody.json")
    finally:
        ack.write_bytes(saved)
    assert calls == native_calls()
    assert files == combined.custody_files(production_peer.directory)
    run_child(executable, test_name, "reconcile", "lost", reopened, retained_path,
              directory / "reconciled.json")
    final_calls = [operation for _, operation in production_peer.calls]
    assert final_calls.count("PrepareOrder") == final_calls.count("CommitOrder") == 1
    assert final_calls.count("CommitAssignment") == 1
    assert "QueryOrder" in final_calls[len(calls[0]):]
    assert production_peer.order_plan["key"].decode() == retained["original_plan"]["steps"][1]["key"]


def main():
    executable, test_name, kind = sys.argv[1:]
    assert kind in ("gates", "recovery")
    cases = ("success", "cache", "identity", "generation", "future", "past", "before_prepare",
             "after_prepare", "post_marker", "custody_late", "all_met", "dual", "dual_manager", "dual_workshop",
             "dual_late") if kind == "gates" else ("lost", "dual_lost")
    for case in cases:
        run_case(executable, test_name, case)
        print(f"combined-manager-case:{case}:checked")
    print(f"combined-manager-campaign:{kind}:checked")


if __name__ == "__main__":
    main()
