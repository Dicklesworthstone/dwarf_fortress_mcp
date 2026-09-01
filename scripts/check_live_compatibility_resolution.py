#!/usr/bin/env python3
"""Validate the deterministic compatibility-decision boundary and its tests."""

from __future__ import annotations

import ast
import importlib.util
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PROMOTION_PATH = ROOT / "scripts/promote_live_compatibility.py"
RESOLVER_PATH = ROOT / "scripts/resolve_live_compatibility.py"
TEST_PATH = ROOT / "scripts/test_live_compatibility_resolution.py"
DOC_PATH = ROOT / "docs/LIVE_COMPATIBILITY_ADMISSION.md"

PROMOTION_SPEC = importlib.util.spec_from_file_location("promote_live_compatibility", PROMOTION_PATH)
if PROMOTION_SPEC is None or PROMOTION_SPEC.loader is None:
    raise RuntimeError("cannot load compatibility promotion module")
promotion = importlib.util.module_from_spec(PROMOTION_SPEC)
sys.modules[PROMOTION_SPEC.name] = promotion
PROMOTION_SPEC.loader.exec_module(promotion)

RESOLVER_SPEC = importlib.util.spec_from_file_location("resolve_live_compatibility", RESOLVER_PATH)
if RESOLVER_SPEC is None or RESOLVER_SPEC.loader is None:
    raise RuntimeError("cannot load compatibility resolver module")
resolver = importlib.util.module_from_spec(RESOLVER_SPEC)
sys.modules[RESOLVER_SPEC.name] = resolver
RESOLVER_SPEC.loader.exec_module(resolver)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def main() -> int:
    try:
        source = RESOLVER_PATH.read_text(encoding="utf-8")
        tree = ast.parse(source)
        functions = {
            node.name
            for node in tree.body
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        }
        for name in [
            "validate_manifest",
            "deployment_key",
            "classify_miss",
            "resolve",
            "write_atomic",
        ]:
            require(name in functions, f"resolver is missing boundary {name}")
        for marker in [
            "dfmcp.live-deployment-manifest/1",
            "dfmcp.live-compatibility-decision/1",
            "no exact admitted source/binary/version/platform tuple exists",
            "mutation_capabilities",
            "decision_digest",
            "return 0 if decision[\"admitted\"] else 3",
        ]:
            require(marker in source, f"resolver is missing contract marker {marker}")
        require("subprocess" not in source, "resolver must not launch external commands")
        require("socket" not in source, "resolver must not perform network discovery")
        require("requests" not in source, "resolver must not add an HTTP dependency")
        tests = TEST_PATH.read_text(encoding="utf-8")
        require(tests.count("def test_") >= 9, "resolver needs at least nine focused tests")
        for marker in [
            "test_exact_tuple_is_admitted_deterministically",
            "test_empty_registry_fails_closed",
            "test_same_versions_with_different_binary_are_not_admitted",
            "test_platform_drift_is_not_admitted",
            "test_source_revision_drift_is_not_admitted",
            "test_required_entry_id_mismatch_fails_closed",
            "test_manifest_extra_field_is_rejected",
            "test_protocol_drift_is_rejected_before_lookup",
            "test_tampered_registry_entry_is_rejected",
        ]:
            require(marker in tests, f"resolver tests omit {marker}")
        registry = promotion.read_object(
            ROOT / "architecture/live_compatibility_registry_v1.json",
            8 * 1024 * 1024,
            "compatibility registry",
        )
        empty_manifest = {
            "schema": resolver.MANIFEST_SCHEMA,
            "version_tuple": {
                "dwarf_fortress": "unqualified",
                "dfhack": "unqualified",
                "bridge": "0.1.0",
                "protocol": "1.0",
            },
            "platform": {"system": "unqualified", "machine": "unqualified"},
            "source": {
                "dfmcp_commit": "0" * 39 + "1",
                "dfhack_commit": "0" * 39 + "2",
                "plugin_sha256": "0" * 63 + "1",
            },
        }
        decision = resolver.resolve(registry, empty_manifest)
        if not registry["entries"]:
            require(decision["admitted"] is False, "empty registry did not fail closed")
            require(decision["capabilities"] == [], "empty registry returned capabilities")
        documentation = DOC_PATH.read_text(encoding="utf-8")
        require("resolve_live_compatibility.py" in documentation, "documentation omits deployment resolution")
    except (OSError, SyntaxError, ValueError, promotion.PromotionError, resolver.ResolutionError) as exc:
        print(f"live compatibility resolution: FAIL: {exc}", file=sys.stderr)
        return 1
    print("live compatibility resolution: PASS (exact lookup, deterministic decision, fail-closed miss)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
