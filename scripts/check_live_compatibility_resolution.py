#!/usr/bin/env python3
"""Validate revocation-aware exact compatibility resolution."""

from __future__ import annotations

import ast
import importlib.util
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REGISTRY_PATH = ROOT / "architecture/live_compatibility_registry_v1.json"
RESOLVER_PATH = ROOT / "scripts/resolve_live_compatibility.py"
TEST_PATH = ROOT / "scripts/test_live_compatibility_resolution.py"
DOC_PATH = ROOT / "docs/LIVE_COMPATIBILITY_ADMISSION.md"

SPEC = importlib.util.spec_from_file_location("resolve_live_compatibility", RESOLVER_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load compatibility resolver")
resolver = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = resolver
SPEC.loader.exec_module(resolver)
promotion = resolver.promotion


class ContractError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def function_names(source: str) -> set[str]:
    tree = ast.parse(source)
    return {
        node.name
        for node in ast.walk(tree)
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
    }


def check_implementation() -> None:
    source = RESOLVER_PATH.read_text(encoding="utf-8")
    names = function_names(source)
    for name in ["validate_manifest", "deployment_key", "classify_miss", "revocation_reason", "resolve"]:
        require(name in names, f"compatibility resolver omits {name}")
    for marker in [
        'DECISION_SCHEMA = "dfmcp.live-compatibility-decision/2"',
        "validate_registry_components",
        "matching_entry_ids",
        "matching_revocations",
        "registry_historical_entry_count",
        "registry_active_entry_count",
        "registry_revocation_count",
        "registry_revocations_digest",
        "explicitly required exact compatibility entry is revoked",
        "copy.deepcopy",
        "decision_digest",
    ]:
        require(marker in source, f"compatibility resolver omits {marker}")


def check_empty_registry() -> None:
    registry = promotion.read_object(
        REGISTRY_PATH,
        promotion.MAX_JSON_BYTES,
        "checked-in compatibility registry",
    )
    decision = resolver.resolve(
        registry,
        {
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
                "plugin_sha256": "3" * 64,
            },
        },
        "4" * 64,
    )
    require(decision.get("admitted") is False, "empty registry admitted a deployment")
    require(decision.get("registry_historical_entry_count") == 0, "empty historical count drifted")
    require(decision.get("registry_active_entry_count") == 0, "empty active count drifted")
    require(decision.get("registry_revocation_count") == 0, "empty revocation count drifted")
    require(decision.get("matching_entry_ids") == [], "empty registry produced a historical match")
    require(decision.get("matching_revocations") == [], "empty registry produced a revocation match")


def check_tests_and_docs() -> None:
    tests = TEST_PATH.read_text(encoding="utf-8")
    require(tests.count("def test_") >= 17, "compatibility resolver needs at least seventeen focused tests")
    for name in [
        "test_exact_tuple_is_admitted_deterministically",
        "test_required_entry_id_mismatch_fails_closed_and_is_bound",
        "test_registry_generation_changes_decision_identity",
        "test_revoked_exact_tuple_fails_closed_with_evidence",
        "test_required_revoked_entry_cannot_fall_through_to_active_requalification",
        "test_revocation_changes_decision_identity_for_unrelated_active_entry",
        "test_decision_lists_do_not_alias_registry_lists",
    ]:
        require(f"def {name}" in tests, f"compatibility resolver tests omit {name}")
    docs = DOC_PATH.read_text(encoding="utf-8").casefold()
    for marker in ["revoked entry", "matching revocations", "required entry", "fail closed"]:
        require(marker in docs, f"compatibility resolution documentation omits {marker}")


def main() -> int:
    try:
        check_implementation()
        check_empty_registry()
        check_tests_and_docs()
    except (OSError, SyntaxError, json.JSONDecodeError, promotion.PromotionError, resolver.ResolutionError, ContractError) as exc:
        print(f"live compatibility resolution: FAIL: {exc}", file=sys.stderr)
        return 1
    print("live compatibility resolution: PASS (exact, generation-bound, and revocation-aware)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
