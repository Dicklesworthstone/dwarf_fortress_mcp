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
            {
                "gate": "R1",
                "status": "passed",
                "receipt_sha256": digest(f"native-{label}"),
            },
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
    return {
        "entry_id": promotion.sha256_bytes(promotion.canonical_json(unsigned)),
        **unsigned,
    }


def registry(entries: list[dict[str, Any]]) -> dict[str, Any]:
    ordered = sorted(entries, key=lambda item: item["entry_id"])
    return {
        "schema_version": promotion.REGISTRY_SCHEMA,
        "status": "admitted_live_tuples" if ordered else "no_admitted_live_tuples",
        "entries": ordered,
    }


class Fixture:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.private = root / "private"
        self.private.mkdir(mode=0o700)
        self.private.chmod(0o700)
        self.floor_path = self.private / "floor.json"
        self.registry_path = root / "registry.json"
        self.write_registry(registry([]))

    def write_registry(self, value: dict[str, Any], *, compact: bool = False) -> None:
        if compact:
            payload = json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n"
        else:
            payload = json.dumps(value, sort_keys=True, indent=2) + "\n"
        self.registry_path.write_text(payload, encoding="utf-8")

    def initialize(self) -> dict[str, Any]:
        return floor.initialize_floor(self.floor_path, self.registry_path)

    def file_sha256(self) -> str:
        return hashlib.sha256(self.floor_path.read_bytes()).hexdigest()


class CompatibilityFloorTests(unittest.TestCase):
    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Fixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, Fixture(Path(temporary.name))

    def test_initialize_and_verify_exact_generation(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            initialized = fixture.initialize()
            verified, file_sha256 = floor.verify_floor(
                fixture.floor_path, fixture.registry_path
            )
            self.assertEqual(initialized, verified)
            self.assertEqual(initialized["sequence"], 0)
            self.assertEqual(initialized["entry_ids"], [])
            self.assertEqual(file_sha256, fixture.file_sha256())
            self.assertEqual(fixture.floor_path.stat().st_mode & 0o777, 0o600)

    def test_relative_floor_path_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            with self.assertRaises(floor.FloorError):
                floor.initialize_floor(Path("relative-floor.json"), fixture.registry_path)

    def test_permissive_parent_directory_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.private.chmod(0o750)
            with self.assertRaises(floor.FloorError):
                fixture.initialize()

    def test_permissive_floor_file_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.initialize()
            fixture.floor_path.chmod(0o640)
            with self.assertRaises(floor.FloorError):
                floor.verify_floor(fixture.floor_path, fixture.registry_path)

    def test_symbolic_link_floor_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.initialize()
            real = fixture.private / "real-floor.json"
            fixture.floor_path.replace(real)
            try:
                fixture.floor_path.symlink_to(real)
            except OSError:
                self.skipTest("symbolic links are unavailable")
            with self.assertRaises(floor.FloorError):
                floor.verify_floor(fixture.floor_path, fixture.registry_path)

    def test_advance_appends_entries_and_chains_floor_digest(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            initial = fixture.initialize()
            fixture.write_registry(registry([entry("alpha")]))
            advanced, changed = floor.advance_floor(
                fixture.floor_path, fixture.registry_path, fixture.file_sha256()
            )
            self.assertTrue(changed)
            self.assertEqual(advanced["sequence"], 1)
            self.assertEqual(advanced["previous_floor_digest"], initial["floor_digest"])
            self.assertEqual(advanced["entry_ids"], [entry("alpha")["entry_id"]])
            self.assertEqual(
                floor.verify_floor(fixture.floor_path, fixture.registry_path)[0],
                advanced,
            )

    def test_registry_rollback_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.write_registry(registry([entry("alpha")]))
            fixture.initialize()
            fixture.write_registry(registry([]))
            with self.assertRaises(floor.FloorError):
                floor.advance_floor(
                    fixture.floor_path, fixture.registry_path, fixture.file_sha256()
                )

    def test_entry_rewrite_is_rejected_as_removal(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.write_registry(registry([entry("alpha")]))
            fixture.initialize()
            fixture.write_registry(registry([entry("beta")]))
            with self.assertRaises(floor.FloorError):
                floor.advance_floor(
                    fixture.floor_path, fixture.registry_path, fixture.file_sha256()
                )

    def test_stale_compare_and_swap_digest_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.initialize()
            fixture.write_registry(registry([entry("alpha")]))
            with self.assertRaises(floor.FloorError):
                floor.advance_floor(
                    fixture.floor_path, fixture.registry_path, digest("stale-floor")
                )

    def test_same_generation_advance_is_idempotent(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            initial = fixture.initialize()
            current_sha = fixture.file_sha256()
            result, changed = floor.advance_floor(
                fixture.floor_path, fixture.registry_path, current_sha
            )
            self.assertFalse(changed)
            self.assertEqual(result, initial)
            self.assertEqual(fixture.file_sha256(), current_sha)

    def test_formatting_only_registry_change_is_an_explicit_generation(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            value = registry([entry("alpha")])
            fixture.write_registry(value, compact=False)
            initial = fixture.initialize()
            fixture.write_registry(value, compact=True)
            advanced, changed = floor.advance_floor(
                fixture.floor_path, fixture.registry_path, fixture.file_sha256()
            )
            self.assertTrue(changed)
            self.assertEqual(advanced["sequence"], 1)
            self.assertEqual(advanced["registry_digest"], initial["registry_digest"])
            self.assertNotEqual(
                advanced["registry_file_sha256"], initial["registry_file_sha256"]
            )

    def test_tampered_floor_digest_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.initialize()
            value = json.loads(fixture.floor_path.read_text(encoding="utf-8"))
            value["registry_digest"] = digest("tampered")
            fixture.floor_path.write_text(json.dumps(value) + "\n", encoding="utf-8")
            fixture.floor_path.chmod(0o600)
            with self.assertRaises(floor.FloorError):
                floor.verify_floor(fixture.floor_path)

    def test_duplicate_json_keys_are_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.floor_path.write_text(
                '{"schema":"dfmcp.live-compatibility-floor/1",'
                '"sequence":0,"sequence":1}\n',
                encoding="utf-8",
            )
            fixture.floor_path.chmod(0o600)
            with self.assertRaises((floor.FloorError, promotion.PromotionError)):
                floor.verify_floor(fixture.floor_path)

    def test_lock_rejects_concurrent_writer(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            with floor.floor_lock(fixture.floor_path):
                with self.assertRaises(floor.FloorError):
                    with floor.floor_lock(fixture.floor_path):
                        self.fail("second compatibility floor lock unexpectedly succeeded")


if __name__ == "__main__":
    unittest.main()
