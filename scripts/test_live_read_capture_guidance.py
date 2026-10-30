#!/usr/bin/env python3

from __future__ import annotations

import copy
import importlib.util
import sys
import unittest
from pathlib import Path
from unittest import mock
import check_live_capture_plan as capture_gate

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))
MODULE_PATH = SCRIPT_DIR / "live_read_capture_guidance.py"
SPEC = importlib.util.spec_from_file_location("live_read_capture_guidance", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load live-read capture guidance")
guidance = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = guidance
SPEC.loader.exec_module(guidance)


class LiveReadCaptureGuidanceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.acceptance = guidance.journal.contract()
        cls.plan = guidance.read_plan()

    def test_plan_matches_every_acceptance_case_in_order(self) -> None:
        indexed = guidance.validate_plan(self.plan, self.acceptance)
        expected = guidance.journal.expected_sequence(self.acceptance)[1:]
        self.assertEqual(list(indexed), expected)

    def test_shell_interpreter_and_secret_fragments_are_rejected(self) -> None:
        with self.assertRaises(guidance.GuidanceError):
            guidance.validate_argv(["bash", "-c", "echo bad"], "argv")
        for executable in ["/bin/sh", "/usr/bin/bash", "powershell.exe", "/bin/PWSH"]:
            with self.subTest(executable=executable), self.assertRaises(guidance.GuidanceError):
                guidance.validate_argv([executable, "literal"], "argv")
        self.assertEqual(guidance.validate_argv(["probe", "handshake-case", "fresh_handshake"], "argv"),
                         ["probe", "handshake-case", "fresh_handshake"])
        with self.assertRaises(guidance.GuidanceError):
            guidance.validate_argv(
                ["probe", "DFMCP_BRIDGE_TOKEN=secret"], "argv"
            )

    def test_duplicate_case_is_rejected(self) -> None:
        plan = copy.deepcopy(self.plan)
        plan["cases"].insert(1, copy.deepcopy(plan["cases"][0]))
        with self.assertRaises(guidance.GuidanceError):
            guidance.validate_plan(plan, self.acceptance)

    def test_case_order_drift_is_rejected(self) -> None:
        plan = copy.deepcopy(self.plan)
        plan["cases"][0], plan["cases"][1] = plan["cases"][1], plan["cases"][0]
        with self.assertRaises(guidance.GuidanceError):
            guidance.validate_plan(plan, self.acceptance)

    def test_probe_guidance_is_ready_and_uses_append_probe(self) -> None:
        state = {
            "next_index": 4,
            "records": [{}, {}, {}, {}],
            "sealed": False,
        }
        with mock.patch.object(
            guidance.journal,
            "load_journal",
            return_value=(Path("/tmp/run"), state, self.acceptance),
        ), mock.patch.object(guidance, "read_plan", return_value=self.plan):
            result = guidance.next_guidance(Path("/tmp/run"))
        self.assertEqual(result["next"]["case"], "presented_token_short")
        self.assertTrue(result["next"]["ready_to_execute"])
        self.assertEqual(result["next"]["append_argv"][2], "append-probe")
        self.assertEqual(result["next"]["argv"][0], "cargo")
        self.assertEqual(result["command_representation"], "argv array without shell interpolation")

    def test_scanner_guidance_requires_artifact_root(self) -> None:
        state = {
            "next_index": 12,
            "records": [{} for _ in range(12)],
            "sealed": False,
        }
        with mock.patch.object(
            guidance.journal,
            "load_journal",
            return_value=(Path("/tmp/run"), state, self.acceptance),
        ), mock.patch.object(guidance, "read_plan", return_value=self.plan):
            result = guidance.next_guidance(Path("/tmp/run"))
        self.assertEqual(result["next"]["case"], "secret_scan")
        self.assertFalse(result["next"]["ready_to_execute"])
        self.assertEqual(result["next"]["required_inputs"], ["artifact_root"])
        self.assertEqual(result["next"]["append_argv"][2], "append")

    def test_scanner_guidance_substitutes_explicit_artifact_root(self) -> None:
        state = {
            "next_index": 12,
            "records": [{} for _ in range(12)],
            "sealed": False,
        }
        with mock.patch.object(
            guidance.journal,
            "load_journal",
            return_value=(Path("/tmp/run"), state, self.acceptance),
        ), mock.patch.object(guidance, "read_plan", return_value=self.plan):
            result = guidance.next_guidance(
                Path("/tmp/run"), artifact_root=Path("/tmp/artifacts")
            )
        self.assertTrue(result["next"]["ready_to_execute"])
        self.assertIn(str(Path("/tmp/artifacts").resolve()), result["next"]["argv"])
        self.assertEqual(result["next"]["required_inputs"], [])

    def test_composite_case_is_explicitly_not_automatable(self) -> None:
        first_r4 = 1 + len(self.acceptance["gates"]["R2"]["required_cases"]) + len(
            self.acceptance["gates"]["R3"]["required_cases"]
        )
        state = {
            "next_index": first_r4,
            "records": [{} for _ in range(first_r4)],
            "sealed": False,
        }
        with mock.patch.object(
            guidance.journal,
            "load_journal",
            return_value=(Path("/tmp/run"), state, self.acceptance),
        ), mock.patch.object(guidance, "read_plan", return_value=self.plan):
            result = guidance.next_guidance(Path("/tmp/run"))
        self.assertEqual(result["next"]["case"], "restart_generation_changed")
        self.assertFalse(result["next"]["automatable"])
        self.assertFalse(result["next"]["ready_to_execute"])
        self.assertEqual(result["next"]["capture_kind"], "composite")

    def test_complete_journal_returns_only_finalize_guidance(self) -> None:
        count = len(guidance.journal.expected_sequence(self.acceptance))
        state = {
            "next_index": count,
            "records": [{} for _ in range(count)],
            "sealed": False,
        }
        with mock.patch.object(
            guidance.journal,
            "load_journal",
            return_value=(Path("/tmp/run"), state, self.acceptance),
        ), mock.patch.object(guidance, "read_plan", return_value=self.plan):
            result = guidance.next_guidance(Path("/tmp/run"))
        self.assertTrue(result["complete"])
        self.assertIsNone(result["next"])
        self.assertEqual(result["finalize_argv"][2], "finalize")

    def test_capture_gate_allows_defensive_literals_but_rejects_executable_secret_routes(self) -> None:
        failures = []
        capture_gate.check_guidance(failures)
        self.assertEqual(failures, [])
        original = capture_gate.read
        for addition in [
            '\nSECRET = "DFMCP_BRIDGE_TOKEN=secret"\n',
            '\n__import__("subprocess").run(["probe"])\n',
            '\nimport subprocess\n',
        ]:
            def altered(path, failures):
                source = original(path, failures)
                return source + addition if path == capture_gate.GUIDANCE else source
            failures = []
            with mock.patch.object(capture_gate, "read", side_effect=altered):
                capture_gate.check_guidance(failures)
            self.assertTrue(failures, addition)

    def test_capture_gate_rejects_real_shell_arguments_without_substring_false_positives(self) -> None:
        failures = []
        capture_gate.check_plan(failures)
        self.assertEqual(failures, [])
        original = capture_gate.parse_json
        for argv in [["/bin/sh", "literal"], ["probe", "-c", "literal"]]:
            bad = copy.deepcopy(self.plan)
            bad["cases"][0]["argv"] = argv
            def altered(path, failures):
                return bad if path == capture_gate.PLAN else original(path, failures)
            failures = []
            with mock.patch.object(capture_gate, "parse_json", side_effect=altered):
                capture_gate.check_plan(failures)
            self.assertTrue(failures, argv)


if __name__ == "__main__":
    unittest.main()
