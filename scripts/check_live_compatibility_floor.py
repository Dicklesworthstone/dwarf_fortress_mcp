#!/usr/bin/env python3
"""Validate the owner-private monotonic compatibility-floor boundary."""

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


def fail(message: str) -> None:
    raise ContractError(message)


def require(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


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
        contract.get("schema_version")
        == "dfmcp.live-compatibility-floor-contract/1",
        "compatibility floor contract schema drifted",
    )
    require(
        contract.get("floor_schema") == "dfmcp.live-compatibility-floor/1",
        "compatibility floor data schema drifted",
    )
    custody = contract.get("custody", {})
    require(custody.get("floor_path_must_be_absolute") is True, "absolute floor path is not required")
    require(custody.get("parent_directory_mode") == "0700", "floor parent mode drifted")
    require(custody.get("file_mode") == "0600", "floor file mode drifted")
    require(custody.get("symbolic_links_allowed") is False, "floor contract permits symbolic links")
    generation = contract.get("generation", {})
    require(generation.get("advance_policy") == "strict_compare_and_swap", "floor CAS policy drifted")
    require(generation.get("idempotent_same_generation") is True, "floor idempotency drifted")
    require(contract.get("authority", {}).get("grants_capabilities") == [], "floor contract grants capability")
    require(contract.get("authority", {}).get("mutation_capabilities") == [], "floor contract grants mutation")


def check_implementation() -> None:
    source = IMPLEMENTATION_PATH.read_text(encoding="utf-8")
    names = function_names(source)
    for name in [
        "validate_private_directory",
        "read_private_bytes",
        "validate_floor",
        "read_floor",
        "registry_generation",
        "build_floor",
        "verify_floor",
        "write_private_exclusive",
        "write_private_atomic",
        "floor_lock",
        "initialize_floor",
        "advance_floor",
    ]:
        require(name in names, f"compatibility floor implementation is missing {name}")
    for marker in [
        "O_NOFOLLOW",
        "0o700",
        "0o600",
        "expected_floor_file_sha256",
        "previous_floor_digest",
        "rolls back prior admitted entry IDs",
        "strict canonical order",
        "compatibility registry generation does not match the trusted monotonic floor",
        "--expected-floor-sha256",
    ]:
        require(marker in source, f"compatibility floor implementation is missing marker {marker}")
    for forbidden in ["subprocess", "requests", "urllib", "shell=True"]:
        require(forbidden not in source, f"compatibility floor implementation contains forbidden dependency {forbidden}")


def check_tests_and_docs() -> None:
    tests = TEST_PATH.read_text(encoding="utf-8")
    require(tests.count("def test_") >= 14, "compatibility floor needs at least fourteen focused tests")
    for name in [
        "test_initialize_and_verify_exact_generation",
        "test_relative_floor_path_is_rejected",
        "test_permissive_parent_directory_is_rejected",
        "test_symbolic_link_floor_is_rejected",
        "test_advance_appends_entries_and_chains_floor_digest",
        "test_registry_rollback_is_rejected",
        "test_stale_compare_and_swap_digest_is_rejected",
        "test_same_generation_advance_is_idempotent",
        "test_formatting_only_registry_change_is_an_explicit_generation",
        "test_tampered_floor_digest_is_rejected",
        "test_duplicate_json_keys_are_rejected",
        "test_lock_rejects_concurrent_writer",
    ]:
        require(f"def {name}" in tests, f"compatibility floor tests omit {name}")
    documentation = DOC_PATH.read_text(encoding="utf-8")
    for marker in [
        "monotonic floor",
        "anti-rollback",
        "owner-only",
        "compare-and-swap",
        "does not admit",
    ]:
        require(marker.lower() in documentation.lower(), f"compatibility admission documentation omits {marker}")


def main() -> int:
    try:
        check_contract()
        check_implementation()
        check_tests_and_docs()
    except (OSError, SyntaxError, json.JSONDecodeError, ContractError) as exc:
        print(f"live compatibility floor: FAIL: {exc}", file=sys.stderr)
        return 1
    print("live compatibility floor: PASS (owner-private monotonic anti-rollback custody)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
