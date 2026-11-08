#!/usr/bin/env python3
"""Required workshop component inputs cannot silently disappear from evidence."""
from pathlib import Path
import os
import json
import shutil
import tempfile
import unittest

from test_workshop_native import INPUTS, source_inventory
from check_workshop_construction import ROOT, SOURCES, check


class WorkshopSourceInventoryTests(unittest.TestCase):
    def sources(self, root: Path) -> None:
        for name in INPUTS:
            p = root / name
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text("bounded source\n", encoding="utf-8")

    def test_required_inventory_contains_every_declared_input(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            self.sources(root)
            self.assertEqual(set(source_inventory(root)), set(INPUTS))

    def test_missing_required_proto_and_cmake_are_refused(self) -> None:
        for name in INPUTS:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                self.sources(root)
                (root / name).unlink()
                with self.assertRaises(ValueError):
                    source_inventory(root)

    def test_symlink_special_oversized_and_non_utf8_are_refused(self) -> None:
        for kind in ("link", "fifo", "large", "utf8", "nul"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                self.sources(root)
                p = root / INPUTS[0]
                p.unlink()
                if kind == "link":
                    p.symlink_to(root / INPUTS[1])
                elif kind == "fifo":
                    os.mkfifo(p)
                elif kind == "large":
                    with p.open("wb") as output:
                        output.truncate(4 * 1024 * 1024 + 1)
                else:
                    p.write_bytes(b"\xff" if kind == "utf8" else b"nul\0source")
                with self.assertRaises((ValueError, UnicodeError)):
                    source_inventory(root)

    def test_source_mutation_changes_complete_inventory(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            self.sources(root)
            before = source_inventory(root)
            (root / INPUTS[-1]).write_text("changed source\n", encoding="utf-8")
            self.assertNotEqual(before, source_inventory(root))

    def copy_consumer(self, root: Path) -> None:
        for name in SOURCES:
            target = root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)

    def test_registered_consumer_contract_and_source_inventory_agree(self) -> None:
        result = check(ROOT)
        self.assertEqual(set(result["source_sha256"]), set(SOURCES))
        self.assertEqual(result["status"], "passed_static_only")
        self.assertFalse(result["rust_executed"])
        self.assertFalse(result["sdk_or_game_executed"])

    def test_missing_consumer_source_and_unregistered_module_are_refused(self) -> None:
        for kind in ("missing", "origin", "origin_tests", "unregistered"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                self.copy_consumer(root)
                if kind == "missing":
                    (root / "crates/dfmcp-adapter/src/workshop_construction/rpc_tests.rs").unlink()
                elif kind in ("origin", "origin_tests"):
                    name = "production_origin.rs" if kind == "origin" else "production_origin_tests.rs"
                    (root / "crates/dfmcp-adapter/src/workshop_construction" / name).unlink()
                else:
                    path = root / "crates/dfmcp-adapter/src/lib.rs"
                    text = path.read_text()
                    self.assertEqual(text.count("pub mod workshop_construction;"), 1)
                    path.write_text(text.replace("pub mod workshop_construction;", ""))
                with self.assertRaises(ValueError):
                    check(root)

    def test_missing_dispatch_acknowledgement_and_wrong_wire_tool_are_refused(self) -> None:
        mutations = (
            ("crates/dfmcp-adapter/src/workshop_construction/journal.rs",
             "guard.acknowledge(self.pin(), &work.current()?)?;", ""),
            ("crates/dfmcp-mcp/src/workshop_construction_server/presentation.rs",
             "\"fortress_commit\"", "\"fortress.commit\""),
        )
        for name, before, after in mutations:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                self.copy_consumer(root)
                path = root / name
                text = path.read_text()
                self.assertGreater(text.count(before), 0)
                path.write_text(text.replace(before, after))
                with self.assertRaises(ValueError):
                    check(root)

    def test_tree_material_import_cannot_grant_writes_or_recovery_discovery(self) -> None:
        mutations = (
            ("tree_material_adapter", "donor_writes", True),
            ("tree_material_adapter", "causal_provenance_known", True),
            ("tree_material_consumer", "recovery_tree_native_calls", True),
            ("tree_material_consumer", "maximum_offers", 65),
        )
        for section, field, value in mutations:
            with self.subTest(field=field), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                self.copy_consumer(root)
                path = root / "architecture/workshop_construction_v1_20.json"
                contract = json.loads(path.read_text())
                contract[section][field] = value
                path.write_text(json.dumps(contract))
                with self.assertRaises(ValueError):
                    check(root)

    def test_production_origin_contract_preserves_readonly_custody_and_freshness(self) -> None:
        mutations = (
            ("donor_writes", True),
            ("fresh_complete_required", False),
            ("canonical_world_fact", True),
            ("owner_cloneable", True),
            ("recovery_floor_custody", "acknowledged by donor"),
            ("decoded_manifest_consumed_before_refresh_preconditions", False),
        )
        for field, value in mutations:
            with self.subTest(field=field), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                self.copy_consumer(root)
                path = root / "architecture/workshop_construction_v1_20.json"
                contract = json.loads(path.read_text())
                contract["production_origin_adapter"][field] = value
                path.write_text(json.dumps(contract))
                with self.assertRaises(ValueError):
                    check(root)


if __name__ == "__main__":
    unittest.main()
