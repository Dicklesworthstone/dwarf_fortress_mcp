#!/usr/bin/env python3
"""Validate fail-closed exact-tuple resolution and registry-generation binding."""

from __future__ import annotations

import ast
import importlib.util
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REGISTRY_PATH = ROOT / "architecture/live_compatibility_registry_v1.json"
PROMOTION_PATH = ROOT / "scripts/promote_live_compatibility.py"
RESOLVER_PATH = ROOT / "scripts/resolve_live_compatibility.py"
TEST_PATH = ROOT / "scripts/test_live_compatibility_resolution.py"
DOC_PATH = ROOT / "docs/LIVE_COMPATIBILITY_ADMISSION.md"


def load(name: str, path: Path):
    specification = importlib.util.spec_from_file_location(name, path)
    if specification is None or specification.loader is None:
        raise RuntimeError(f"cannot load {name}")
    module = importlib.util.module_from_spec(specification)
    sys.modules[specification.name] = module
    specification.loader.exec_module(module)
    return module


promotion = load("promote_live_compatibility", PROMOTION_PATH)
resolver = load("resolve_live_compatibility", RESOLVER_PATH)


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


def check_source() -> None:
    source = RESOLVER_PATH.read_text(encoding="utf-8")
    names = function_names(source)
    for function in [
        "validate_manifest",
        "deployment_key",
        "classify_miss",
        "resolve",
        "write_atomic",
    ]:
        require(function in names, f"compatibility resolver is missing {function}")
    for marker in [
        "registry_digest",
        "registry_status",
        "required_entry_id",
        "list(entry[\"capabilities\"])",
        "list(entry[\"omitted_domains\"])",
        "decision_digest",
        "--require-entry-id",
        "promotion.write_atomic",
    ]:
        require(marker in source, f"compatibility resolver is missing marker {marker}")
    require("DFMCP_BRIDGE_TOKEN" not in source, "compatibility resolution must never read the bearer token")
    require("subprocess" not in source, "compatibility resolution must not execute the candidate deployment")
    require("requests" not in source, "compatibility resolution must not introduce HTTP authority")

    tests = TEST_PATH.read_text(encoding="utf-8")
    require(tests.count("def test_") >= 12, "compatibility resolution needs at least twelve focused tests")
    for name in [
        "test_exact_tuple_is_admitted_deterministically",
        "test_required_entry_id_mismatch_fails_closed_and_is_bound",
        "test_correct_entry_fence_changes_decision_identity",
        "test_registry_generation_changes_decision_identity",
        "test_decision_lists_do_not_alias_registry_lists",
    ]:
        require(f"def {name}" in tests, f"compatibility resolution tests omit {name}")

    documentation = DOC_PATH.read_text(encoding="utf-8")
    for marker in [
        "deployment manifest",
        "exact",
        "fail closed",
        "registry generation",
        "entry ID",
        "decision digest",
    ]:
        require(marker.lower() in documentation.lower(), f"compatibility documentation omits {marker}")


def check_empty_registry() -> None:
    registry = promotion.read_object(
        REGISTRY_PATH, promotion.MAX_JSON_BYTES, "compatibility registry"
    )
    entries = promotion.validate_registry(registry)
    if entries:
        return
    manifest = {
        "schema": resolver.MANIFEST_SCHEMA,
        "version_tuple": {
            "dwarf_fortress": "unadmitted",
            "dfhack": "unadmitted",
            "bridge": "unadmitted",
            "protocol": "1.0",
        },
        "platform": {"system": "unadmitted", "machine": "unadmitted"},
        "source": {
            "dfmcp_commit": "0" * 40,
            "dfhack_commit": "0" * 40,
            "plugin_sha256": "0" * 64,
        },
    }
    decision = resolver.resolve(registry, manifest, "1" * 64)
    require(decision["admitted"] is False, "an empty registry admitted a deployment")
    require(decision["entry_id"] is None, "an empty registry returned an entry identifier")
    require(decision["required_entry_id"] == "1" * 64, "entry fence was not bound")
    require(decision["capabilities"] == [], "an empty registry granted capabilities")
    require(decision["mutation_capabilities"] == [], "an empty registry granted mutation authority")
    require(decision["registry_entry_count"] == 0, "empty registry count is incorrect")
    require(
        decision["registry_digest"]
        == promotion.sha256_bytes(promotion.canonical_json(registry)),
        "empty-registry decision does not bind the registry generation",
    )


def main() -> int:
    try:
        check_source()
        check_empty_registry()
    except (
        OSError,
        SyntaxError,
        ContractError,
        promotion.PromotionError,
        resolver.ResolutionError,
    ) as exc:
        print(f"live compatibility resolution: FAIL: {exc}", file=sys.stderr)
        return 1
    print("live compatibility resolution: PASS (exact tuple, registry generation, and entry fence)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
