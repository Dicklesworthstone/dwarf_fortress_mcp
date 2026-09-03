#!/usr/bin/env python3
"""Validate exact promotion plus evidence-bearing append-only revocation."""

from __future__ import annotations

import ast
import importlib.util
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REGISTRY_PATH = ROOT / "architecture/live_compatibility_registry_v1.json"
PROMOTION_PATH = ROOT / "scripts/promote_live_compatibility.py"
TEST_PATH = ROOT / "scripts/test_live_compatibility_registry.py"
DOC_PATH = ROOT / "docs/LIVE_COMPATIBILITY_ADMISSION.md"

SPEC = importlib.util.spec_from_file_location("promote_live_compatibility", PROMOTION_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load compatibility mutation module")
promotion = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = promotion
SPEC.loader.exec_module(promotion)


class ContractError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def functions(source: str) -> set[str]:
    tree = ast.parse(source)
    return {
        node.name
        for node in ast.walk(tree)
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
    }


def check_registry() -> None:
    registry = promotion.read_object(
        REGISTRY_PATH,
        promotion.MAX_JSON_BYTES,
        "checked-in compatibility registry",
    )
    entries, revocations = promotion.validate_registry_components(registry)
    require(registry.get("schema_version") == promotion.REGISTRY_SCHEMA, "registry schema drifted")
    require(registry.get("status") == "no_admitted_live_tuples", "checked-in registry unexpectedly admits or revokes a tuple")
    require(entries == [], "checked-in registry unexpectedly contains historical entries")
    require(revocations == [], "checked-in registry unexpectedly contains revocations")
    require(promotion.active_entries(registry) == [], "empty registry unexpectedly has active entries")
    require(
        promotion.revocations_digest(registry)
        == promotion.sha256_bytes(promotion.canonical_json([])),
        "empty revocation digest drifted",
    )


def check_implementation() -> None:
    source = PROMOTION_PATH.read_text(encoding="utf-8")
    names = functions(source)
    for name in [
        "validate_live_receipt",
        "validate_native_receipt",
        "compatibility_key",
        "validate_registry_components",
        "registry_revocations",
        "revoked_entry_ids",
        "active_entries",
        "revocations_digest",
        "build_registry",
        "build_entry",
        "build_revocation",
        "_promote",
        "promote",
        "_revoke",
        "revoke",
        "registry_lock",
        "write_atomic",
    ]:
        require(name in names, f"compatibility mutation implementation omits {name}")
    for marker in [
        'REGISTRY_SCHEMA = "dfmcp.live-compatibility-registry/2"',
        'REVOCATION_SCOPE = "runtime_admission"',
        '"compatibility_regression"',
        '"evidence_invalidated"',
        '"operational_withdrawal"',
        '"security_incident"',
        "without rewriting history",
        "already revoked",
        "cannot revoke an entry absent",
        "expected_registry_sha256",
        '"revocations"',
        '"partially_revoked_live_tuples"',
        '"all_live_tuples_revoked"',
        "--revoke-entry-id",
        "--reason-code",
        "--evidence-sha256",
    ]:
        require(marker in source, f"compatibility mutation implementation omits {marker}")
    for forbidden in ["subprocess", "requests", "urllib", "shell=True"]:
        require(forbidden not in source, f"compatibility mutation implementation contains forbidden dependency {forbidden}")


def check_tests() -> None:
    source = TEST_PATH.read_text(encoding="utf-8")
    require(source.count("def test_") >= 23, "compatibility registry needs at least twenty-three focused tests")
    for name in [
        "test_qualified_receipts_promote_deterministically",
        "test_expected_registry_digest_is_a_compare_and_swap_fence",
        "test_lock_prevents_concurrent_in_place_mutation",
        "test_revocation_is_deterministic_content_addressed_and_history_preserving",
        "test_absent_entry_and_duplicate_revocation_are_rejected",
        "test_revocation_requires_supported_reason_and_exact_evidence",
        "test_revocation_compare_and_swap_fence_is_enforced",
        "test_revocation_cannot_target_missing_history_or_be_reordered",
        "test_same_exact_tuple_can_be_requalified_only_after_revocation",
        "test_multiple_active_entries_for_same_tuple_are_rejected",
        "test_status_cannot_disagree_with_active_and_revoked_counts",
        "test_cli_modes_are_mutually_exclusive_and_complete",
    ]:
        require(f"def {name}" in source, f"compatibility tests omit {name}")


def check_docs() -> None:
    text = DOC_PATH.read_text(encoding="utf-8").casefold()
    for marker in [
        "evidence-bearing revocation",
        "future process",
        "historical entry",
        "does not terminate",
        "compare-and-swap",
    ]:
        require(marker.casefold() in text, f"compatibility documentation omits {marker}")


def main() -> int:
    try:
        check_registry()
        check_implementation()
        check_tests()
        check_docs()
    except (OSError, SyntaxError, json.JSONDecodeError, promotion.PromotionError, ContractError) as exc:
        print(f"live compatibility registry: FAIL: {exc}", file=sys.stderr)
        return 1
    print("live compatibility registry: PASS (exact admission plus append-only evidence-bearing revocation)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
