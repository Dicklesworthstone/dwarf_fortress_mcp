#!/usr/bin/env python3

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any

MODULE_PATH = Path(__file__).with_name("verify_live_announcement_acceptance.py")
SPEC = importlib.util.spec_from_file_location(
    "verify_live_announcement_acceptance", MODULE_PATH
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load live announcement acceptance verifier")
verifier = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verifier
SPEC.loader.exec_module(verifier)


def digest(label: str) -> str:
    return hashlib.sha256(label.encode()).hexdigest()


class Fixture:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.contract_path = root / "contract.json"
        self.native_contract_path = root / "native-contract.json"
        self.base_native_path = root / "base-native.json"
        self.native_path = root / "native-v1-1.json"
        self.events_path = root / "events.jsonl"
        self.contract = json.loads(verifier.DEFAULT_CONTRACT.read_text(encoding="utf-8"))
        self.native_contract = json.loads(
            verifier.DEFAULT_NATIVE_CONTRACT.read_text(encoding="utf-8")
        )
        self.contract_path.write_text(
            json.dumps(self.contract, sort_keys=True) + "\n", encoding="utf-8"
        )
        self.native_contract_path.write_text(
            json.dumps(self.native_contract, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        self.dfmcp_commit = "1" * 40
        self.dfhack_commit = "2" * 40
        self.plugin_sha256 = digest("plugin")
        self.generation_digests = {
            name: digest(f"generation:{name}")
            for name in self.native_contract["required_source_digests"]
        }
        self.base_native = self.base_native_receipt()
        self.base_native_path.write_text(
            json.dumps(self.base_native, sort_keys=True) + "\n", encoding="utf-8"
        )
        self.native = self.native_receipt()
        self.write_native(self.native)
        self.events = self.make_events()
        self.write_events(self.events)

    def base_native_receipt(self) -> dict[str, Any]:
        source_digests = {
            "fixture": digest("fixture-source"),
            **{
                f"bound_{name}": value
                for name, value in self.generation_digests.items()
            },
        }
        return {
            "schema": verifier.promotion.NATIVE_RECEIPT_SCHEMA,
            "status": "native-build-passed",
            "source": {
                "dfmcp_commit": self.dfmcp_commit,
                "dfmcp_dirty": False,
                "dfhack_commit": self.dfhack_commit,
                "dfhack_dirty": False,
            },
            "plugin": {
                "sha256": self.plugin_sha256,
                "rpc_methods": verifier.promotion.EXPECTED_RPC_METHODS,
                "mutation_rpc_methods": [],
                "strings_inventory": "passed",
                "symbols_inventory": "passed",
            },
            "source_digests": source_digests,
        }

    def native_receipt(self) -> dict[str, Any]:
        unsigned: dict[str, Any] = {
            "schema": verifier.native_generation.RECEIPT_SCHEMA,
            "status": "qualified",
            "base_receipt": {
                "file_sha256": verifier.promotion.sha256_file(self.base_native_path),
                "content_digest": verifier.native_generation.sha256_bytes(
                    verifier.native_generation.canonical_json(self.base_native)
                ),
                "receipt": self.base_native,
                "source_digests": self.base_native["source_digests"],
            },
            "source": {
                "dfmcp_commit": self.dfmcp_commit,
                "dfmcp_dirty": False,
                "dfhack_commit": self.dfhack_commit,
                "dfhack_dirty": False,
            },
            "bridge": self.native_contract["bridge"],
            "plugin": {
                "sha256": self.plugin_sha256,
                "rpc_methods": ["Handshake", "ReadObservation"],
                "mutation_rpc_methods": [],
                "strings_inventory": "passed",
                "symbols_inventory": "passed",
            },
            "source_digests": self.generation_digests,
            "capabilities_granted": [],
            "mutation_capabilities": [],
            "claims_established": self.native_contract["claims_established"],
            "claims_not_established": self.native_contract[
                "claims_not_established"
            ],
        }
        receipt = {
            **unsigned,
            "receipt_digest": verifier.native_generation.sha256_bytes(
                verifier.native_generation.canonical_json(unsigned)
            ),
        }
        return verifier.native_generation.validate_receipt(
            receipt, self.native_contract
        )

    def write_native(self, value: dict[str, Any]) -> None:
        self.native_path.write_text(
            json.dumps(value, sort_keys=True) + "\n", encoding="utf-8"
        )

    def make_events(self) -> list[dict[str, Any]]:
        native_sha = verifier.promotion.sha256_file(self.native_path)
        events: list[dict[str, Any]] = []
        sequence = 0
        for gate in self.contract["gates"]:
            for case in gate["cases"]:
                sequence += 1
                artifacts = {
                    name: digest(f"{gate['gate']}:{case['case']}:{name}")
                    for name in case["required_artifact_digests"]
                }
                assertions = copy.deepcopy(case["required_equals"])
                event = {
                    "schema": self.contract["event_schema"],
                    "sequence": sequence,
                    "gate": gate["gate"],
                    "case": case["case"],
                    "status": "passed",
                    "source": {
                        "dfmcp_commit": self.dfmcp_commit,
                        "dfmcp_dirty": False,
                        "dfhack_commit": self.dfhack_commit,
                        "dfhack_dirty": False,
                        "plugin_sha256": self.plugin_sha256,
                        "native_build_receipt_sha256": native_sha,
                    },
                    "version_tuple": {
                        "dwarf_fortress": "0.51.11",
                        "dfhack": "0.51.11-r1",
                        "bridge": "0.2.0",
                        "protocol": "1.1",
                    },
                    "host": {"system": "Linux", "machine": "x86_64"},
                    "assertions": assertions,
                    "artifacts": artifacts,
                }
                event["evidence_digest"] = verifier.sha256_bytes(
                    verifier.canonical_json(
                        {"assertions": assertions, "artifacts": artifacts}
                    )
                )
                events.append(event)
        return events

    def write_events(self, events: list[dict[str, Any]]) -> None:
        self.events_path.write_text(
            "".join(
                json.dumps(event, separators=(",", ":"), ensure_ascii=False)
                + "\n"
                for event in events
            ),
            encoding="utf-8",
        )

    def verify(self) -> dict[str, Any]:
        return verifier.verify(
            self.events_path,
            self.native_path,
            self.contract_path,
            self.native_contract_path,
        )


class LiveAnnouncementAcceptanceTests(unittest.TestCase):
    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Fixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, Fixture(Path(temporary.name))

    def test_complete_exact_campaign_is_deterministic(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            first = fixture.verify()
            second = fixture.verify()
            self.assertEqual(first, second)
            self.assertEqual(first["status"], "qualified")
            self.assertEqual(
                [gate["gate"] for gate in first["gates"]],
                ["A1", "A2", "A3", "A4", "A5", "A6"],
            )
            self.assertEqual(first["evidence"]["event_count"], 43)
            self.assertEqual(first["mutation_capabilities"], [])
            unsigned = dict(first)
            del unsigned["receipt_digest"]
            self.assertEqual(
                first["receipt_digest"],
                verifier.sha256_bytes(verifier.canonical_json(unsigned)),
            )

    def test_native_generation_receipt_is_required(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.write_native(fixture.base_native)
            fixture.events = fixture.make_events()
            fixture.write_events(fixture.events)
            with self.assertRaises(verifier.native_generation.ReceiptError):
                fixture.verify()

    def test_missing_or_reordered_event_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.write_events(fixture.events[:-1])
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()
            reordered = copy.deepcopy(fixture.events)
            reordered[0], reordered[1] = reordered[1], reordered[0]
            fixture.write_events(reordered)
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_duplicate_json_key_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            lines = fixture.events_path.read_text(encoding="utf-8").splitlines()
            first = lines[0]
            first = first.replace(
                '"sequence":1', '"sequence":1,"sequence":1', 1
            )
            fixture.events_path.write_text(
                first + "\n" + "\n".join(lines[1:]) + "\n",
                encoding="utf-8",
            )
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_assertion_tampering_is_rejected_even_with_recomputed_digest(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            events = copy.deepcopy(fixture.events)
            events[0]["assertions"]["native_receipt_valid"] = False
            events[0]["evidence_digest"] = verifier.sha256_bytes(
                verifier.canonical_json(
                    {
                        "assertions": events[0]["assertions"],
                        "artifacts": events[0]["artifacts"],
                    }
                )
            )
            fixture.write_events(events)
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_evidence_digest_tampering_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            events = copy.deepcopy(fixture.events)
            events[0]["evidence_digest"] = digest("wrong-evidence")
            fixture.write_events(events)
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_source_or_native_receipt_mismatch_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            events = copy.deepcopy(fixture.events)
            events[3]["source"]["dfmcp_commit"] = "3" * 40
            fixture.write_events(events)
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

            fixture.write_events(fixture.events)
            native = copy.deepcopy(fixture.native)
            native["plugin"]["sha256"] = digest("different-plugin")
            unsigned = dict(native)
            unsigned.pop("receipt_digest", None)
            native["receipt_digest"] = verifier.native_generation.sha256_bytes(
                verifier.native_generation.canonical_json(unsigned)
            )
            fixture.write_native(native)
            fixture.events = fixture.make_events()
            fixture.write_events(fixture.events)
            with self.assertRaises(verifier.native_generation.ReceiptError):
                fixture.verify()

    def test_dirty_source_or_protocol_drift_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            events = copy.deepcopy(fixture.events)
            events[0]["source"]["dfhack_dirty"] = True
            fixture.write_events(events)
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

            events = copy.deepcopy(fixture.events)
            events[0]["version_tuple"]["protocol"] = "1.0"
            fixture.write_events(events)
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_forbidden_secret_marker_is_rejected_before_parsing(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            lines = fixture.events_path.read_text(encoding="utf-8").splitlines()
            first = json.loads(lines[0])
            first["DFMCP_BRIDGE_TOKEN"] = "must-not-appear"
            lines[0] = json.dumps(first, separators=(",", ":"))
            fixture.events_path.write_text(
                "\n".join(lines) + "\n", encoding="utf-8"
            )
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_extra_assertion_or_artifact_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            events = copy.deepcopy(fixture.events)
            events[0]["assertions"]["extra"] = True
            events[0]["evidence_digest"] = verifier.sha256_bytes(
                verifier.canonical_json(
                    {
                        "assertions": events[0]["assertions"],
                        "artifacts": events[0]["artifacts"],
                    }
                )
            )
            fixture.write_events(events)
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

            events = copy.deepcopy(fixture.events)
            events[0]["artifacts"]["extra"] = digest("extra")
            events[0]["evidence_digest"] = verifier.sha256_bytes(
                verifier.canonical_json(
                    {
                        "assertions": events[0]["assertions"],
                        "artifacts": events[0]["artifacts"],
                    }
                )
            )
            fixture.write_events(events)
            with self.assertRaises(verifier.VerificationError):
                fixture.verify()

    def test_cli_writes_canonical_atomic_receipt(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            output = fixture.root / "receipt.json"
            result = verifier.main(
                [
                    str(fixture.events_path),
                    str(fixture.native_path),
                    "--contract",
                    str(fixture.contract_path),
                    "--native-contract",
                    str(fixture.native_contract_path),
                    "--output",
                    str(output),
                ]
            )
            self.assertEqual(result, 0)
            self.assertEqual(json.loads(output.read_text()), fixture.verify())


if __name__ == "__main__":
    unittest.main()
