#!/usr/bin/env python3
"""Validate the exact live compatibility registry and its promotion boundary."""

from __future__ import annotations

import ast
import importlib.util
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REGISTRY_PATH = ROOT / "architecture/live_compatibility_registry_v1.json"
PROMOTION_PATH = ROOT / "scripts/promote_live_compatibility.py"
TEST_PATH = ROOT / "scripts/test_live_compatibility_registry.py"
DOC_PATH = ROOT / "docs/LIVE_COMPATIBILITY_ADMISSION.md"

SPEC = importlib.util.spec_from_file_location("promote_live_compatibility", PROMOTION_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load live compatibility promotion module")
promotion = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = promotion
SPEC.loader.exec_module(promotion)


class RegistryError(ValueError):
    pass


def fail(message: str) -> None:
    raise RegistryError(message)


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


def check_source_contracts() -> None:
    source = PROMOTION_PATH.read_text(encoding="utf-8")
    names = function_names(source)
    for function in [
        "duplicate_rejecting_object",
        "bounded_tree",
        "read_object_with_digest",
        "validate_live_receipt",
        "validate_native_receipt",
        "validate_registry",
        "registry_lock",
        "write_atomic",
    ]:
        require(function in names, f"promotion script is missing boundary {function}")
    for marker in [
        'receipt.get("status") != "qualified"',
        'plugin.get("mutation_rpc_methods") != []',
        'plugin.get("strings_inventory") != "passed"',
        'plugin.get("symbols_inventory") != "passed"',
        "EXPECTED_LIVE_CASE_COUNTS",
        "expected_registry_sha256",
        "compatibility registry promotion lock already exists",
        "the exact source/binary/version/platform tuple already has",
        "os.O_NOFOLLOW",
        "os.fsync",
        "--in-place",
        "--output",
    ]:
        require(marker in source, f"promotion script is missing contract marker {marker}")
    require("subprocess" not in source, "promotion boundary must not execute external commands")
    require("requests" not in source, "promotion boundary must not introduce an HTTP dependency")

    tests = TEST_PATH.read_text(encoding="utf-8")
    required_tests = [
        "test_qualified_receipts_promote_deterministically",
        "test_duplicate_json_keys_are_rejected",
        "test_expected_registry_digest_is_a_compare_and_swap_fence",
        "test_self_digested_but_structurally_invalid_existing_entry_is_rejected",
        "test_lock_prevents_concurrent_in_place_promotion",
        "test_candidate_identity_is_returned_independently_of_sort_position",
    ]
    require(tests.count("def test_") >= 14, "compatibility promotion needs at least fourteen focused tests")
    for name in required_tests:
        require(f"def {name}" in tests, f"compatibility promotion tests omit {name}")

    documentation = DOC_PATH.read_text(encoding="utf-8")
    for marker in [
        "R1",
        "R2",
        "R3",
        "R4",
        "R5",
        "experimental",
        "exact tuple",
        "no mutation",
        "registry generation",
        "compare-and-swap",
    ]:
        require(
            marker.lower() in documentation.lower(),
            f"compatibility admission documentation omits {marker}",
        )


def main() -> int:
    try:
        registry = promotion.read_object(
            REGISTRY_PATH, promotion.MAX_JSON_BYTES, "compatibility registry"
        )
        entries = promotion.validate_registry(registry)
        check_source_contracts()
    except (OSError, SyntaxError, promotion.PromotionError, RegistryError) as exc:
        print(f"live compatibility registry: FAIL: {exc}", file=sys.stderr)
        return 1
    print(f"live compatibility registry: PASS ({len(entries)} exact admitted tuple(s))")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
