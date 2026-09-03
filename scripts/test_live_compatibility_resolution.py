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


def entry(label: str = "base") -> dict[str, Any]:
    unsigned: dict[str, Any] = {
        "support_level": "experimental",
        "version_tuple": manifest()["version_tuple"],
        "platform": manifest()["platform"],
        "source": {
            **manifest()["source"],
            "dfmcp_dirty": False,
            "native_build_receipt_sha256": digest(f"native-receipt-{label}"),
            "live_acceptance_receipt_sha256": digest(f"live-receipt-file-{label}"),
            "live_acceptance_receipt_digest": digest(f"live-receipt-content-{label}"),
        },
        "gates": [
            {
                "gate": "R1",
                "status": "passed",
                "receipt_sha256": digest(f"native-receipt-{label}"),
            },
            *[
                {
                    "gate": gate,
                    "status": "passed",
                    "case_count": count,
                    "evidence_digest": digest(f"gate-{gate}-{label}"),
                }
                for gate, count in promotion.EXPECTED_LIVE_CASE_COUNTS.items()
            ],
        ],
        "capabilities": promotion.READ_ONLY_CAPABILITIES,
        "mutation_capabilities": [],
        "observed_domains": promotion.OBSERVED_DOMAINS,
        "conditional_domains": promotion.CONDITIONAL_DOMAINS,
        "omitted_domains": promotion.OMITTED_DOMAINS,
        "evidence_locator": f"qualification/{label}/live-read-acceptance-receipt.json",
        "limitations": promotion.LIMITATIONS,
    }
    return {"entry_id": promotion.sha256_bytes(promotion.canonical_json(unsigned)), **unsigned}


def different_entry(label: str) -> dict[str, Any]:
    value = copy.deepcopy(entry(label))
    value["version_tuple"]["dfhack"] = f"0.51.11-r1-{label}"
    value["source"]["dfmcp_commit"] = digest(f"commit-{label}")[:40]
    value["source"]["plugin_sha256"] = digest(f"plugin-{label}")
    unsigned = dict(value)
    del unsigned["entry_id"]
    value["entry_id"] = promotion.sha256_bytes(promotion.canonical_json(unsigned))
    return value


def revocation(item: dict[str, Any], label: str = "revoked") -> dict[str, Any]:
    return promotion.build_revocation(
        item["entry_id"],
        "evidence_invalidated",
        f"The evidence for {label} no longer supports runtime admission.",
        f"qualification/revocation/{label}.json",
        digest(f"revocation-{label}"),
    )


