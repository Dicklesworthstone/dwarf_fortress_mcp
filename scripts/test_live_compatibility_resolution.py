#!/usr/bin/env python3

from __future__ import annotations

import copy
import hashlib
import importlib.util
import sys
import unittest
from pathlib import Path
from typing import Any

PROMOTION_PATH = Path(__file__).with_name("promote_live_compatibility.py")
PROMOTION_SPEC = importlib.util.spec_from_file_location("promote_live_compatibility", PROMOTION_PATH)
if PROMOTION_SPEC is None or PROMOTION_SPEC.loader is None:
    raise RuntimeError("cannot load compatibility promotion module")
promotion = importlib.util.module_from_spec(PROMOTION_SPEC)
sys.modules[PROMOTION_SPEC.name] = promotion
PROMOTION_SPEC.loader.exec_module(promotion)

RESOLVER_PATH = Path(__file__).with_name("resolve_live_compatibility.py")
RESOLVER_SPEC = importlib.util.spec_from_file_location("resolve_live_compatibility", RESOLVER_PATH)
if RESOLVER_SPEC is None or RESOLVER_SPEC.loader is None:
    raise RuntimeError("cannot load compatibility resolver module")
resolver = importlib.util.module_from_spec(RESOLVER_SPEC)
sys.modules[RESOLVER_SPEC.name] = resolver
RESOLVER_SPEC.loader.exec_module(resolver)


def digest(label: str) -> str:
    return hashlib.sha256(label.encode()).hexdigest()


def manifest() -> dict[str, Any]:
    return {
        "schema": resolver.MANIFEST_SCHEMA,
        "version_tuple": {
            "dwarf_fortress": "0.51.11",
            "dfhack": "0.51.11-r1",
            "bridge": "0.1.0",
            "protocol": "1.0",
        },
        "platform": {"system": "Linux", "machine": "x86_64"},
        "source": {
            "dfmcp_commit": "1" * 40,
            "dfhack_commit": "2" * 40,
            "plugin_sha256": digest("plugin"),
        },
    }


def entry() -> dict[str, Any]:
    unsigned: dict[str, Any] = {
        "support_level": "experimental",
        "version_tuple": manifest()["version_tuple"],
        "platform": manifest()["platform"],
        "source": {
            **manifest()["source"],
            "dfmcp_dirty": False,
            "native_build_receipt_sha256": digest("native-receipt"),
            "live_acceptance_receipt_sha256": digest("live-receipt-file"),
            "live_acceptance_receipt_digest": digest("live-receipt-content"),
        },
        "gates": [
            {
                "gate": "R1",
                "status": "passed",
                "receipt_sha256": digest("native-receipt"),
            },
            *[
                {
                    "gate": gate,
                    "status": "passed",
                    "case_count": count,
                    "evidence_digest": digest(f"gate-{gate}"),
                }
                for gate, count in [("R2", 12), ("R3", 14), ("R4", 7), ("R5", 1)]
            ],
        ],
        "capabilities": promotion.READ_ONLY_CAPABILITIES,
        "mutation_capabilities": [],
        "observed_domains": promotion.OBSERVED_DOMAINS,
        "conditional_domains": promotion.CONDITIONAL_DOMAINS,
        "omitted_domains": promotion.OMITTED_DOMAINS,
        "evidence_locator": "qualification/fixture/live-read-acceptance-receipt.json",
        "limitations": [
            "admission applies only to this exact source, binary, version, and platform tuple",
            "host compromise is outside the loopback bearer threat model",
            "no live mutation method is admitted",
            "durable production custody and release support are not established by R1-R5",
        ],
    }
    return {"entry_id": promotion.sha256_bytes(promotion.canonical_json(unsigned)), **unsigned}


def registry(entries: list[dict[str, Any]]) -> dict[str, Any]:
    ordered = sorted(entries, key=lambda item: item["entry_id"])
    return {
        "schema_version": promotion.REGISTRY_SCHEMA,
        "status": "admitted_live_tuples" if ordered else "no_admitted_live_tuples",
        "entries": ordered,
    }


class CompatibilityResolutionTests(unittest.TestCase):
    def test_exact_tuple_is_admitted_deterministically(self) -> None:
        value = registry([entry()])
        first = resolver.resolve(value, manifest())
        second = resolver.resolve(value, manifest())
        self.assertEqual(first, second)
        self.assertTrue(first["admitted"])
        self.assertEqual(first["entry_id"], entry()["entry_id"])
        self.assertEqual(first["support_level"], "experimental")
        self.assertEqual(first["mutation_capabilities"], [])
        unsigned = dict(first)
        del unsigned["decision_digest"]
        self.assertEqual(
            first["decision_digest"],
            promotion.sha256_bytes(promotion.canonical_json(unsigned)),
        )

    def test_empty_registry_fails_closed(self) -> None:
        decision = resolver.resolve(registry([]), manifest())
        self.assertFalse(decision["admitted"])
        self.assertIsNone(decision["entry_id"])
        self.assertEqual(decision["capabilities"], [])
        self.assertTrue(decision["reasons"])

    def test_same_versions_with_different_binary_are_not_admitted(self) -> None:
        deployment = manifest()
        deployment["source"]["plugin_sha256"] = digest("different-plugin")
        decision = resolver.resolve(registry([entry()]), deployment)
        self.assertFalse(decision["admitted"])
        self.assertTrue(any("binary digest" in reason for reason in decision["reasons"]))

    def test_platform_drift_is_not_admitted(self) -> None:
        deployment = manifest()
        deployment["platform"]["machine"] = "aarch64"
        decision = resolver.resolve(registry([entry()]), deployment)
        self.assertFalse(decision["admitted"])
        self.assertTrue(any("architecture" in reason for reason in decision["reasons"]))

    def test_source_revision_drift_is_not_admitted(self) -> None:
        deployment = manifest()
        deployment["source"]["dfmcp_commit"] = "3" * 40
        decision = resolver.resolve(registry([entry()]), deployment)
        self.assertFalse(decision["admitted"])
        self.assertTrue(any("source revision" in reason for reason in decision["reasons"]))

    def test_required_entry_id_mismatch_fails_closed(self) -> None:
        decision = resolver.resolve(registry([entry()]), manifest(), digest("other-entry"))
        self.assertFalse(decision["admitted"])
        self.assertIsNone(decision["entry_id"])

    def test_manifest_extra_field_is_rejected(self) -> None:
        deployment = manifest()
        deployment["token"] = "must-not-be-accepted"
        with self.assertRaises(resolver.ResolutionError):
            resolver.resolve(registry([entry()]), deployment)

    def test_protocol_drift_is_rejected_before_lookup(self) -> None:
        deployment = manifest()
        deployment["version_tuple"]["protocol"] = "2.0"
        with self.assertRaises(resolver.ResolutionError):
            resolver.resolve(registry([entry()]), deployment)

    def test_tampered_registry_entry_is_rejected(self) -> None:
        damaged = copy.deepcopy(entry())
        damaged["capabilities"] = ["observe", "pause"]
        value = {
            "schema_version": promotion.REGISTRY_SCHEMA,
            "status": "admitted_live_tuples",
            "entries": [damaged],
        }
        with self.assertRaises(promotion.PromotionError):
            resolver.resolve(value, manifest())


if __name__ == "__main__":
    unittest.main()
