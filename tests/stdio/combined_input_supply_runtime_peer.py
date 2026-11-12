#!/usr/bin/env python3
"""Real production Runtime and private custody over bounded native TCP peers.

The driver encodes raw operations inventories independently, counts native
setters, and checks durable markers before injecting late supply loss. It is
component evidence, not an MCP implementation, native plugin or live game.
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
HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
PHASE = "COMBINED_INPUT_SUPPLY_PHASE"
CASE = "COMBINED_INPUT_SUPPLY_CASE"
INPUT = "COMBINED_INPUT_SUPPLY_INPUT"
OUTPUT = "COMBINED_INPUT_SUPPLY_OUTPUT"
STAGE = "COMBINED_INPUT_SUPPLY_STAGE"


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, HERE / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run_child(executable, test_name, phase, case, environment, source, destination):
    environment = environment.copy()
    environment.update({PHASE: phase, CASE: case, INPUT: str(source),
                        OUTPUT: str(destination), "PYTHONDONTWRITEBYTECODE": "1"})
    result = subprocess.run([executable, "--exact", test_name, "--nocapture"],
                            env=environment, timeout=180, capture_output=True,
                            text=True, check=False)
    assert result.returncode == 0, f"{phase}/{case}:\n{result.stdout}\n{result.stderr}"
    assert f"combined-input-supply-runtime:{phase}:{case}:checked" in result.stdout


def reference(value):
    return b"\0" if value is None else b"\1" + struct.pack(">I", value)


def close_peers(peers):
    failures = []
    for peer in reversed(peers):
        try:
            peer.close()
        except BaseException as error:
            failures.append(error)
    if failures:
        raise BaseExceptionGroup("native peer shutdown failures", failures)


def inventory(combined, tick, wood, *, ancestor=False, attached=False,
              next_item=100, already_met=False):
    """Complete sorted native rows with one independently described WOOD stack."""
    year, annual = divmod(tick, 403200)
    jobs = (b"DFMJ1200" + struct.pack(">IIBiI", year, annual, 1, combined.SITE, 100)
            + combined.text(b"region1") + struct.pack(">I", int(attached)))
    if attached:
        jobs += (struct.pack(">Ii", 3, 69) + combined.text(b"ConstructBed")
                 + combined.text(b"") + b"\0\0" + struct.pack(">iii", 1, 1, 1)
                 + reference(None) + reference(None) + struct.pack(">iII", -1, 1, 1))
    raw = (b"DFMO1300" + struct.pack(">I", len(jobs)) + jobs
           + struct.pack(">III", 100, next_item, 0))
    rows = [(20, 0, b"WOOD", wood, 64, 21 if ancestor else None)] if wood else []
    if ancestor:
        rows.append((21, 6, b"BIN", 1, 65, None))
    if already_met:
        rows.extend(((2, 5, b"BED", 1, 64, None), (3, 6, b"DOOR", 1, 64, None)))
    raw += struct.pack(">I", len(rows))
    for native_id, kind, key, units, flags, container in sorted(rows):
        raw += (struct.pack(">Ii", native_id, kind) + combined.text(key)
                + struct.pack(">iiiIiiiI", -1, 420, 0, units, 1, 1, 1, flags)
                + reference(container) + reference(None))
    raw += struct.pack(">I", int(attached))
    if attached:
        raw += struct.pack(">IIii", 3, 20, 0, 0)
    return raw


def dual_donors(executable, test_name, combined, directory):
    """Seed acknowledged originals through their actual Rust clients/owners."""
    manager = load("input_supply_manager_native", "manager_appointment.py")
    joined = load("input_supply_manager_helpers", "combined_manager_prerequisite_runtime_peer.py")
    workshop = load("input_supply_workshop_native", "workshop.py")
    workshop_driver = load("input_supply_workshop_helpers", "combined_workshop_runtime_peer.py")
    manager.MINOR, manager.PROFILE = 23, "1.23"
    workshop.VECTORS = {key: bytes.fromhex(value) for key, value in
                        json.loads((ROOT / "tests/native/workshop/vectors.json").read_text()).items()}
    capture = workshop.VECTORS["capture"]
    site = int.from_bytes(capture[32:36], "big")
    dimensions = [int.from_bytes(capture[at:at + 4], "big") for at in (36, 40, 44)]
    vectors = {key: bytes.fromhex(value) for key, value in
               json.loads((ROOT / "tests/fixtures/manager_v1_23_component.json").read_text()).items()}
    vectors = joined.rebase_manager(vectors, site, dimensions)
    _, (at, size) = joined.record_captures(vectors["assigned"])
    ready = vectors["assigned"][at:at + size]
    tick = int.from_bytes(ready[24:32], "big")
    combined.SITE, combined.DF, combined.DFH, combined.TICK = site, b"test-df", b"test-dfhack", tick
    citizen_encoder = combined.citizen_reply

    def citizen_reply(operation, request, citizen_ids=(42,)):
        raw = citizen_encoder(operation, request, citizen_ids)
        if operation == "ReadObservation":
            fields = manager.fields(raw)
            fields[11], fields[12] = divmod(tick, 403200)
            raw = b"".join(manager.number(tag, value) if isinstance(value, int)
                           else manager.blob(tag, value) for tag, value in fields.items())
        return raw

    combined.citizen_reply = citizen_reply
    progress = bytearray(workshop.VECTORS["progress_complete"])
    progress[16:24] = tick.to_bytes(8, "big")
    workshop.VECTORS["progress_complete"] = bytes(progress)

    class ReadyManager(manager.Peer):
        def reply(self, stream, raw):
            fields = manager.fields(raw)
            if self.writer_count and 9 in fields:
                fields[9] = ready
                raw = b"".join(manager.number(tag, value) if isinstance(value, int)
                               else manager.blob(tag, value) for tag, value in fields.items())
            return super().reply(stream, raw)

    peers = []
    try:
        clean = {key: value for key, value in os.environ.items() if not key.startswith("DFMCP_")}
        prefix = test_name.rsplit("::", 2)[0]
        for family in ("manager", "workshop"):
            donor = directory / family
            donor.mkdir(mode=0o700)
            custody = donor / "custody"
            custody.mkdir(mode=0o700)
            peer = ReadyManager(donor, vectors) if family == "manager" else workshop.Peer(donor)
            peers.append(peer)
            source, destination = directory / f"{family}-seed.json", directory / f"{family}.json"
            seed = {"directory": str(custody), "endpoint": peer.endpoint,
                    "world_folder": "region1", "site_id": site, "df_version": "test-df",
                    "dfhack_version": "test-dfhack", "map_dimensions": dimensions}
            if family == "manager":
                seed.update(generation=int.from_bytes(vectors["capture"][8:16], "big"),
                            unit_id=256, histfig_id=361, capture_tick=tick)
                child = prefix + "::manager_prerequisite_tests::native_manager_prerequisite_qualifies_and_rechecks_each_order_boundary"
                driver = joined
            else:
                seed.update(generation=int.from_bytes(capture[8:16], "big"),
                            capture_tick=int.from_bytes(capture[24:32], "big"))
                child = prefix + "::workshop_prerequisite_tests::native_workshop_prerequisite_qualifies_and_rechecks_each_order_boundary"
                driver = workshop_driver
            source.write_text(json.dumps(seed))
            driver.run_child(executable, child, "seed", "input-supply", clean, source, destination)
            assert peer.writer_count == 1
        return peers, {family: json.loads((directory / f"{family}.json").read_text())
                       for family in ("manager", "workshop")}, joined
    except BaseException:
        close_peers(peers)
        raise


def run_case(executable, test_name, case):
    portfolio = load("input_supply_portfolio", "combined_portfolio.py")
    combined = portfolio.b
    markers = load("input_supply_dispatch_markers", "combined_manager_prerequisite_runtime_peer.py")
    assignment = case in ("legacy", "portfolio", "stock_targets", "post_marker", "dual", "lost", "dual_lost")
    single = case in ("legacy", "existing", "stock_target", "dual", "lost", "dual_lost")
    all_met = case in ("all_met", "all_met_fenced")
    with tempfile.TemporaryDirectory(prefix="dfmcp-input-supply-") as temporary:
        directory = Path(temporary).resolve()
        production = directory / "production"
        production.mkdir(mode=0o700)
        stage = directory / "stage"
        peers, donors, donor_helper = ([], {}, None)
        production_peer = None
        try:
            if all_met or case in ("dual", "dual_lost", "dual_four_targets"):
                peers, donors, donor_helper = dual_donors(executable, test_name, combined, directory)
            donor_snapshots = [donor_helper.snapshot(Path(donors[family]["directory"]))
                               for family in ("manager", "workshop") if donors]
            donor_calls = [len(peer.calls) for peer in peers]

            class InputSupplyPeer(portfolio.PortfolioPeer):
                def __init__(self):
                    super().__init__(production)
                    self.existing_labor_ready = not assignment
                    self.wood = 0 if all_met else {
                        "shortage": 1, "reserve": 2, "dual_four_targets": 5}.get(case, 3)
                    self.next_item = 100
                    self.stage = None
                    self.after_markers = False
                    if not assignment:
                        def enabled(capture):
                            capture["automatic"] = False
                            capture["details"] = []
                            capture["labor_keys"] = ["MINE", "CARPENTRY"]
                            for unit in capture["units"]:
                                unit["labors"] = [1, 0]
                        self.workforce_capture_hook = enabled
                    if case == "extra_queue":
                        self.orders[100] = {"recipe": 1, "amount": 1, "remaining": 1,
                                            "status": 3, "retired": False}

                def operations(self):
                    return inventory(combined, self.order_tick, self.wood,
                                     ancestor=case == "ancestor", attached=case == "attachment",
                                     next_item=self.next_item, already_met=all_met)

                def apply_stage(self):
                    value = stage.read_text() if stage.exists() else ""
                    if value == self.stage:
                        return
                    self.stage = value
                    if value == "before_prepare":
                        if case == "before_prepare":
                            self.wood = 2
                        elif case == "floor_regression":
                            self.next_item = 99
                    elif value == "before_second_prepare":
                        assert len(self.created_keys) == 1
                        if case == "after_first":
                            # Enough for the remaining uninserted order plus
                            # reserve, but not the complete original demand.
                            self.wood = 2
                        elif case == "extra_after_first":
                            self.orders[101] = {"recipe": 3, "amount": 1, "remaining": 1,
                                                "status": 3, "retired": False}
                            self.order_sequence += 1
                        elif case == "retired_prefix":
                            self.orders[100]["retired"] = True
                            self.order_sequence += 1

                def reply(self, plugin, operation, request):
                    self.apply_stage()
                    if (case == "post_marker" and not self.after_markers
                            and plugin == "dfmcp_operations_v1_3" and operation == "ReadObservation"
                            and self.plans):
                        if markers.dispatch_markers(production) == (True, True):
                            self.wood = 2
                            self.after_markers = True
                    response = super().reply(plugin, operation, request)
                    if case.endswith("lost") and operation == "CommitOrder":
                        raise ConnectionResetError("lost original reply after input-qualified insertion")
                    return response

            production_peer = InputSupplyPeer()
            environment = combined.environment(production, production_peer.endpoint)
            parent = json.loads(environment["DFMCP_COMBINED_PRODUCTION_CONFIG"])
            parent["input_supply"] = {"schema": "dfmcp.furniture-input-supply/1",
                                      "reserve_wood_stack_units": 0 if case == "shortage" else 1}
            for family, donor in donors.items():
                parent[f"{family}_prerequisite"] = donor
                environment[f"DFMCP_COMBINED_PRODUCTION_{family.upper()}_TOKEN"] = "t" * 32
            if all_met:
                parent["inventory_clock"] = {"generation": combined.INVENTORY_CLOCK_GENERATION}
                orders = json.loads(environment["DFMCP_SEMANTIC_WORK_ORDER_CONFIG"])
                orders["inventory_clock"] = parent["inventory_clock"]
                environment["DFMCP_SEMANTIC_WORK_ORDER_CONFIG"] = json.dumps(orders)
                environment["DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK"] = "1"
                environment["DFMCP_SEMANTIC_WORK_ORDER_INVENTORY_CLOCK_TOKEN"] = combined.TOKEN.decode()
                environment.pop("DFMCP_COMBINED_PRODUCTION_MANAGER_TOKEN")
                environment.pop("DFMCP_COMBINED_PRODUCTION_WORKSHOP_TOKEN")
            environment["DFMCP_COMBINED_PRODUCTION_CONFIG"] = json.dumps(parent)
            environment[STAGE] = str(stage)
            retained_path = directory / "retained.json"
            run_child(executable, test_name, "control", case, environment,
                      directory / "unused.json", retained_path)
            calls = [operation for _, operation in production_peer.calls]
            refused_plan = case in ("shortage", "reserve", "ancestor", "attachment", "extra_queue",
                                    "positive_fenced")
            if refused_plan or all_met:
                assert not any(operation.startswith(("Prepare", "Commit")) for operation in calls), calls
                assert not production_peer.created_keys and not production_peer.workforce_applied
                if all_met or case == "positive_fenced":
                    assert sum(plugin == "dfmcp_operations_v1_3" and operation == "ReadObservation"
                               for plugin, operation in production_peer.calls) == 1
                    assert "ReadOrders" not in calls
                    assert [len(peer.calls) for peer in peers] == donor_calls
            else:
                assert calls.count("CommitAssignment") == int(assignment)
                if case in ("before_prepare", "floor_regression"):
                    assert "PrepareOrder" not in calls and "CommitOrder" not in calls
                elif case == "post_marker":
                    assert calls.count("PrepareOrder") == 1 and "CommitOrder" not in calls
                    assert production_peer.after_markers
                    assert markers.dispatch_markers(production) == (True, True)
                else:
                    expected = (4 if case == "dual_four_targets" else 1
                                if single or case in ("after_first", "extra_after_first", "retired_prefix") else 2)
                    assert calls.count("PrepareOrder") == calls.count("CommitOrder") == expected
                    assert len(production_peer.created_keys) == expected
                    retained = json.loads(retained_path.read_text())
                    steps = retained["original_plan"]["steps"][int(assignment):]
                    assert [key.decode() for key in production_peer.created_keys] == [step["key"] for step in steps[:expected]]
                    assert sum(plan["amount"] for plan in production_peer.plans.values()) <= (4 if case == "dual_four_targets" else 2)
            if case.endswith("lost"):
                recover(executable, test_name, case, directory, environment, retained_path,
                        production_peer, peers, combined)
            assert not production_peer.clock_commits and not production_peer.inventory_clock_commits
            for index, family in enumerate(("manager", "workshop") if donors else ()):
                assert donor_snapshots[index] == donor_helper.snapshot(Path(donors[family]["directory"]))
                assert peers[index].writer_count == 1
                assert all(operation not in (2, 3, 5) for operation in peers[index].calls[donor_calls[index]:])
        finally:
            try:
                if production_peer is not None:
                    production_peer.close()
            finally:
                close_peers(peers)


def recover(executable, test_name, case, directory, environment, retained_path,
            peer, donors, combined):
    retained = json.loads(retained_path.read_text())
    reopened = combined.reopen_environment(environment, retained["recovery"],
                                           retained["native_custody"]["workforce_journal_id"])
    offline = {key: value for key, value in reopened.items()
               if not (key.startswith("DFMCP_") and (key.endswith("_TOKEN")
                       or key.endswith("_ALLOW_LABOR") or key.endswith("_ALLOW_PRODUCTION")))}

    def calls():
        return [list(peer.calls)] + [list(donor.calls) for donor in donors]

    before_calls, before_files = calls(), combined.custody_files(peer.directory)
    run_child(executable, test_name, "offline", case, offline, retained_path, directory / "offline.json")
    assert before_calls == calls()
    assert before_files == combined.custody_files(peer.directory)
    for change in ("strip", "substitute", "floor"):
        invalid = offline.copy()
        config = json.loads(invalid["DFMCP_COMBINED_PRODUCTION_CONFIG"])
        if change == "strip":
            config.pop("input_supply")
            config["recovery"].pop("input_supply_digest")
            config["recovery"].pop("input_supply_floor")
        elif change == "substitute":
            config["input_supply"]["reserve_wood_stack_units"] += 1
        else:
            config["recovery"]["input_supply_floor"]["next_item_id"] -= 1
        invalid["DFMCP_COMBINED_PRODUCTION_CONFIG"] = json.dumps(config)
        run_child(executable, test_name, "reject", change, invalid, retained_path,
                  directory / f"reject-{change}.json")
    assert before_calls == calls()
    assert before_files == combined.custody_files(peer.directory)
    run_child(executable, test_name, "reconcile", case, reopened, retained_path,
              directory / "reconciled.json")
    final = [operation for _, operation in peer.calls]
    assert final.count("CommitOrder") == final.count("PrepareOrder") == 1
    assert final.count("CommitAssignment") == 1
    assert "QueryOrder" in final[len(before_calls[0]):]


def main():
    executable, test_name, kind = sys.argv[1:]
    assert kind in ("gates", "recovery")
    cases = ("legacy", "existing", "portfolio", "existing_portfolio", "stock_target",
             "stock_targets", "shortage", "reserve", "ancestor", "attachment", "extra_queue",
             "before_prepare", "post_marker", "after_first", "extra_after_first", "retired_prefix",
             "floor_regression", "all_met", "all_met_fenced", "positive_fenced",
             "dual", "dual_four_targets") if kind == "gates" else ("lost", "dual_lost")
    for case in cases:
        run_case(executable, test_name, case)
        print(f"combined-input-supply-case:{case}:checked", flush=True)
    print(f"combined-input-supply-campaign:{kind}:checked")


if __name__ == "__main__":
    main()
