#!/usr/bin/env python3

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import tempfile
import unittest
import sys
from pathlib import Path
from typing import Any

MODULE_PATH = Path(__file__).with_name("verify_live_read_acceptance.py")
SPEC = importlib.util.spec_from_file_location("verify_live_read_acceptance", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load verifier module")
verifier = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verifier
SPEC.loader.exec_module(verifier)


def digest(label: str) -> str:
    return hashlib.sha256(label.encode()).hexdigest()


def anchor(epoch: int, sequence: int, label: str) -> dict[str, Any]:
    return {
        "fortress_id": "77",
        "epoch": epoch,
        "sequence": sequence,
        "game_tick": 42_348_345,
        "state_hash": digest(label),
    }


class Fixture:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.source_root = root / "source"
        self.source_root.mkdir()
        self.contract_path = self.source_root / "architecture/live_read_acceptance_v1.json"
        self.verifier_path = self.source_root / "scripts/verify_live_read_acceptance.py"
        self.contract_path.parent.mkdir(parents=True)
        self.verifier_path.parent.mkdir(parents=True)
        self.contract_path.write_bytes(
            Path(__file__).parents[1].joinpath("architecture/live_read_acceptance_v1.json").read_bytes()
        )
        self.verifier_path.write_bytes(MODULE_PATH.read_bytes())
        self.contract = json.loads(self.contract_path.read_text())
        for relative in self.contract["source_binding"]["required_source_digests"].values():
            path = self.source_root / relative
            if path in {self.contract_path, self.verifier_path}:
                continue
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f"fixture source for {relative}\n", encoding="utf-8")
        self.dfmcp_commit = "1" * 40
        self.dfhack_commit = "2" * 40
        self.plugin_sha256 = digest("plugin")
        self.native_receipt_path = root / "native-build-receipt.json"
        native_receipt = {
            "schema": "dfmcp.dfhack-plugin-qualification/1",
            "status": "native-build-passed",
            "source": {
                "dfmcp_commit": self.dfmcp_commit,
                "dfmcp_dirty": False,
                "dfhack_commit": self.dfhack_commit,
            },
            "plugin": {
                "sha256": self.plugin_sha256,
                "rpc_methods": ["Handshake", "ReadObservation"],
                "mutation_rpc_methods": [],
            },
        }
        self.native_receipt_path.write_text(
            json.dumps(native_receipt, sort_keys=True) + "\n", encoding="utf-8"
        )
        self.events = self.make_events()
        self.evidence_path = root / "evidence.jsonl"
        self.write_events(self.events)

    def source_digests(self) -> dict[str, str]:
        return {
            name: verifier.sha256_file(self.source_root / relative)
            for name, relative in self.contract["source_binding"]["required_source_digests"].items()
        }

    def base_event(self, gate: str, case: str, result: str, error_code: str | None) -> dict[str, Any]:
        return {
            "schema": self.contract["event_schema"],
            "event_id": f"{gate.lower()}.{case}",
            "gate": gate,
            "case": case,
            "result": result,
            "error_code": error_code,
        }

    def observation(
        self,
        case: str,
        page_size: int,
        names_included: bool,
        capsule: str,
        snapshot: str,
        observation_anchor: dict[str, Any],
    ) -> dict[str, Any]:
        event = self.base_event("R3", case, "accepted", None)
        event.update(
            {
                "paused": True,
                "names_included": names_included,
                "page_size": page_size,
                "page_count": max(1, (3 + page_size - 1) // page_size),
                "citizen_count": 3,
                "complete": True,
                "publication_count": 1,
                "bridge_generation": 91,
                "capsule_sha256": capsule,
                "snapshot_sha256": snapshot,
                "citizen_identity_sha256": digest("citizen-identities"),
                "anchor": observation_anchor,
            }
        )
        return event

    def make_events(self) -> list[dict[str, Any]]:
        baseline_anchor = anchor(4, 9, "included-snapshot")
        omitted_anchor = anchor(4, 10, "omitted-snapshot")
        included_capsule = digest("included-capsule")
        included_snapshot = baseline_anchor["state_hash"]
        omitted_capsule = digest("omitted-capsule")
        omitted_snapshot = omitted_anchor["state_hash"]
        manifest = {
            "schema": self.contract["event_schema"],
            "event_id": "manifest",
            "gate": "meta",
            "case": "manifest",
            "result": "passed",
            "error_code": None,
            "run_id": "fixture-run-001",
            "synthetic": False,
            "source": {
                "dfmcp_commit": self.dfmcp_commit,
                "dfmcp_dirty": False,
                "dfhack_commit": self.dfhack_commit,
                "plugin_sha256": self.plugin_sha256,
                "native_build_receipt_sha256": verifier.sha256_file(self.native_receipt_path),
                "dwarf_fortress_version": "0.51.11",
                "dfhack_version": "0.51.11-r1",
                "bridge_version": "0.1.0",
                "bridge_protocol": "1.0",
            },
            "host": {"system": "Linux", "machine": "x86_64"},
            "source_digests": self.source_digests(),
        }
        events = [manifest]
        rejection_defaults = {
            "sensitive_manifest_disclosed": False,
            "bridge_generation": 0,
            "supported_methods": [],
            "world_loaded": False,
            "fortress_mode": False,
        }
        for expected in self.contract["gates"]["R2"]["required_cases"]:
            case = expected["case"]
            event = self.base_event("R2", case, expected["result"], expected["error_code"])
            if case in {
                "missing_token",
                "configured_token_short",
                "configured_token_long",
                "presented_token_short",
                "presented_token_long",
                "wrong_token",
                "nonce_short",
                "nonce_long",
                "protocol_mismatch",
            }:
                event.update(rejection_defaults)
            elif case == "correct_token":
                event.update(
                    {
                        "protocol_major": 1,
                        "protocol_minor": 0,
                        "bridge_generation": 91,
                        "supported_methods": ["Handshake", "ReadObservation"],
                        "nonce_correlated": True,
                        "world_loaded": True,
                        "fortress_mode": True,
                        "dwarf_fortress_version": "0.51.11",
                        "dfhack_version": "0.51.11-r1",
                        "bridge_version": "0.1.0",
                    }
                )
            elif case == "nonce_mismatch":
                event.update({"nonce_correlated": False, "published": False})
            elif case == "secret_scan":
                event.update(
                    {
                        "scanner": "fixture-secret-scanner/1",
                        "token_fingerprint_sha256": digest("fixture-token"),
                        "match_count": 0,
                        "scanned_artifacts": [
                            {"path": "logs/bridge.log", "sha256": digest("bridge-log")},
                            {"path": "logs/mcp.log", "sha256": digest("mcp-log")},
                        ],
                    }
                )
            events.append(event)
        observations = {
            "baseline_names_included": (4096, True, included_capsule, included_snapshot, baseline_anchor),
            "repeat_names_included": (4096, True, included_capsule, included_snapshot, baseline_anchor),
            "page_size_1": (1, True, included_capsule, included_snapshot, baseline_anchor),
            "page_size_2": (2, True, included_capsule, included_snapshot, baseline_anchor),
            "page_size_7": (7, True, included_capsule, included_snapshot, baseline_anchor),
            "page_size_64": (64, True, included_capsule, included_snapshot, baseline_anchor),
            "page_size_256": (256, True, included_capsule, included_snapshot, baseline_anchor),
            "page_size_4096": (4096, True, included_capsule, included_snapshot, baseline_anchor),
            "baseline_names_omitted": (4096, False, omitted_capsule, omitted_snapshot, omitted_anchor),
            "repeat_names_omitted": (4096, False, omitted_capsule, omitted_snapshot, omitted_anchor),
        }
        for expected in self.contract["gates"]["R3"]["required_cases"]:
            case = expected["case"]
            if case in observations:
                events.append(self.observation(case, *observations[case]))
                continue
            event = self.base_event("R3", case, expected["result"], expected["error_code"])
            if case == "offset_at_total":
                event.update(
                    {
                        "citizen_count": 3,
                        "requested_offset": 3,
                        "canonical_offset": 3,
                        "returned_citizens": 0,
                        "complete": True,
                    }
                )
            elif case == "offset_beyond_total":
                event.update(
                    {
                        "citizen_count": 3,
                        "requested_offset": 99,
                        "canonical_offset": 3,
                        "returned_citizens": 0,
                        "complete": True,
                    }
                )
            elif case == "running_multipage_rejected":
                event.update({"paused": False, "published": False, "pages_attempted": 1})
            events.append(event)
        restart_anchor = anchor(5, 0, "restart-snapshot")
        for expected in self.contract["gates"]["R4"]["required_cases"]:
            case = expected["case"]
            event = self.base_event("R4", case, expected["result"], expected["error_code"])
            if case == "restart_generation_changed":
                event.update(
                    {
                        "before_generation": 91,
                        "after_generation": 117,
                        "before_anchor": baseline_anchor,
                        "after_anchor": restart_anchor,
                    }
                )
            elif case == "old_client_rejected":
                event.update(
                    {"expected_generation": 91, "observed_generation": 117, "published": False}
                )
            elif case in {"world_unloaded", "non_fortress_mode", "summary_drift"}:
                event["published"] = False
            elif case == "partial_not_published":
                event.update(
                    {
                        "pages_received": 1,
                        "complete": False,
                        "published": False,
                        "canonical_anchor_issued": False,
                    }
                )
            elif case == "fresh_handshake":
                event.update(
                    {
                        "bridge_generation": 117,
                        "supported_methods": ["Handshake", "ReadObservation"],
                    }
                )
            events.append(event)
        r5 = self.base_event("R5", "cold_agent_turn", "accepted", None)
        r5.update(
            {
                "anchor": baseline_anchor,
                "capsule_sha256": included_capsule,
                "receipt_sha256": digest("live-observation-receipt"),
                "source": {
                    "dwarf_fortress_version": "0.51.11",
                    "dfhack_version": "0.51.11-r1",
                    "bridge_version": "0.1.0",
                },
                "authority": "read_only",
                "continuity": "bootstrap",
                "summary": {
                    "paused": True,
                    "current_year": 105,
                    "current_year_tick": 12_345,
                    "site_id": 7,
                    "citizen_count": 3,
                },
                "citizen_drilldown_bounded": True,
                "mutation_capabilities": [],
                "mutation_affordances": [],
                "mutation_recommendations": [],
                "coverage": [
                    {
                        "domain": "fortress.citizens.roster",
                        "status": "complete",
                        "epistemic_state": "observed",
                        "can_prove_absence": True,
                        "anchor_state_hash": baseline_anchor["state_hash"],
                    },
                    *[
                        {
                            "domain": domain,
                            "status": "omitted",
                            "epistemic_state": "unknown",
                            "can_prove_absence": False,
                        }
                        for domain in self.contract["gates"]["R5"]["required_omitted_domains"]
                    ],
                ],
            }
        )
        events.append(r5)
        return events

    def write_events(self, events: list[dict[str, Any]]) -> None:
        self.evidence_path.write_text(
            "".join(json.dumps(event, sort_keys=True) + "\n" for event in events),
            encoding="utf-8",
        )

    def options(self) -> Any:
        return verifier.VerificationOptions(
            source_root=self.source_root,
            expected_dfmcp_commit=self.dfmcp_commit,
            native_build_receipt=self.native_receipt_path,
        )

    def verify(self, events: list[dict[str, Any]] | None = None) -> dict[str, Any]:
        if events is not None:
            self.write_events(events)
        return verifier.verify_acceptance(self.evidence_path, self.contract_path, self.options())


class LiveReadAcceptanceTests(unittest.TestCase):
    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Fixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, Fixture(Path(temporary.name))

    def test_valid_evidence_is_qualified_and_deterministic(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            first = fixture.verify()
            second = fixture.verify()
            self.assertEqual(first, second)
            self.assertEqual(first["status"], "qualified")
            self.assertEqual([gate["gate"] for gate in first["gates"]], ["R2", "R3", "R4", "R5"])
            self.assertEqual(first["receipt_digest"], verifier.sha256_bytes(verifier.canonical_json({k: v for k, v in first.items() if k != "receipt_digest"})))

    def test_missing_case_fails_closed(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            events = copy.deepcopy(fixture.events)
            del events[2]
            with self.assertRaises(verifier.VerificationError):
                fixture.verify(events)

    def test_pagination_digest_drift_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            events = copy.deepcopy(fixture.events)
            target = next(event for event in events if event.get("case") == "page_size_7")
            target["capsule_sha256"] = digest("wrong-capsule")
            with self.assertRaises(verifier.VerificationError):
                fixture.verify(events)

    def test_secret_bearing_key_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            events = copy.deepcopy(fixture.events)
            target = next(event for event in events if event.get("case") == "wrong_token")
            target["bearer_token"] = "must-never-appear"
            with self.assertRaises(verifier.VerificationError):
                fixture.verify(events)

    def test_restart_generation_must_change(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            events = copy.deepcopy(fixture.events)
            target = next(event for event in events if event.get("case") == "restart_generation_changed")
            target["after_generation"] = target["before_generation"]
            with self.assertRaises(verifier.VerificationError):
                fixture.verify(events)

    def test_partial_capsule_cannot_publish(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            events = copy.deepcopy(fixture.events)
            target = next(event for event in events if event.get("case") == "partial_not_published")
            target["published"] = True
            with self.assertRaises(verifier.VerificationError):
                fixture.verify(events)

    def test_agent_turn_cannot_advertise_mutation(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            events = copy.deepcopy(fixture.events)
            target = next(event for event in events if event.get("case") == "cold_agent_turn")
            target["mutation_affordances"] = ["pause"]
            with self.assertRaises(verifier.VerificationError):
                fixture.verify(events)

    def test_duplicate_event_identity_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            events = copy.deepcopy(fixture.events)
            events[2]["event_id"] = events[1]["event_id"]
            with self.assertRaises(verifier.VerificationError):
                fixture.verify(events)

    def test_oversized_event_line_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            events = copy.deepcopy(fixture.events)
            events[1]["padding"] = "x" * (fixture.contract["limits"]["maximum_event_bytes"] + 1)
            fixture.write_events(events)
            with self.assertRaises(verifier.VerificationError):
                verifier.verify_acceptance(
                    fixture.evidence_path, fixture.contract_path, fixture.options()
                )


if __name__ == "__main__":
    unittest.main()
