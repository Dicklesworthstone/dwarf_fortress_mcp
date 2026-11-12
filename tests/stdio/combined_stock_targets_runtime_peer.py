#!/usr/bin/env python3
"""Run the real native stock-target portfolio owner against TCP source doubles.

This driver encodes complete operations/1.3 inventories and controlled source
changes. The Rust child owns planning, original archives, native dispatch,
reconciliation and goal proofs. These are component regressions, not live-game
qualification or MCP transport evidence.
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import sys
import tempfile

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    "stock_targets_runtime_helpers", HERE / "combined_stock_target_runtime_peer.py"
)
helpers = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helpers)


def inventory_capture(combined, tick, inventory):
    """One complete no-job inventory with stable native IDs for every item type."""
    year, annual = divmod(tick, 403200)
    jobs = (
        b"DFMJ1200"
        + struct.pack(">IIBiI", year, annual, 1, combined.SITE, 100)
        + combined.text(b"region1")
        + struct.pack(">I", 0)
    )
    raw = (
        b"DFMO1300"
        + struct.pack(">I", len(jobs))
        + jobs
        + struct.pack(">III", 100, 100, 0)
    )
    rows = [
        (index + 2, key, inventory.get(key, 0))
        for index, key in enumerate(("BED", "DOOR", "TABLE", "CHAIR"))
        if inventory.get(key, 0) > 0
    ]
    raw += struct.pack(">I", len(rows))
    for native_id, key, units in rows:
        raw += (
            struct.pack(">Ii", native_id, 5)
            + combined.text(key.encode("ascii"))
            + struct.pack(">iiiIiiiI", -1, 420, 0, units, 1, 1, 1, 64)
            + b"\x00\x00"
        )
    return raw + struct.pack(">I", 0)


def run_case(executable, test_name, case):
    portfolio = helpers.load(
        "stock_targets_portfolio_native_fixture", "combined_portfolio.py"
    )
    combined = portfolio.b
    with tempfile.TemporaryDirectory(prefix="dfmcp-stock-targets-runtime-") as temporary:
        directory = Path(temporary).resolve()
        production = directory / "production"
        production.mkdir(mode=0o700)
        stage = directory / "stage.txt"

        class StockTargetsPeer(portfolio.PortfolioPeer):
            def __init__(self):
                super().__init__(production)
                self.existing_labor_ready = True
                self.inventory = {
                    "BED": 6 if case == "already" else 2,
                    "DOOR": 1 if case.startswith("multiple") else 3,
                }
                self.last_stage = None
                self.injected_after_dispatch = False
                self.injected_prefix_change = False
                self.first_created_record = None

                def enabled_labor(capture):
                    capture["automatic"] = False
                    capture["details"] = []
                    capture["labor_keys"] = ["MINE", "CARPENTRY"]
                    for unit in capture["units"]:
                        unit["labors"] = [1, 0]

                self.workforce_capture_hook = enabled_labor

            def operations(self):
                return inventory_capture(combined, self.order_tick, self.inventory)

            def apply_stage(self):
                value = stage.read_text() if stage.exists() else ""
                if value == self.last_stage:
                    return
                self.last_stage = value
                if value == "before_prepare" and case == "before_stock":
                    # A previously met target remains part of the exact census,
                    # even though it has no native order of its own.
                    self.inventory["DOOR"] = 4
                elif value == "before_second_prepare" and case in (
                    "multiple_extra", "multiple_retired"
                ):
                    assert len(self.created_keys) == 1 and 100 in self.orders
                    self.injected_prefix_change = True
                    if case == "multiple_extra":
                        self.orders[101] = {
                            "recipe": 4, "amount": 1, "remaining": 1,
                            "status": 3, "retired": False,
                        }
                    else:
                        self.orders[100]["retired"] = True
                    self.order_sequence += 1
                elif value in ("goal_first", "goal_second", "goal_third"):
                    offset = {
                        "goal_first": 5, "goal_second": 10, "goal_third": 15
                    }[value]
                    self.inventory["BED"] = 6
                    self.inventory["DOOR"] = (
                        1 if case in ("parent_regression", "parent_clock")
                        and value == "goal_first" else 3
                    )
                    self.goal_tick(combined.TICK + offset)

            def reply(self, plugin, operation, request):
                self.apply_stage()
                if case == "after_stock" and not self.injected_after_dispatch:
                    if 2 in helpers.creation_states(production):
                        self.injected_after_dispatch = True
                        self.inventory["DOOR"] = 4
                if operation == "CommitOrder":
                    plan = self.plans[request[5]]
                    token = hashlib.sha256(
                        b"dfmcp-work-order-token/1\0"
                        + struct.pack(">Q", combined.OGEN)
                        + combined.text(plan["key"]) + plan["digest"]
                    ).digest()[:16]
                    assert request[10] == token, "commit must retain its native review"
                response = super().reply(plugin, operation, request)
                if operation == "CommitOrder" and len(self.created_keys) == 1:
                    self.first_created_record = self.effect(
                        self.plans[self.created_keys[0]], True
                    )
                if case == "lost" and operation == "CommitOrder":
                    raise ConnectionResetError(
                        "lost reply after original stock-target portfolio insertion"
                    )
                return response

        peer = None
        try:
            peer = StockTargetsPeer()
            environment = combined.environment(production, peer.endpoint)
            if case == "parent_clock":
                for name in (
                    "DFMCP_COMBINED_PRODUCTION_CONFIG",
                    "DFMCP_SEMANTIC_WORK_ORDER_CONFIG",
                ):
                    config = json.loads(environment[name])
                    config["inventory_clock"] = {
                        "generation": combined.INVENTORY_CLOCK_GENERATION
                    }
                    environment[name] = json.dumps(config)
                environment["DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK"] = "1"
                environment["DFMCP_SEMANTIC_WORK_ORDER_INVENTORY_CLOCK_TOKEN"] = (
                    combined.TOKEN.decode()
                )
            environment[helpers.STAGE] = str(stage)
            retained_path = directory / "retained.json"
            helpers.run_child(
                executable, test_name, "control", case, environment,
                directory / "unused.json", retained_path,
            )
            calls = [operation for _, operation in peer.calls]
            assert not peer.workforce_applied and peer.workforce_plan is None
            assert not any(operation in (
                "PrepareAssignment", "CommitAssignment", "QueryAssignment"
            ) for operation in calls), calls
            assert not peer.clock_commits
            if case == "parent_clock":
                assert len(peer.inventory_clock_commits) == 1
                run = peer.inventory_clock_plans[peer.inventory_clock_commits[0]]
                before = combined.inventory_run_wire.snapshot(run["before"])
                assert before["tick"] == combined.TICK + 10
                assert run["ticks"] == 5 and peer.order_tick == combined.TICK + 15
            else:
                assert not peer.inventory_clock_commits

            if case in ("mixed", "parent_regression", "parent_clock", "lost", "multiple",
                        "multiple_extra", "multiple_retired"):
                expected_count = 2 if case == "multiple" else 1
                assert calls.count("PrepareOrder") == expected_count
                assert calls.count("CommitOrder") == expected_count
                assert len(peer.created_keys) == expected_count
                assert peer.order_created
                retained = json.loads(retained_path.read_text())
                original = retained["original_plan"]["steps"]
                assert [key.decode() for key in peer.created_keys] == [
                    step["key"] for step in original[:expected_count]
                ]
                for index, (recipe, amount) in enumerate(((1, 4), (2, 2))[:expected_count]):
                    plan = peer.plans[peer.created_keys[index]]
                    assert (plan["id"], plan["recipe"], plan["amount"]) == (
                        100 + index, recipe, amount
                    )
                first = peer.plans[peer.created_keys[0]]
                assert peer.effect(first, True) == peer.first_created_record
                if case == "multiple":
                    second = peer.plans[peer.created_keys[1]]
                    assert second["before"] == first["after"]
                    assert second["sequence"] == first["sequence"] + 1
                    assert set(peer.orders) == {100, 101}
                if case in ("multiple_extra", "multiple_retired"):
                    assert peer.injected_prefix_change, "queue change missed second prepare"
                    assert len(original) == 2 and len(peer.plans) == 1
            elif case == "after_stock":
                assert peer.injected_after_dispatch, "late change missed durable dispatch"
                assert 2 in helpers.creation_states(production)
                assert calls.count("PrepareOrder") == 1 and "CommitOrder" not in calls
                assert not peer.order_created
            else:
                # No-op, oversized or early source-drift refusals must never
                # prepare a native mutation.
                assert not any(operation.startswith(("Prepare", "Commit", "Cancel"))
                               for operation in calls), calls
                assert not peer.order_created

            if case == "lost":
                retained = json.loads(retained_path.read_text())
                reopened = combined.reopen_environment(
                    environment, retained["recovery"],
                    retained["native_custody"]["workforce_journal_id"],
                )
                offline = {
                    key: value for key, value in reopened.items()
                    if not (key.startswith("DFMCP_") and (
                        key.endswith("_TOKEN")
                        or key.endswith("_ALLOW_LABOR")
                        or key.endswith("_ALLOW_PRODUCTION")
                    ))
                }
                peer.inventory = {"BED": 9, "DOOR": 9}
                before_calls = list(peer.calls)
                before_files = combined.custody_files(production)
                helpers.run_child(
                    executable, test_name, "offline", case, offline,
                    retained_path, directory / "offline.json",
                )
                assert before_calls == peer.calls, "Offline contacted a native source"
                assert before_files == combined.custody_files(production), (
                    "Offline changed original custody"
                )
                helpers.run_child(
                    executable, test_name, "reconcile", case, reopened,
                    retained_path, directory / "reconciled.json",
                )
                after = [operation for _, operation in peer.calls]
                assert after.count("PrepareOrder") == after.count("CommitOrder") == 1
                assert "QueryOrder" in after[len(before_calls):]
                assert len(peer.created_keys) == len(peer.plans) == 1
                plan = peer.plans[peer.created_keys[0]]
                assert plan["amount"] == 4
                assert plan["key"].decode() == retained["original_plan"]["steps"][0]["key"]
                assert peer.effect(plan, True) == peer.first_created_record
        finally:
            if peer is not None:
                peer.close()


def main():
    executable, test_name, campaign = sys.argv[1:]
    cases = {
        "planning": ("mixed", "already", "multiple", "cap"),
        "recovery": ("before_stock", "after_stock", "lost",
                     "multiple_extra", "multiple_retired"),
        "parent": ("parent_regression", "parent_clock"),
        "all": ("mixed", "already", "multiple", "cap", "before_stock",
                "after_stock", "lost", "parent_regression", "parent_clock",
                "multiple_extra", "multiple_retired"),
    }[campaign]
    for case in cases:
        run_case(executable, test_name, case)
    print(f"combined-stock-targets-campaign:{campaign}:checked")


if __name__ == "__main__":
    main()
