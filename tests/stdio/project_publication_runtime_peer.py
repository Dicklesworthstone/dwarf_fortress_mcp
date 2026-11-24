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
import struct
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


def portfolio_inventory(combined, tick, stocks):
    """The existing complete operations/1.3 fixture with three retained types."""
    year, annual = divmod(tick, 403200)
    jobs = (b"DFMJ1200" + struct.pack(">IIBiI", year, annual, 1, combined.SITE, 100)
            + combined.text(b"region1") + struct.pack(">I", 0))
    raw = (b"DFMO1300" + struct.pack(">I", len(jobs)) + jobs
           + struct.pack(">III", 100, 100, 0))
    rows = [(kind, stack) for kind, stack in sorted(stocks.items()) if stack > 0]
    raw += struct.pack(">I", len(rows))
    for ident, (kind, stack) in enumerate(rows, start=2):
        raw += (struct.pack(">Ii", ident, 5 + ident) + combined.text(kind.encode())
                + struct.pack(">iiiIiiiI", -1, 420, 0, stack, 1, 1, 1, 64)
                + b"\x00\x00")
    return raw + struct.pack(">I", 0)


def pending_environment(combined, environment, retained):
    """Pin first clock IDs while deliberately omitting the lost original selector."""
    reference = (retained["intent"]["recovery_reference"]
                 if retained["intent"] is not None else retained["actual"])
    first = {family + "_journal_id": reference[family]["journal_id"]
             for family in ("clock", "inventory_clock") if reference[family] is not None}
    reopened = combined.reopen_environment(
        environment, first, retained["workforce_journal_id"])
    parent = json.loads(reopened["DFMCP_COMBINED_PRODUCTION_CONFIG"])
    parent["recovery"] = None
    reopened["DFMCP_COMBINED_PRODUCTION_CONFIG"] = json.dumps(parent)
    return reopened


