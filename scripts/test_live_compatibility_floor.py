#!/usr/bin/env python3

from __future__ import annotations

import hashlib
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any

MODULE_PATH = Path(__file__).with_name("live_compatibility_floor.py")
SPEC = importlib.util.spec_from_file_location("live_compatibility_floor", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load compatibility floor module")
floor = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = floor
SPEC.loader.exec_module(floor)
promotion = floor.promotion


def digest(label: str) -> str:
    return hashlib.sha256(label.encode()).hexdigest()


def entry(label: str) -> dict[str, Any]:
    unsigned: dict[str, Any] = {
        "support_level": "experimental",
        "version_tuple": {
            "dwarf_fortress": f"0.51.11-{label}",
            "dfhack": f"0.51.11-r1-{label}",
            "bridge": "0.1.0",
            "protocol": "1.0",
        },
        "platform": {"system": "Linux", "machine": "x86_64"},
        "source": {
            "dfmcp_commit": digest(f"dfmcp-{label}")[:40],
            "dfmcp_dirty": False,
            "dfhack_commit": digest(f"dfhack-{label}")[:40],
            "plugin_sha256": digest(f"plugin-{label}"),
            "native_build_receipt_sha256": digest(f"native-{label}"),
            "live_acceptance_receipt_sha256": digest(f"live-file-{label}"),
            "live_acceptance_receipt_digest": digest(f"live-content-{label}"),
        },
        "gates": [
            {"gate": "R1", "status": "passed", "receipt_sha256": digest(f"native-{label}")},
            *[
                {
                    "gate": gate,
                    "status": "passed",
                    "case_count": count,
                    "evidence_digest": digest(f"{label}-{gate}"),
                }
                for gate, count in promotion.EXPECTED_LIVE_CASE_COUNTS.items()
            ],
        ],
        "capabilities": promotion.READ_ONLY_CAPABILITIES,
        "mutation_capabilities": [],
        "observed_domains": promotion.OBSERVED_DOMAINS,
        "conditional_domains": promotion.CONDITIONAL_DOMAINS,
        "omitted_domains": promotion.OMITTED_DOMAINS,
        "evidence_locator": f"qualification/{label}/receipt.json",
        "limitations": promotion.LIMITATIONS,
    }
    return {"entry_id": promotion.sha256_bytes(promotion.canonical_json(unsigned)), **unsigned}


def revoke(item: dict[str, Any]) -> dict[str, Any]:
    return promotion.build_revocation(
        item["entry_id"],
        "operational_withdrawal",
        "The operator withdrew this exact tuple after reviewing retained evidence.",
        "qualification/revocation/withdrawal.json",
        digest("withdrawal-evidence"),
    )


class Fixture:
    def __init__(self, root: Path) -> None:
        self.private = root / "private"
        self.private.mkdir(mode=0o700)
        self.private.chmod(0o700)
        self.floor = self.private / "floor.json"
        self.registry = root / "registry.json"
        self.write_registry(promotion.build_registry([], []))

    def write_registry(self, value: dict[str, Any], compact: bool = False) -> None:
        payload = json.dumps(value, sort_keys=True, separators=(",", ":")) if compact else json.dumps(value, sort_keys=True, indent=2)
        self.registry.write_text(payload + "\n", encoding="utf-8")

    def initialize(self) -> dict[str, Any]:
        return floor.initialize_floor(self.floor, self.registry)

    def sha(self) -> str:
        return hashlib.sha256(self.floor.read_bytes()).hexdigest()


class CompatibilityFloorTests(unittest.TestCase):
    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Fixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, Fixture(Path(temporary.name))

    def test_initialize_binds_empty_active_and_revocation_sets(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            value = fixture.initialize()
            self.assertEqual(value["schema"], floor.FLOOR_SCHEMA)
            self.assertEqual(value["entry_ids"], [])
            self.assertEqual(value["revocation_ids"], [])
            self.assertEqual(value["revoked_entry_ids"], [])
            self.assertEqual(value["active_entry_ids"], [])
            self.assertEqual(floor.verify_floor(fixture.floor, fixture.registry)[0], value)
            self.assertEqual(fixture.floor.stat().st_mode & 0o777, 0o600)

    def test_relative_permissive_and_symbolic_custody_fail_closed(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            with self.assertRaises(floor.FloorError):
                floor.initialize_floor(Path("relative.json"), fixture.registry)
            fixture.private.chmod(0o750)
            with self.assertRaises(floor.FloorError):
                fixture.initialize()
            fixture.private.chmod(0o700)
            fixture.initialize()
            fixture.floor.chmod(0o640)
            with self.assertRaises(floor.FloorError):
                floor.verify_floor(fixture.floor)
            fixture.floor.chmod(0o600)
            target = fixture.private / "target.json"
            fixture.floor.replace(target)
            try:
                fixture.floor.symlink_to(target)
            except OSError:
                return
            with self.assertRaises(floor.FloorError):
                floor.verify_floor(fixture.floor)

    def test_entry_and_revocation_advance_preserve_history(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            initial = fixture.initialize()
            item = entry("alpha")
            fixture.write_registry(promotion.build_registry([item], []))
            admitted, changed = floor.advance_floor(fixture.floor, fixture.registry, fixture.sha())
            self.assertTrue(changed)
            self.assertEqual(admitted["entry_ids"], [item["entry_id"]])
            self.assertEqual(admitted["active_entry_ids"], [item["entry_id"]])
            self.assertEqual(admitted["previous_floor_digest"], initial["floor_digest"])
            revoked = revoke(item)
            fixture.write_registry(promotion.build_registry([item], [revoked]))
            withdrawn, changed = floor.advance_floor(fixture.floor, fixture.registry, fixture.sha())
            self.assertTrue(changed)
            self.assertEqual(withdrawn["entry_ids"], [item["entry_id"]])
            self.assertEqual(withdrawn["revocation_ids"], [revoked["revocation_id"]])
            self.assertEqual(withdrawn["revoked_entry_ids"], [item["entry_id"]])
            self.assertEqual(withdrawn["active_entry_ids"], [])

    def test_historical_entry_and_revocation_rollback_are_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            item = entry("alpha")
            revoked = revoke(item)
            fixture.write_registry(promotion.build_registry([item], [revoked]))
            fixture.initialize()
            fixture.write_registry(promotion.build_registry([item], []))
            with self.assertRaises(floor.FloorError):
                floor.advance_floor(fixture.floor, fixture.registry, fixture.sha())
            fixture.write_registry(promotion.build_registry([], []))
            with self.assertRaises(floor.FloorError):
                floor.advance_floor(fixture.floor, fixture.registry, fixture.sha())

    def test_compare_and_swap_and_same_generation_semantics(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            initial = fixture.initialize()
            same, changed = floor.advance_floor(fixture.floor, fixture.registry, fixture.sha())
            self.assertFalse(changed)
            self.assertEqual(same, initial)
            fixture.write_registry(promotion.build_registry([entry("alpha")], []))
            with self.assertRaises(floor.FloorError):
                floor.advance_floor(fixture.floor, fixture.registry, digest("stale"))

    def test_formatting_only_registry_change_is_explicit_generation(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            value = promotion.build_registry([entry("alpha")], [])
            fixture.write_registry(value)
            initial = fixture.initialize()
            fixture.write_registry(value, compact=True)
            advanced, changed = floor.advance_floor(fixture.floor, fixture.registry, fixture.sha())
            self.assertTrue(changed)
            self.assertEqual(advanced["registry_digest"], initial["registry_digest"])
            self.assertNotEqual(advanced["registry_file_sha256"], initial["registry_file_sha256"])

    def test_legacy_floor_requires_explicit_v2_migration(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            generation = floor.registry_generation(fixture.registry)
            unsigned = {
                "schema": floor.LEGACY_FLOOR_SCHEMA,
                "sequence": 0,
                "registry_file_sha256": generation["registry_file_sha256"],
                "registry_digest": generation["registry_digest"],
                "entry_ids": generation["entry_ids"],
                "previous_floor_digest": None,
            }
            legacy = {**unsigned, "floor_digest": promotion.sha256_bytes(promotion.canonical_json(unsigned))}
            fixture.floor.write_text(json.dumps(legacy) + "\n", encoding="utf-8")
            fixture.floor.chmod(0o600)
            self.assertEqual(floor.verify_floor(fixture.floor, fixture.registry)[0]["schema"], floor.LEGACY_FLOOR_SCHEMA)
            migrated, changed = floor.advance_floor(fixture.floor, fixture.registry, fixture.sha())
            self.assertTrue(changed)
            self.assertEqual(migrated["schema"], floor.FLOOR_SCHEMA)
            self.assertEqual(migrated["previous_floor_digest"], legacy["floor_digest"])

    def test_legacy_floor_cannot_verify_a_revocation_generation(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            item = entry("alpha")
            fixture.write_registry(promotion.build_registry([item], []))
            generation = floor.registry_generation(fixture.registry)
            unsigned = {
                "schema": floor.LEGACY_FLOOR_SCHEMA,
                "sequence": 0,
                "registry_file_sha256": generation["registry_file_sha256"],
                "registry_digest": generation["registry_digest"],
                "entry_ids": generation["entry_ids"],
                "previous_floor_digest": None,
            }
            legacy = {**unsigned, "floor_digest": promotion.sha256_bytes(promotion.canonical_json(unsigned))}
            fixture.floor.write_text(json.dumps(legacy) + "\n", encoding="utf-8")
            fixture.floor.chmod(0o600)
            fixture.write_registry(promotion.build_registry([item], [revoke(item)]))
            with self.assertRaises(floor.FloorError):
                floor.verify_floor(fixture.floor, fixture.registry)

    def test_active_revoked_partition_and_digest_tampering_are_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            item = entry("alpha")
            fixture.write_registry(promotion.build_registry([item], [revoke(item)]))
            fixture.initialize()
            value = json.loads(fixture.floor.read_text(encoding="utf-8"))
            value["active_entry_ids"] = [item["entry_id"]]
            unsigned = dict(value)
            unsigned.pop("floor_digest", None)
            value["floor_digest"] = promotion.sha256_bytes(promotion.canonical_json(unsigned))
            fixture.floor.write_text(json.dumps(value) + "\n", encoding="utf-8")
            fixture.floor.chmod(0o600)
            with self.assertRaises(floor.FloorError):
                floor.verify_floor(fixture.floor)

    def test_duplicate_keys_and_concurrent_writers_are_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.floor.write_text('{"schema":"dfmcp.live-compatibility-floor/2","sequence":0,"sequence":1}\n', encoding="utf-8")
            fixture.floor.chmod(0o600)
            with self.assertRaises((floor.FloorError, promotion.PromotionError)):
                floor.verify_floor(fixture.floor)
            fixture.floor.unlink()
            with floor.floor_lock(fixture.floor):
                with self.assertRaises(floor.FloorError):
                    with floor.floor_lock(fixture.floor):
                        self.fail("second floor writer unexpectedly acquired the lock")


if __name__ == "__main__":
    unittest.main()
