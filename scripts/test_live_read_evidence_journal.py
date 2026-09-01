#!/usr/bin/env python3

from __future__ import annotations

import copy
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

MODULE_PATH = SCRIPT_DIR / "live_read_evidence_journal.py"
SPEC = importlib.util.spec_from_file_location("live_read_evidence_journal", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load live-read evidence journal")
journal = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = journal
SPEC.loader.exec_module(journal)


class LiveReadEvidenceJournalTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.contract = journal.contract()

    def handshake_probe(self, case: str, accepted: bool, error_code: str | None) -> dict:
        return {
            "schema": journal.PROBE_SCHEMA,
            "kind": "handshake",
            "case": case,
            "accepted": accepted,
            "server_accepted": accepted,
            "error_code": error_code,
            "sensitive_manifest_disclosed": accepted,
            "bridge_generation": 42 if accepted else 0,
            "supported_methods": ["Handshake", "ReadObservation"] if accepted else [],
            "world_loaded": accepted,
            "fortress_mode": accepted,
            "nonce_correlated": True,
            "protocol_major": 1,
            "protocol_minor": 0,
            "bridge_version": "0.1.0" if accepted else "",
            "dfhack_version": "0.51.11-r1" if accepted else "",
            "dwarf_fortress_version": "0.51.11" if accepted else "",
        }

    def capsule_probe(self, page_size: int, names_included: bool = True) -> dict:
        digest = "a" * 64
        return {
            "schema": journal.PROBE_SCHEMA,
            "kind": "capsule",
            "paused": True,
            "names_included": names_included,
            "page_size": page_size,
            "page_count": 1,
            "citizen_count": 3,
            "complete": True,
            "publication_count": 1,
            "bridge_generation": 42,
            "capsule_sha256": digest,
            "snapshot_sha256": "b" * 64,
            "receipt_sha256": "c" * 64,
            "citizen_identity_sha256": "d" * 64,
            "anchor": {
                "fortress_id": "77",
                "epoch": 0,
                "sequence": 0,
                "game_tick": 1,
                "state_hash": "b" * 64,
            },
        }

    def test_wrong_token_probe_normalizes_without_secret_material(self) -> None:
        event = journal.normalize_probe(
            self.handshake_probe("wrong_token", False, "AUTH_FAILED"),
            "R2",
            "wrong_token",
            self.contract,
        )
        self.assertEqual(event["result"], "rejected")
        self.assertEqual(event["error_code"], "AUTH_FAILED")
        self.assertFalse(event["sensitive_manifest_disclosed"])
        self.assertNotIn("bearer_token", json.dumps(event))

    def test_probe_acceptance_must_match_the_normative_case(self) -> None:
        with self.assertRaises(journal.JournalError):
            journal.normalize_probe(
                self.handshake_probe("wrong_token", True, None),
                "R2",
                "wrong_token",
                self.contract,
            )

    def test_page_size_case_is_bound_to_the_requested_size(self) -> None:
        event = journal.normalize_probe(
            self.capsule_probe(7),
            "R3",
            "page_size_7",
            self.contract,
        )
        self.assertEqual(event["page_size"], 7)
        with self.assertRaises(journal.JournalError):
            journal.normalize_probe(
                self.capsule_probe(64),
                "R3",
                "page_size_7",
                self.contract,
            )

    def test_name_projection_cases_are_not_interchangeable(self) -> None:
        with self.assertRaises(journal.JournalError):
            journal.normalize_probe(
                self.capsule_probe(4096, False),
                "R3",
                "baseline_names_included",
                self.contract,
            )
        with self.assertRaises(journal.JournalError):
            journal.normalize_probe(
                self.capsule_probe(4096, True),
                "R3",
                "baseline_names_omitted",
                self.contract,
            )

    def test_secret_bearing_normalized_event_is_rejected(self) -> None:
        event = journal.event_base(self.contract, "R2", "missing_token")
        event.update(
            {
                "sensitive_manifest_disclosed": False,
                "bridge_generation": 0,
                "supported_methods": [],
                "world_loaded": False,
                "fortress_mode": False,
                "bearer_token": "must-not-be-recorded",
            }
        )
        with self.assertRaises(journal.verifier.VerificationError):
            journal.validate_event(event, self.contract, "R2", "missing_token")

    def test_append_writes_artifacts_before_advancing_root(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            run_directory = Path(temporary)
            (run_directory / journal.EVENTS_DIRECTORY).mkdir()
            (run_directory / journal.RAW_DIRECTORY).mkdir()
            state = {
                "schema": journal.JOURNAL_SCHEMA,
                "sealed": False,
                "development_evidence": False,
                "contract_sha256": "0" * 64,
                "source_commit": "1" * 40,
                "native_build_receipt_sha256": "2" * 64,
                "next_index": 1,
                "records": [
                    {
                        "index": 0,
                        "gate": "meta",
                        "case": "manifest",
                        "event_id": "manifest",
                        "event_file": "events/000-meta-manifest.json",
                        "event_sha256": "3" * 64,
                        "raw_file": None,
                        "raw_sha256": None,
                    }
                ],
            }
            probe = self.handshake_probe("missing_token", False, "AUTH_REQUIRED")
            event = journal.normalize_probe(
                probe, "R2", "missing_token", self.contract
            )
            result = journal.append_record(
                run_directory,
                state,
                self.contract,
                event,
                journal.canonical_json(probe),
            )
            self.assertEqual(result["recorded_events"], 2)
            self.assertTrue((run_directory / journal.STATE_FILE).is_file())
            self.assertEqual(json.loads((run_directory / journal.STATE_FILE).read_text())["next_index"], 2)
            self.assertEqual(len(list((run_directory / journal.EVENTS_DIRECTORY).iterdir())), 1)
            self.assertEqual(len(list((run_directory / journal.RAW_DIRECTORY).iterdir())), 1)

    def test_sealed_journal_rejects_new_events(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            run_directory = Path(temporary)
            (run_directory / journal.EVENTS_DIRECTORY).mkdir()
            (run_directory / journal.RAW_DIRECTORY).mkdir()
            state = {
                "sealed": True,
                "next_index": 1,
                "records": [],
            }
            probe = self.handshake_probe("missing_token", False, "AUTH_REQUIRED")
            event = journal.normalize_probe(
                probe, "R2", "missing_token", self.contract
            )
            with self.assertRaises(journal.JournalError):
                journal.append_record(
                    run_directory,
                    state,
                    self.contract,
                    event,
                    journal.canonical_json(probe),
                )

    def test_composite_cases_cannot_be_fabricated_from_one_probe(self) -> None:
        with self.assertRaises(journal.JournalError):
            journal.normalize_probe(
                self.capsule_probe(4096),
                "R4",
                "restart_generation_changed",
                self.contract,
            )

    def test_event_identity_must_match_next_slot(self) -> None:
        event = journal.event_base(self.contract, "R2", "wrong_token")
        event["gate"] = "R3"
        with self.assertRaises(journal.JournalError):
            journal.validate_event(event, self.contract, "R2", "wrong_token")


if __name__ == "__main__":
    unittest.main()
