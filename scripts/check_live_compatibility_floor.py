#!/usr/bin/env python3
"""Validate monotonic admission and revocation custody."""

from __future__ import annotations

import ast
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACT_PATH = ROOT / "architecture/live_compatibility_floor_v1.json"
IMPLEMENTATION_PATH = ROOT / "scripts/live_compatibility_floor.py"
TEST_PATH = ROOT / "scripts/test_live_compatibility_floor.py"
DOC_PATH = ROOT / "docs/LIVE_COMPATIBILITY_ADMISSION.md"


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


def check_contract() -> None:
    contract = json.loads(CONTRACT_PATH.read_text(encoding="utf-8"))
    require(
        contract.get("schema_version") == "dfmcp.live-compatibility-floor-contract/2",
        "compatibility floor contract schema drifted",
    )
    require(contract.get("floor_schema") == "dfmcp.live-compatibility-floor/2", "floor data schema drifted")
    require(contract.get("legacy_floor_schema") == "dfmcp.live-compatibility-floor/1", "legacy floor schema drifted")
    custody = contract.get("custody", {})
    require(custody.get("floor_path_must_be_absolute") is True, "absolute floor path is not required")
    require(custody.get("parent_directory_mode") == "0700", "floor parent mode drifted")
    require(custody.get("file_mode") == "0600", "floor file mode drifted")
    require(custody.get("symbolic_links_allowed") is False, "floor contract permits symbolic links")
    generation = contract.get("generation", {})
    require(generation.get("registry_schema") == "dfmcp.live-compatibility-registry/2", "floor registry schema drifted")
    require(generation.get("advance_policy") == "strict_compare_and_swap", "floor CAS policy drifted")
    require(generation.get("idempotent_same_generation") is True, "floor idempotency drifted")
    require("revocation_id" in generation.get("revocation_policy", ""), "floor revocation monotonicity is unspecified")
    require("partition" in generation.get("partition_policy", ""), "floor active/revoked partition is unspecified")
    require(contract.get("authority", {}).get("grants_capabilities") == [], "floor grants capability")
    require(contract.get("authority", {}).get("mutation_capabilities") == [], "floor grants mutation")


def check_implementation() -> None:
    source = IMPLEMENTATION_PATH.read_text(encoding="utf-8")
    names = function_names(source)
    for name in [
        "validate_private_directory",
        "read_private_bytes",
        "validate_floor",
        "read_floor",
        "registry_generation_from_value",
        "registry_generation",
        "build_floor",
        "verify_generation",
        "verify_floor",
        "write_private_exclusive",
        "write_private_atomic",
        "floor_lock",
        "initialize_floor",
        "advance_floor",
    ]:
        require(name in names, f"compatibility floor implementation omits {name}")
    for marker in [
        'FLOOR_SCHEMA = "dfmcp.live-compatibility-floor/2"',
        'LEGACY_FLOOR_SCHEMA = "dfmcp.live-compatibility-floor/1"',
        "revocation_ids",
        "revoked_entry_ids",
        "active_entry_ids",
        "rolls back prior historical entry IDs",
        "rolls back prior revocation IDs",
        "active and revoked entries do not partition history",
        "expected_floor_file_sha256",
        "strict canonical order",
        "--expected-floor-sha256",
    ]:
        require(marker in source, f"compatibility floor implementation omits {marker}")
    for forbidden in ["subprocess", "requests", "urllib", "shell=True"]:
        require(forbidden not in source, f"compatibility floor contains forbidden dependency {forbidden}")


def check_tests_and_docs() -> None:
    tests = TEST_PATH.read_text(encoding="utf-8")
    require(tests.count("def test_") >= 10, "compatibility floor needs at least ten focused tests")
    for name in [
        "test_initialize_binds_empty_active_and_revocation_sets",
        "test_relative_permissive_and_symbolic_custody_fail_closed",
        "test_entry_and_revocation_advance_preserve_history",
        "test_historical_entry_and_revocation_rollback_are_rejected",
        "test_compare_and_swap_and_same_generation_semantics",
        "test_formatting_only_registry_change_is_explicit_generation",
        "test_legacy_floor_requires_explicit_v2_migration",
        "test_legacy_floor_cannot_verify_a_revocation_generation",
        "test_active_revoked_partition_and_digest_tampering_are_rejected",
        "test_duplicate_keys_and_concurrent_writers_are_rejected",
    ]:
        require(f"def {name}" in tests, f"compatibility floor tests omit {name}")
    docs = DOC_PATH.read_text(encoding="utf-8").casefold()
    for marker in [
        "monotonic floor",
        "revocation id",
        "revoked entry",
        "legacy floor",
        "compare-and-swap",
        "does not terminate",
    ]:
        require(marker in docs, f"compatibility floor documentation omits {marker}")


def main() -> int:
    try:
        check_contract()
        check_implementation()
        check_tests_and_docs()
    except (OSError, SyntaxError, json.JSONDecodeError, ContractError) as exc:
        print(f"live compatibility floor: FAIL: {exc}", file=sys.stderr)
        return 1
    print("live compatibility floor: PASS (owner-private monotonic admission and revocation custody)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