def run_case(executable, test, requested_case, clocks=(), one_order=False):
    portfolio = requested_case.startswith("portfolio_")
    case = requested_case.removeprefix("portfolio_")
    reopening = case in ("reopened", "reopened_refusal")
    case = "published" if case == "reopened" else case
    prefix = "portfolio_" if portfolio else ""
    combined = load("project_publication_production_peer", "combined_production.py")
    stock = load("project_publication_workshop_seed", "combined_stock_target_runtime_peer.py")
    with tempfile.TemporaryDirectory(prefix="dfmcp-project-publication-") as temporary:
        directory = Path(temporary).resolve()
        donor = stock.seed_workshop(executable, combined, directory)
        workshop, custody, donor_before, donor_calls, prerequisite, helper = donor
        production = directory / "production"
        production.mkdir(mode=0o700)
        stage = directory / "stage.txt"
        original_capture = combined.workforce_capture

        def carpenter_capture(ids=(42,), option_count=0):
            # Observe and Prepare must use identical native bytes. The generic
            # peer's observation-only hook would leave Prepare pinned to MINE.
            capture = combined.wf.capture(original_capture(ids, option_count))
            capture["labor_keys"] = ["CARPENTER", "MINE"]
            return combined.wf.encode_capture(capture)

        combined.workforce_capture = carpenter_capture
        class ProjectPeer(combined.Peer):
            def __init__(self, directory):
                super().__init__(directory)
                self.chair_stack = 1
                if portfolio:
                    self.operations_capture = lambda tick, bed_stack: portfolio_inventory(
                        combined, tick, {"BED": bed_stack, "CHAIR": self.chair_stack, "DOOR": 1})

            def reply(self, plugin, operation, request):
                phase = stage.read_text() if stage.exists() else ""
                offset = {"first_sample": 5, "second_sample": 10, "at_parent_floor": 20}.get(phase)
                if offset is not None:
                    assert not self.order_created and not self.workforce_applied
                    self.order_tick = self.workforce_tick = combined.TICK + offset
                if plugin == "dfmcp_bridge":
                    return combined.citizen_reply(
                        operation, request, self.citizen_ids, tick=self.order_tick)
                return super().reply(plugin, operation, request)

        peer = ProjectPeer(production)
        try:
            met = case in ("no_work", "no_work_floor") or reopening
            peer.inventory_stack = 6 if met else 2
            peer.chair_stack = 2 if met or one_order else 1
            environment = combined.environment(production, peer.endpoint)
            environment["PROJECT_PUBLICATION_RUNTIME_STAGE"] = str(stage)
            if one_order:
                environment["PROJECT_PUBLICATION_RUNTIME_ONE_ORDER"] = "1"
            if clocks:
                environment["PROJECT_PUBLICATION_RUNTIME_CLOCKS"] = ",".join(clocks)
                for family, generation, flag in (
                    ("clock", combined.CLOCK_GENERATION, "CLOCK"),
                    ("inventory_clock", combined.INVENTORY_CLOCK_GENERATION, "INVENTORY_CLOCK"),
                ):
                    if family not in clocks:
                        continue
                    for name in ("DFMCP_COMBINED_PRODUCTION_CONFIG", "DFMCP_SEMANTIC_WORK_ORDER_CONFIG"):
                        configured = json.loads(environment[name])
                        configured[family] = {"generation": generation}
                        environment[name] = json.dumps(configured)
                    environment[f"DFMCP_SEMANTIC_WORK_ORDER_ALLOW_{flag}"] = "1"
                    environment[f"DFMCP_SEMANTIC_WORK_ORDER_{flag}_TOKEN"] = combined.TOKEN.decode()
            parent = json.loads(environment["DFMCP_COMBINED_PRODUCTION_CONFIG"])
            parent["workshop_prerequisite"] = prerequisite
            environment["DFMCP_COMBINED_PRODUCTION_CONFIG"] = json.dumps(parent)
            environment["DFMCP_COMBINED_PRODUCTION_WORKSHOP_TOKEN"] = "t" * 32
            retained_path = directory / "publication.json"
            if reopening:
                # First retain the complete fixed vector in a no-work session.
                # Reopen actual native custodians before compiling any original,
                # with the same root deadline and independently retained floor.
                child(executable, test, "control", "portfolio_no_work", environment,
                      retained_path, retained_path)
                preplan = json.loads(retained_path.read_text())
                assert preplan["intent"] is None and preplan["actual"]["recovery"] is None
                environment = pending_environment(combined, environment, preplan)
                parent = json.loads(environment["DFMCP_COMBINED_PRODUCTION_CONFIG"])
                parent["workshop_prerequisite_checkpoint"] = \
                    preplan["actual"]["workshop_prerequisite"]["checkpoint"]
                environment["DFMCP_COMBINED_PRODUCTION_CONFIG"] = json.dumps(parent)
                peer.inventory_stack, peer.chair_stack = 2, 2 if one_order else 1
            child(executable, test, "control", requested_case, environment,
                  retained_path, retained_path)
            retained = json.loads(retained_path.read_text())
            if reopening:
                assert retained["goal"] == preplan["goal"], "reopen retargeted or renewed the root"
                assert retained["workforce_journal_id"] == preplan["workforce_journal_id"]
            if portfolio and retained["intent"] is not None:
                assert len(retained["intent"]["goal"]["targets"]) == 3
                assert [slot["step_id"] for slot in
                        retained["intent"]["recovery_reference"]["recovery"]["portfolio"]] == ([1] if one_order else [1, 2])
                reference = retained["intent"]["recovery_reference"]
                if clocks:
                    pins = reference["portfolio_clocks"]
                    assert len(pins) == (1 if one_order else 2)
                    for family in ("clock", "inventory_clock"):
                        assert reference[family] == pins[0][family]
                        for pin in pins:
                            assert (pin[family] is not None) == (family in clocks)
                            if family in clocks:
                                assert pin[family]["head"] == pin[family]["journal_id"]
                else:
                    assert "portfolio_clocks" not in reference
            calls = [operation for _, operation in peer.calls]
            assert calls.count("PrepareAssignment") == int(case in ("advanced", "prepared_floor"))
            assert not any(operation.startswith("Commit") for operation in calls)
            assert "PrepareOrder" not in calls
            assert not peer.workforce_applied and not peer.order_created
            if case in ("published", "partial", "advanced"):
                reopened = pending_environment(combined, environment, retained)
                offline = {key: value for key, value in reopened.items()
                           if not (key.startswith("DFMCP_") and (
                               key.endswith("_TOKEN") or key.endswith("_ALLOW_LABOR")
                               or key.endswith("_ALLOW_PRODUCTION") or key.endswith("_ALLOW_CLOCK")
                               or key.endswith("_ALLOW_INVENTORY_CLOCK")))}
                calls_before = (list(peer.calls), list(workshop.calls))
                files_before = combined.custody_files(production)
                child(executable, test, "recover", requested_case, offline, retained_path,
                      directory / "recovered.json")
                assert calls_before == (peer.calls, workshop.calls), "recovery contacted a native source"
                assert files_before == combined.custody_files(production), "recovery repaired or rewrote custody"
                if case == "published":
                    child(executable, test, "recover", prefix + "published_floor", offline,
                          retained_path, directory / "floor-recovered.json")
                    assert calls_before == (peer.calls, workshop.calls), "parent-floor recovery contacted a source"
                    assert files_before == combined.custody_files(production), "parent-floor recovery rewrote custody"
                    retained_files = [production / "parent" / name for name in (
                        "original.plan", "original.citizens", "original.workshop", "execution.journal")]
                    for slot in range(1, (1 if one_order else 2) + 1):
                        for family in clocks:
                            basename = family.replace("_", "-") + ".journal"
                            name = basename if slot == 1 else f"portfolio-{slot}-{basename}"
                            retained_files.append(production / "orders" / name)
                    for path in retained_files:
                        name = path.name
                        hidden = directory / f"saved-{path.parent.name}-{name}"
                        path.rename(hidden)
                        try:
                            absent = combined.custody_files(production)
                            child(executable, test, "recover", prefix + f"missing-{name}", offline,
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
                            child(executable, test, "recover", prefix + f"corrupt-{name}", offline,
                                  retained_path, directory / "refused.json")
                            assert corrupt == combined.custody_files(production)
                        finally:
                            path.write_bytes(original)
                        if path.parent.name == "orders" and name.startswith("portfolio-"):
                            # A valid complete header belonging to the first
                            # slot must not replace an inactive slot's custody.
                            first = path.with_name(name.removeprefix("portfolio-2-"))
                            try:
                                path.write_bytes(first.read_bytes())
                                swapped = combined.custody_files(production)
                                child(executable, test, "recover", prefix + f"substituted-{name}", offline,
                                      retained_path, directory / "refused.json")
                                assert swapped == combined.custody_files(production)
                            finally:
                                path.write_bytes(original)
                    foreign = json.loads(retained_path.read_text())
                    foreign["intent"]["recovery_reference"]["recovery"]["object_digest"] = \
                        hashlib.sha256(b"another complete stock-target original").hexdigest()
                    foreign_path = directory / "foreign.json"
                    foreign_path.write_text(json.dumps(foreign))
                    child(executable, test, "recover", prefix + "foreign-original", offline,
                          foreign_path, directory / "refused.json")
                    if portfolio:
                        for changed in ("missing-order-slot", "changed-met-minimum", "dropped-met-minimum"):
                            foreign = json.loads(retained_path.read_text())
                            if changed == "missing-order-slot":
                                foreign["intent"]["recovery_reference"]["recovery"]["portfolio"].pop()
                            elif changed == "changed-met-minimum":
                                foreign["goal"]["targets"][2]["minimum_stack_units"] = 2
                                foreign["intent"]["goal"]["targets"][2]["minimum_stack_units"] = 2
                            else:
                                foreign["goal"]["targets"].pop()
                                foreign["intent"]["goal"]["targets"].pop()
                            foreign_path.write_text(json.dumps(foreign))
                            child(executable, test, "recover", prefix + changed, offline,
                                  foreign_path, directory / "refused.json")
                            assert files_before == combined.custody_files(production)
                            assert calls_before == (peer.calls, workshop.calls)
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
    cases = ("no_work", "no_work_floor", "ack_refusal", "published", "partial", "advanced", "prepared_floor")
    for case in cases:
        run_case(executable, test, case)
    for case in (*cases, "reopened"):
        run_case(executable, test, "portfolio_" + case)
    for clocks in (("clock",), ("inventory_clock",), ("clock", "inventory_clock")):
        for one_order in (False, True):
            run_case(executable, test, "portfolio_published", clocks, one_order)
        for case in ("ack_refusal", "partial", "reopened_refusal"):
            run_case(executable, test, "portfolio_" + case, clocks)
    run_case(executable, test, "portfolio_no_work", ("clock", "inventory_clock"))
    print("project-publication-campaign:checked")


if __name__ == "__main__":
    main()
