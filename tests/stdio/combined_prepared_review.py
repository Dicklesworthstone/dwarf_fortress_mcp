#!/usr/bin/env python3
"""Exercise fresh original-preparation review through the actual modern MCP server.

Authenticated TCP peers model native DFHack replies. They are component fixtures,
not a running game; this campaign does not establish production admission.
"""
from __future__ import annotations

import argparse
from collections import Counter
import importlib.util
import json
from pathlib import Path
import sys
import unittest

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location(
    "combined_prepared_review_fixture", Path(__file__).with_name("combined_production.py")
)
fixture = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = fixture
spec.loader.exec_module(fixture)


class PreparedReviewStdio(unittest.TestCase):
    def setUp(self):
        self.directory = LOGS / self._testMethodName
        self.directory.mkdir(mode=0o700)
        self.peer = fixture.Peer(self.directory)
        self.env = fixture.environment(self.directory, self.peer.endpoint)
        self.client = fixture.stdio.Client(self.directory, {}, env=self.env)
        self.openings = 0

    def tearDown(self):
        try:
            self.client.close()
        finally:
            self.peer.close()

    def writes(self):
        return Counter(
            method for _, method in self.peer.calls
            if method.startswith(("Prepare", "Commit", "Cancel"))
        )

    def prepare(self, step=0):
        opened = self.client.open(max_wall_millis=60000)
        self.assertTrue(opened["ok"], opened)
        sid = opened["agent_turn"]["session_id"]
        observed = self.client.tool("observe", session_id=sid)
        self.assertTrue(observed["ok"], observed)
        request = {
            "expected_observation": observed["observation"]["seal"],
            "units": ["43"], "labor": "MINE", "enabled": True,
            "recipe": "wooden_bed", "amount": 1,
            "deadline_tick": 300, "poll_interval_ticks": 5, "stable_samples": 2,
        }
        planned = self.client.tool("plan", session_id=sid, request=json.dumps(request))
        self.assertTrue(planned["ok"], planned)
        original = planned["original_plan"]
        prepared = self.next_step(sid, original)
        if step == 1:
            applied = self.commit(sid, prepared)
            self.assertTrue(applied["ok"], applied)
            prepared = self.next_step(sid, original)
        self.assertEqual(prepared["review"]["step_id"], step)
        self.assertEqual(prepared["review"]["idempotency_key"], original["steps"][step]["key"])
        return sid, original, prepared

    def next_step(self, sid, original):
        prepared = self.client.tool("plan", session_id=sid, request=json.dumps({
            "kind": "step", "original_plan_digest": original["plan_digest"],
        }))
        self.assertTrue(prepared["ok"], prepared)
        return prepared

    def reopen(self, previous, mode="control"):
        self.client.close(crash=True)
        env = fixture.reopen_environment(
            self.env, previous["recovery"], previous["native_custody"]["workforce_journal_id"]
        )
        if mode != "control":
            env.pop("DFMCP_SEMANTIC_WORKFORCE_ALLOW_LABOR", None)
            env.pop("DFMCP_SEMANTIC_WORK_ORDER_ALLOW_PRODUCTION", None)
        self.openings += 1
        self.client = fixture.stdio.Client(
            self.directory, {}, suffix=f"reopened-{self.openings}-{mode}", env=env
        )
        opened = self.client.open(mode=mode, max_wall_millis=60000)
        self.assertTrue(opened["ok"], opened)
        self.assertIsNone(opened.get("review"))
        self.assertFalse(opened["original_goal_proven"])
        return opened["agent_turn"]["session_id"], opened

    def review(self, sid, original, **extra):
        return self.client.tool("plan", session_id=sid, request=json.dumps({
            "kind": "review_prepared", "original_plan_digest": original["plan_digest"], **extra,
        }))

    def commit(self, sid, prepared):
        return self.client.tool(
            "commit", session_id=sid, paired_seal=prepared["review"]["paired_seal"], confirmed=True
        )

    def assert_same_original(self, before, after):
        self.assertEqual(after["original_plan"], before["original_plan"])
        for field in ("step_id", "idempotency_key", "family"):
            self.assertEqual(after["review"][field], before["review"][field])
        self.assertNotEqual(after["review"]["paired_seal"], before["review"]["paired_seal"])
        self.assertNotEqual(after["recovery"]["frontier"]["head"], before["recovery"]["frontier"]["head"])

    def test_restarted_labor_can_finish_original_without_second_native_prepare(self):
        _, original, prepared = self.prepare()
        native_plan = dict(self.peer.workforce_plan)
        sid, _ = self.reopen(prepared)
        before = self.writes()
        inspected = self.client.tool("query", session_id=sid)
        self.assertTrue(inspected["ok"], inspected)
        self.assertIsNone(inspected.get("review"))
        reviewed = self.review(sid, original)
        self.assertTrue(reviewed["ok"], reviewed)
        self.assert_same_original(prepared, reviewed)
        self.assertEqual(self.writes(), before)
        self.assertEqual(self.peer.workforce_plan, native_plan)
        applied = self.commit(sid, reviewed)
        self.assertTrue(applied["ok"], applied)
        self.assertTrue(self.peer.workforce_applied)
        order = self.next_step(sid, original)
        completed = self.commit(sid, order)
        self.assertTrue(completed["ok"], completed)
        self.assertTrue(completed["frontier"]["all_applied"])
        self.assertFalse(completed["original_goal_proven"])
        self.assertEqual(self.writes(), Counter({
            "PrepareAssignment": 1, "CommitAssignment": 1, "PrepareOrder": 1, "CommitOrder": 1,
        }))

    def test_restarted_order_keeps_predecessor_and_original_native_preparation(self):
        _, original, prepared = self.prepare(step=1)
        native_plan = dict(self.peer.order_plan)
        sid, _ = self.reopen(prepared)
        before = self.writes()
        reviewed = self.review(sid, original)
        self.assertTrue(reviewed["ok"], reviewed)
        self.assert_same_original(prepared, reviewed)
        self.assertEqual(self.writes(), before)
        self.assertEqual(self.peer.order_plan, native_plan)
        result = self.commit(sid, reviewed)
        self.assertTrue(result["ok"], result)
        self.assertTrue(result["frontier"]["all_applied"])
        self.assertTrue(self.peer.order_created)
        self.assertEqual(self.writes()["CommitAssignment"], 1)
        self.assertEqual(self.writes()["PrepareOrder"], 1)
        self.assertEqual(self.writes()["CommitOrder"], 1)

    def test_every_fresh_review_invalidates_previously_issued_seals(self):
        _, original, prepared = self.prepare()
        sid, _ = self.reopen(prepared)
        first = self.review(sid, original)
        self.assertTrue(first["ok"], first)
        second = self.review(sid, original)
        self.assertTrue(second["ok"], second)
        self.assert_same_original(first, second)
        rejected = self.commit(sid, first)
        self.assertFalse(rejected["ok"], rejected)
        self.assertFalse(self.peer.workforce_applied)
        final = self.review(sid, original)
        self.assertTrue(final["ok"], final)
        self.assertTrue(self.commit(sid, final)["ok"])
        self.assertEqual(self.writes(), Counter({"PrepareAssignment": 1, "CommitAssignment": 1}))

    def test_offline_and_reconcile_cannot_issue_a_review(self):
        _, original, prepared = self.prepare()
        before = self.writes()
        for mode in ("offline", "reconcile"):
            with self.subTest(mode=mode):
                sid, opened = self.reopen(prepared, mode=mode)
                calls = len(self.peer.calls)
                rejected = self.review(sid, original)
                self.assertFalse(rejected["ok"], rejected)
                self.assertEqual(self.writes(), before)
                if mode == "offline":
                    self.assertEqual(len(self.peer.calls), calls)
                self.assertIsNone(opened.get("review"))

    def test_changed_workforce_capture_refuses_without_effect_or_reprepare(self):
        _, original, prepared = self.prepare()
        sid, _ = self.reopen(prepared)
        before = self.writes()
        self.peer.workforce_sequence = 9
        rejected = self.review(sid, original)
        self.assertFalse(rejected["ok"], rejected)
        self.assertEqual(self.writes(), before)
        self.assertFalse(self.peer.workforce_applied)

    def test_changed_order_capture_refuses_without_effect_or_reprepare(self):
        _, original, prepared = self.prepare(step=1)
        sid, _ = self.reopen(prepared)
        before = self.writes()
        self.peer.order_sequence += 1
        rejected = self.review(sid, original)
        self.assertFalse(rejected["ok"], rejected)
        self.assertEqual(self.writes(), before)
        self.assertFalse(self.peer.order_created)

    def test_possible_dispatch_cannot_be_reviewed_or_retried_after_restart(self):
        sid, original, prepared = self.prepare()
        self.peer.drop_workforce_reply = True
        uncertain = self.commit(sid, prepared)
        self.assertFalse(uncertain["ok"], uncertain)
        self.assertTrue(self.peer.workforce_applied)
        previous = dict(prepared)
        previous["recovery"] = uncertain["retained_reference"]["recovery"]
        sid, _ = self.reopen(previous)
        before = self.writes()
        rejected = self.review(sid, original)
        self.assertFalse(rejected["ok"], rejected)
        self.assertEqual(self.writes(), before)
        self.assertEqual(before["CommitAssignment"], 1)
        self.assertFalse(self.peer.order_created)

    def test_wrong_original_and_native_selector_are_refused_before_effects(self):
        _, original, prepared = self.prepare()
        sid, _ = self.reopen(prepared)
        before = self.writes()
        for extra in ({"original_plan_digest": "0" * 64}, {"idempotency_key": "replacement"}):
            with self.subTest(extra=extra):
                rejected = self.review(sid, original, **extra)
                self.assertFalse(rejected["ok"], rejected)
                self.assertEqual(self.writes(), before)
        self.assertFalse(self.peer.workforce_applied)


def main():
    global LOGS
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--logs", type=Path, required=True)
    args = parser.parse_args()
    fixture.stdio.BINARY = args.binary.resolve(strict=True)
    fixture.stdio.SERVER_ARGS = ["serve-gameplay-dev", "labor-order"]
    LOGS = args.logs.resolve()
    LOGS.mkdir(mode=0o700, parents=True, exist_ok=False)
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(PreparedReviewStdio)
    if suite.countTestCases() != 8:
        raise RuntimeError("prepared-review campaign must retain its eight-case denominator")
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    summary = {
        "schema": "dfmcp-combined-prepared-review-stdio/1",
        "binary": str(fixture.stdio.BINARY),
        "transport": "actual modern MCP stdio",
        "native_peers": "authenticated TCP component fixtures",
        "live_game": False,
        "expected_cases": 8,
        "tests_run": result.testsRun,
        "failures": len(result.failures),
        "errors": len(result.errors),
        "skipped": len(result.skipped),
        "passed": result.wasSuccessful() and result.testsRun == 8 and not result.skipped,
    }
    (LOGS / "result.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    return 0 if summary["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
