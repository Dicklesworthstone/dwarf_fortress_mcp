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

MODULE_PATH = Path(__file__).with_name("promote_live_compatibility.py")
SPEC = importlib.util.spec_from_file_location("promote_live_compatibility", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load live compatibility promotion module")
promotion = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = promotion
SPEC.loader.exec_module(promotion)


def digest(label: str) -> str:
    return hashlib.sha256(label.encode()).hexdigest()


class Fixture:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.registry_path = root / "registry.json"
        self.live_path = root / "live.json"
        self.native_path = root / "native.json"
        self.dfmcp_commit = "1" * 40
        self.dfhack_commit = "2" * 40
        self.plugin_sha256 = digest("plugin")
        self.registry_path.write_text(
            json.dumps(
                {
                    "schema_version": promotion.REGISTRY_SCHEMA,
                    "status": "no_admitted_live_tuples",
                    "entries": [],
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        self.write_native(self.native_receipt())
        self.write_live(self.live_receipt())

    def native_receipt(self) -> dict[str, Any]:
        return {
            "schema": promotion.NATIVE_RECEIPT_SCHEMA,
            "status": "native-build-passed",
            "source": {
                "dfmcp_commit": self.dfmcp_commit,
                "dfmcp_dirty": False,
                "dfhack_commit": self.dfhack_commit,
            },
            "plugin": {
                "sha256": self.plugin_sha256,
                "rpc_methods": promotion.EXPECTED_RPC_METHODS,
                "mutation_rpc_methods": [],
                "strings_inventory": "passed",
                "symbols_inventory": "passed",
            },
        }

    def live_receipt(self) -> dict[str, Any]:
        unsigned: dict[str, Any] = {
            "schema": promotion.LIVE_RECEIPT_SCHEMA,
            "status": "qualified",
            "run_id": "compatibility-fixture-001",
            "source": {
                "dfmcp_commit": self.dfmcp_commit,
                "dfmcp_dirty": False,
                "dfhack_commit": self.dfhack_commit,
                "plugin_sha256": self.plugin_sha256,
                "native_build_receipt_sha256": promotion.sha256_file(self.native_path),
                "source_digests": {"fixture": digest("fixture-source")},
            },
            "version_tuple": {
                "dwarf_fortress": "0.51.11",
                "dfhack": "0.51.11-r1",
                "bridge": "0.1.0",
                "protocol": "1.0",
            },
            "host": {"system": "Linux", "machine": "x86_64"},
            "evidence": {
                "stream_sha256": digest("stream"),
                "event_count": 35,
                "canonical_events_sha256": digest("events"),
            },
            "gates": [
                {
                    "gate": gate,
                    "status": "passed",
                    "case_count": count,
                    "evidence_digest": digest(f"gate-{gate}"),
                }
                for gate, count in promotion.EXPECTED_LIVE_CASE_COUNTS.items()
            ],
            "claims_established": ["fixture"],
            "claims_not_established": ["mutation"],
        }
        return {
            **unsigned,
            "receipt_digest": promotion.sha256_bytes(promotion.canonical_json(unsigned)),
        }

    def write_native(self, value: dict[str, Any]) -> None:
        self.native_path.write_text(json.dumps(value, sort_keys=True) + "\n", encoding="utf-8")

    def write_live(self, value: dict[str, Any]) -> None:
        self.live_path.write_text(json.dumps(value, sort_keys=True) + "\n", encoding="utf-8")

    def promote(self, expected_registry_sha256: str | None = None) -> dict[str, Any]:
        return promotion.promote(
            self.registry_path,
            self.live_path,
            self.native_path,
            "qualification/fixture-run/live-read-acceptance-receipt.json",
            expected_registry_sha256,
        )


class CompatibilityPromotionTests(unittest.TestCase):
    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Fixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, Fixture(Path(temporary.name))

    def test_qualified_receipts_promote_deterministically(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            first = fixture.promote()
            second = fixture.promote()
            self.assertEqual(first, second)
            self.assertEqual(first["status"], "admitted_live_tuples")
            entry = first["entries"][0]
            unsigned = dict(entry)
            del unsigned["entry_id"]
            self.assertEqual(
                entry["entry_id"],
                promotion.sha256_bytes(promotion.canonical_json(unsigned)),
            )
            self.assertEqual(entry["mutation_capabilities"], [])
            self.assertEqual(
                [gate["gate"] for gate in entry["gates"]],
                ["R1", "R2", "R3", "R4", "R5"],
            )

    def test_development_or_synthetic_status_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            value = fixture.live_receipt()
            value["status"] = "development-evidence"
            fixture.write_live(value)
            with self.assertRaises(promotion.PromotionError):
                fixture.promote()

    def test_live_receipt_field_tampering_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            value = fixture.live_receipt()
            value["version_tuple"]["dfhack"] = "tampered"
            fixture.write_live(value)
            with self.assertRaises(promotion.PromotionError):
                fixture.promote()

    def test_native_receipt_digest_mismatch_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            native = fixture.native_receipt()
            native["plugin"]["sha256"] = digest("different-plugin")
            fixture.write_native(native)
            with self.assertRaises(promotion.PromotionError):
                fixture.promote()

    def test_incomplete_binary_inventory_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            native = fixture.native_receipt()
            native["plugin"]["symbols_inventory"] = "skipped"
            fixture.write_native(native)
            fixture.write_live(fixture.live_receipt())
            with self.assertRaises(promotion.PromotionError):
                fixture.promote()

    def test_mutation_rpc_admission_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            native = fixture.native_receipt()
            native["plugin"]["mutation_rpc_methods"] = ["Pause"]
            fixture.write_native(native)
            fixture.write_live(fixture.live_receipt())
            with self.assertRaises(promotion.PromotionError):
                fixture.promote()

    def test_duplicate_exact_tuple_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            first = fixture.promote()
            fixture.registry_path.write_text(json.dumps(first, sort_keys=True) + "\n", encoding="utf-8")
            with self.assertRaises(promotion.PromotionError):
                fixture.promote()

    def test_existing_entry_identifier_tampering_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            registry = fixture.promote()
            registry["entries"][0]["entry_id"] = digest("wrong-entry")
            fixture.registry_path.write_text(json.dumps(registry, sort_keys=True) + "\n", encoding="utf-8")
            with self.assertRaises(promotion.PromotionError):
                fixture.promote()

    def test_evidence_locator_traversal_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            with self.assertRaises(promotion.PromotionError):
                promotion.promote(
                    fixture.registry_path,
                    fixture.live_path,
                    fixture.native_path,
                    "../outside/receipt.json",
                )

    def test_duplicate_json_keys_are_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.registry_path.write_text(
                '{"schema_version":"dfmcp.live-compatibility-registry/1",'
                '"status":"no_admitted_live_tuples","status":"admitted_live_tuples",'
                '"entries":[]}\n',
                encoding="utf-8",
            )
            with self.assertRaises(promotion.PromotionError):
                fixture.promote()

    def test_expected_registry_digest_is_a_compare_and_swap_fence(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            actual = promotion.sha256_file(fixture.registry_path)
            self.assertEqual(fixture.promote(actual)["status"], "admitted_live_tuples")
            with self.assertRaises(promotion.PromotionError):
                fixture.promote(digest("stale-registry"))

    def test_self_digested_but_structurally_invalid_existing_entry_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            registry = fixture.promote()
            entry = registry["entries"][0]
            entry["unexpected_authority"] = "pause"
            unsigned = dict(entry)
            del unsigned["entry_id"]
            entry["entry_id"] = promotion.sha256_bytes(promotion.canonical_json(unsigned))
            fixture.registry_path.write_text(json.dumps(registry, sort_keys=True) + "\n", encoding="utf-8")
            with self.assertRaises(promotion.PromotionError):
                fixture.promote()

    def test_lock_prevents_concurrent_in_place_promotion(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            with promotion.registry_lock(fixture.registry_path):
                with self.assertRaises(promotion.PromotionError):
                    with promotion.registry_lock(fixture.registry_path):
                        self.fail("second lock unexpectedly acquired")

    def test_candidate_identity_is_returned_independently_of_sort_position(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            candidate_registry, candidate_id = promotion._promote(
                fixture.registry_path,
                fixture.live_path,
                fixture.native_path,
                "qualification/fixture-run/live-read-acceptance-receipt.json",
            )
            candidate = candidate_registry["entries"][0]
            existing = copy.deepcopy(candidate)
            for counter in range(1, 10_000):
                existing["source"]["dfmcp_commit"] = f"{counter:040x}"
                existing["source"]["plugin_sha256"] = digest(f"other-plugin-{counter}")
                unsigned = dict(existing)
                unsigned.pop("entry_id", None)
                existing["entry_id"] = promotion.sha256_bytes(promotion.canonical_json(unsigned))
                if existing["entry_id"] > candidate_id:
                    break
            else:
                self.fail("could not construct a lexicographically later valid entry")
            fixture.registry_path.write_text(
                json.dumps(
                    {
                        "schema_version": promotion.REGISTRY_SCHEMA,
                        "status": "admitted_live_tuples",
                        "entries": [existing],
                    },
                    sort_keys=True,
                )
                + "\n",
                encoding="utf-8",
            )
            output, returned = promotion._promote(
                fixture.registry_path,
                fixture.live_path,
                fixture.native_path,
                "qualification/fixture-run/live-read-acceptance-receipt.json",
            )
            self.assertEqual(returned, candidate_id)
            self.assertIn(candidate_id, [entry["entry_id"] for entry in output["entries"]])
            self.assertNotEqual(output["entries"][-1]["entry_id"], candidate_id)


if __name__ == "__main__":
    unittest.main()
