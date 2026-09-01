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

MODULE_PATH = Path(__file__).with_name("issue_dfhack_plugin_receipt_v1_1.py")
SPEC = importlib.util.spec_from_file_location(
    "issue_dfhack_plugin_receipt_v1_1", MODULE_PATH
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load protocol-1.1 native receipt tool")
receipt_tool = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = receipt_tool
SPEC.loader.exec_module(receipt_tool)


def digest(label: str) -> str:
    return hashlib.sha256(label.encode()).hexdigest()


class Fixture:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.contract = receipt_tool.load_contract(receipt_tool.DEFAULT_CONTRACT)
        self.generation_digests = receipt_tool.source_digests(
            self.contract, receipt_tool.ROOT
        )
        self.base_path = root / "base-native.json"
        self.receipt_path = root / "generation-native.json"
        self.base = self.base_receipt()
        self.write_base(self.base)

    def base_receipt(self) -> dict[str, Any]:
        return {
            "schema": receipt_tool.promotion.NATIVE_RECEIPT_SCHEMA,
            "status": "native-build-passed",
            "source": {
                "dfmcp_commit": "1" * 40,
                "dfmcp_dirty": False,
                "dfhack_commit": "2" * 40,
            },
            "plugin": {
                "sha256": digest("plugin-v1.1"),
                "rpc_methods": ["Handshake", "ReadObservation"],
                "mutation_rpc_methods": [],
                "strings_inventory": "passed",
                "symbols_inventory": "passed",
            },
            "source_digests": {
                "proto": self.generation_digests["protocol_1_1_proto"],
                "native": self.generation_digests["protocol_1_1_native"],
                "qualifier": self.generation_digests["protocol_1_1_qualifier"],
            },
        }

    def write_base(self, value: dict[str, Any]) -> None:
        self.base_path.write_text(
            json.dumps(value, sort_keys=True) + "\n", encoding="utf-8"
        )

    def issue(self) -> dict[str, Any]:
        return receipt_tool.issue(
            self.base_path,
            receipt_tool.ROOT,
            receipt_tool.DEFAULT_CONTRACT,
        )


class ProtocolGenerationNativeReceiptTests(unittest.TestCase):
    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Fixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, Fixture(Path(temporary.name))

    def test_exact_v1_1_base_receipt_issues_deterministically(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            first = fixture.issue()
            second = fixture.issue()
            self.assertEqual(first, second)
            self.assertEqual(first["status"], "qualified")
            self.assertEqual(first["bridge"]["plugin"], "dfmcp_bridge_v1_1")
            self.assertEqual(first["bridge"]["protocol"], "1.1")
            self.assertEqual(first["mutation_capabilities"], [])
            self.assertEqual(
                receipt_tool.validate_receipt(first, fixture.contract), first
            )

    def test_v1_0_source_binding_cannot_be_relabelled_as_v1_1(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            wrong = copy.deepcopy(fixture.base)
            wrong["source_digests"] = {
                "proto": fixture.generation_digests["protocol_1_0_proto"],
                "native": fixture.generation_digests["protocol_1_0_native"],
                "qualifier": digest("legacy-qualifier"),
            }
            fixture.write_base(wrong)
            with self.assertRaises(receipt_tool.ReceiptError):
                fixture.issue()

    def test_mutation_method_or_skipped_inventory_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            mutation = copy.deepcopy(fixture.base)
            mutation["plugin"]["mutation_rpc_methods"] = ["Pause"]
            fixture.write_base(mutation)
            with self.assertRaises(receipt_tool.promotion.PromotionError):
                fixture.issue()

            skipped = copy.deepcopy(fixture.base)
            skipped["plugin"]["symbols_inventory"] = "skipped"
            fixture.write_base(skipped)
            with self.assertRaises(receipt_tool.promotion.PromotionError):
                fixture.issue()

    def test_embedded_base_receipt_tampering_is_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            value = fixture.issue()
            value["base_receipt"]["receipt"]["plugin"]["sha256"] = digest(
                "tampered-plugin"
            )
            unsigned = dict(value)
            unsigned.pop("receipt_digest", None)
            value["receipt_digest"] = receipt_tool.sha256_bytes(
                receipt_tool.canonical_json(unsigned)
            )
            with self.assertRaises(receipt_tool.ReceiptError):
                receipt_tool.validate_receipt(value, fixture.contract)

    def test_bridge_identity_and_receipt_digest_tampering_are_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            value = fixture.issue()
            value["bridge"]["protocol"] = "1.0"
            unsigned = dict(value)
            unsigned.pop("receipt_digest", None)
            value["receipt_digest"] = receipt_tool.sha256_bytes(
                receipt_tool.canonical_json(unsigned)
            )
            with self.assertRaises(receipt_tool.ReceiptError):
                receipt_tool.validate_receipt(value, fixture.contract)

            value = fixture.issue()
            value["receipt_digest"] = digest("wrong-receipt")
            with self.assertRaises(receipt_tool.ReceiptError):
                receipt_tool.validate_receipt(value, fixture.contract)

    def test_generation_source_digest_key_order_is_exact(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            value = fixture.issue()
            reversed_items = list(value["source_digests"].items())[::-1]
            value["source_digests"] = dict(reversed_items)
            unsigned = dict(value)
            unsigned.pop("receipt_digest", None)
            value["receipt_digest"] = receipt_tool.sha256_bytes(
                receipt_tool.canonical_json(unsigned)
            )
            with self.assertRaises(receipt_tool.ReceiptError):
                receipt_tool.validate_receipt(value, fixture.contract)

    def test_cli_issue_and_validate_round_trip(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            self.assertEqual(
                receipt_tool.main(
                    [
                        str(fixture.base_path),
                        "--source-root",
                        str(receipt_tool.ROOT),
                        "--output",
                        str(fixture.receipt_path),
                    ]
                ),
                0,
            )
            self.assertTrue(fixture.receipt_path.is_file())
            self.assertEqual(
                receipt_tool.main([str(fixture.receipt_path), "--validate"]), 0
            )


if __name__ == "__main__":
    unittest.main()