def registry(
    entries: list[dict[str, Any]],
    revocations: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    return promotion.build_registry(entries, [] if revocations is None else revocations)


class CompatibilityResolutionTests(unittest.TestCase):
    def test_exact_tuple_is_admitted_deterministically(self) -> None:
        value = registry([entry()])
        first = resolver.resolve(value, manifest())
        second = resolver.resolve(value, manifest())
        self.assertEqual(first, second)
        self.assertEqual(first["schema"], resolver.DECISION_SCHEMA)
        self.assertTrue(first["admitted"])
        self.assertEqual(first["entry_id"], entry()["entry_id"])
        self.assertEqual(first["support_level"], "experimental")
        self.assertEqual(first["mutation_capabilities"], [])
        self.assertEqual(first["matching_entry_ids"], [entry()["entry_id"]])
        self.assertEqual(first["matching_revocations"], [])
        self.assertEqual(first["registry_active_entry_count"], 1)
        self.assertEqual(first["registry_revocation_count"], 0)
        self.assertEqual(
            first["registry_digest"],
            promotion.sha256_bytes(promotion.canonical_json(value)),
        )
        self.assertEqual(
            first["registry_revocations_digest"],
            promotion.sha256_bytes(promotion.canonical_json([])),
        )
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
        self.assertEqual(decision["registry_status"], "no_admitted_live_tuples")
        self.assertEqual(decision["registry_active_entry_count"], 0)
        self.assertEqual(decision["registry_revocation_count"], 0)

    def test_revoked_exact_tuple_fails_closed_with_evidence(self) -> None:
        item = entry()
        revoked = revocation(item)
        decision = resolver.resolve(registry([item], [revoked]), manifest())
        self.assertFalse(decision["admitted"])
        self.assertIsNone(decision["entry_id"])
        self.assertEqual(decision["capabilities"], [])
        self.assertEqual(decision["matching_entry_ids"], [item["entry_id"]])
        self.assertEqual(decision["matching_revocations"], [revoked])
        self.assertEqual(decision["registry_active_entry_count"], 0)
        self.assertEqual(decision["registry_revocation_count"], 1)
        self.assertTrue(any("revoked" in reason for reason in decision["reasons"]))
        self.assertTrue(any("evidence_invalidated" in reason for reason in decision["reasons"]))

    def test_required_revoked_entry_cannot_fall_through_to_active_requalification(self) -> None:
        historical = entry("historical")
        replacement = entry("replacement")
        revoked = revocation(historical, "historical")
        value = registry([historical, replacement], [revoked])
        unfenced = resolver.resolve(value, manifest())
        self.assertTrue(unfenced["admitted"])
        self.assertEqual(unfenced["entry_id"], replacement["entry_id"])
        fenced = resolver.resolve(value, manifest(), historical["entry_id"])
        self.assertFalse(fenced["admitted"])
        self.assertIsNone(fenced["entry_id"])
        self.assertTrue(any("explicitly required" in reason for reason in fenced["reasons"]))

    def test_required_active_requalification_is_admitted(self) -> None:
        historical = entry("historical")
        replacement = entry("replacement")
        value = registry([historical, replacement], [revocation(historical)])
        decision = resolver.resolve(value, manifest(), replacement["entry_id"])
        self.assertTrue(decision["admitted"])
        self.assertEqual(decision["entry_id"], replacement["entry_id"])
        self.assertEqual(decision["registry_active_entry_count"], 1)

    def test_revocation_changes_decision_identity_for_unrelated_active_entry(self) -> None:
        target = entry()
        other = different_entry("other")
        without = resolver.resolve(registry([target, other]), manifest(), target["entry_id"])
        with_revocation = resolver.resolve(
            registry([target, other], [revocation(other, "other")]),
            manifest(),
            target["entry_id"],
        )
        self.assertTrue(without["admitted"])
        self.assertTrue(with_revocation["admitted"])
        self.assertEqual(without["entry_id"], with_revocation["entry_id"])
        self.assertNotEqual(
            without["registry_revocations_digest"],
            with_revocation["registry_revocations_digest"],
        )
        self.assertNotEqual(without["decision_digest"], with_revocation["decision_digest"])

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

    def test_required_entry_id_mismatch_fails_closed_and_is_bound(self) -> None:
        required = digest("other-entry")
        decision = resolver.resolve(registry([entry()]), manifest(), required)
        self.assertFalse(decision["admitted"])
        self.assertIsNone(decision["entry_id"])
        self.assertEqual(decision["required_entry_id"], required)

    def test_correct_entry_fence_changes_decision_identity(self) -> None:
        value = registry([entry()])
        unfenced = resolver.resolve(value, manifest())
        fenced = resolver.resolve(value, manifest(), entry()["entry_id"])
        self.assertTrue(fenced["admitted"])
        self.assertEqual(fenced["required_entry_id"], entry()["entry_id"])
        self.assertNotEqual(fenced["decision_digest"], unfenced["decision_digest"])

    def test_registry_generation_changes_decision_identity(self) -> None:
        first_registry = registry([entry(), different_entry("alpha")])
        second_registry = registry([entry(), different_entry("beta")])
        first = resolver.resolve(first_registry, manifest(), entry()["entry_id"])
        second = resolver.resolve(second_registry, manifest(), entry()["entry_id"])
        self.assertTrue(first["admitted"])
        self.assertTrue(second["admitted"])
        self.assertEqual(first["entry_id"], second["entry_id"])
        self.assertEqual(first["registry_historical_entry_count"], second["registry_historical_entry_count"])
        self.assertNotEqual(first["registry_digest"], second["registry_digest"])
        self.assertNotEqual(first["decision_digest"], second["decision_digest"])

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
            "revocations": [],
        }
        with self.assertRaises((promotion.PromotionError, resolver.promotion.PromotionError)):
            resolver.resolve(value, manifest())

    def test_tampered_revocation_is_rejected(self) -> None:
        item = entry()
        revoked = revocation(item)
        revoked["reason"] = "tampered after identity"
        value = {
            "schema_version": promotion.REGISTRY_SCHEMA,
            "status": "all_live_tuples_revoked",
            "entries": [item],
            "revocations": [revoked],
        }
        with self.assertRaises((promotion.PromotionError, resolver.promotion.PromotionError)):
            resolver.resolve(value, manifest())

    def test_decision_lists_do_not_alias_registry_lists(self) -> None:
        item = entry()
        revoked = revocation(item)
        value = registry([item], [revoked])
        original_reason = value["revocations"][0]["reason"]
        decision = resolver.resolve(value, manifest())
        decision["matching_entry_ids"].clear()
        decision["matching_revocations"][0]["reason"] = "mutated presentation"
        self.assertEqual(value["revocations"][0]["reason"], original_reason)
        self.assertNotEqual(value["revocations"][0]["reason"], "mutated presentation")


if __name__ == "__main__":
    unittest.main()
