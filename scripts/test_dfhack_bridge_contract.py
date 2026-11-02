#!/usr/bin/env python3
"""Mutation checks for the V1 bridge's Boolean and source-custody gate."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import check_dfhack_bridge as bridge
import check_local_qualification_receipt as receipt


class BridgeContractTests(unittest.TestCase):
    def test_actual_boolean_writer_and_unrelated_uint32_conversion(self):
        source = (bridge.ROOT / "crates/dfmcp-adapter/src/dfhack_wire.rs").read_text()
        self.assertIn("u64::from(value)", source)
        self.assertTrue(bridge.canonical_boolean_writer(source))
        for changed in [
            source.replace("if value { 1 } else { 0 }", "if value { 0 } else { 1 }", 1),
            source.replace("self.varint(if value { 1 } else { 0 });", "self.varint(u64::from(value));", 1),
            source.replace("self.key(field, WireType::Varint);\n        self.varint(if value", "self.key(field, WireType::LengthDelimited);\n        self.varint(if value", 1),
        ]:
            self.assertNotEqual(changed, source)
            self.assertFalse(bridge.canonical_boolean_writer(changed))
        self.assertFalse(bridge.canonical_boolean_writer(
            '// fn boolean(&mut self, field: u32, value: bool) { self.key(field, WireType::Varint); self.varint(if value { 1 } else { 0 }); }'
        ))

    def test_real_two_phase_binding_and_inventory_mutations(self):
        failures = []
        bridge.check_qualification_wiring(failures)
        self.assertEqual(failures, [])
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for attribute, mutation in [
                ("CONTRACT", lambda text: text.replace('"tracked_inventory_must_remain_exact": true', '"tracked_inventory_must_remain_exact": false')),
                ("WRITER", lambda text: text.replace('"ls-tree", "-rz", "--full-tree"', '"ls-files"')),
                ("WRITER", lambda text: text.replace('"source snapshot changed during local qualification"', '"removed"')),
                ("WRAPPER", lambda text: text.replace('python3 "$RECEIPT_WRITER" begin', 'true')),
                ("WRAPPER", lambda text: text.replace('python3 "$RECEIPT_WRITER" finish', 'true')),
            ]:
                original = getattr(receipt, attribute).read_text()
                mutant = mutation(original)
                self.assertNotEqual(mutant, original)
                path = root / (attribute + ".mutant")
                path.write_text(mutant)
                with patch.object(receipt, attribute, path):
                    failures = []
                    bridge.check_qualification_wiring(failures)
                self.assertTrue(any("source binding is invalid" in f.message for f in failures), attribute)

    def test_missing_head_inventory_and_missing_file_fail(self):
        class Listing:
            stdout = ("\0".join(bridge.RECEIPT_BRIDGE_PATHS[:-1]) + "\0").encode()
        with patch.object(bridge.subprocess, "run", return_value=Listing()):
            failures = []
            bridge.check_qualification_wiring(failures)
        self.assertTrue(any(f.path == bridge.RECEIPT_BRIDGE_PATHS[-1] and "HEAD inventory" in f.message for f in failures))
        with patch.object(bridge, "RECEIPT_BRIDGE_PATHS", ["missing/bridge-source"]), \
             patch.object(bridge.subprocess, "run", return_value=Listing()):
            failures = []
            bridge.check_qualification_wiring(failures)
        self.assertTrue(any("required bridge source is absent" == f.message for f in failures))

    def test_inventory_dataflow_rejects_semantic_removal_with_messages_retained(self):
        original = receipt.WRITER.read_text()
        self.assertTrue(bridge.receipt_inventory_dataflow(original))
        for old, new in [
            ("listing.split(b\"\\0\")", "[]"),
            ("if current != snapshot:", "if False:"),
            ("if current_before_publish != snapshot:", "if False:"),
            ("if current_after_publish != snapshot:", "if False:"),
            ('{entry["path"]: entry["sha256"] for entry in snapshot["entries"]}', "{}"),
        ]:
            mutant = original.replace(old, new)
            self.assertNotEqual(mutant, original)
            self.assertFalse(bridge.receipt_inventory_dataflow(mutant), old)


if __name__ == "__main__":
    unittest.main()
