#!/usr/bin/env python3
"""Validate the exact live compatibility registry and its promotion boundary."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path
from typing import Any

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


def require_ordered_strings(value: Any, expected: list[str], path: str) -> None:
    require(value == expected, f"{path} must equal the canonical ordered set {expected}")


def validate_entry(entry: dict[str, Any], index: int) -> None:
    path = f"entries[{index}]"
    require(entry.get("support_level") == "experimental", f"{path}.support_level must be experimental")
    version = promotion.require_object(entry.get("version_tuple"), f"{path}.version_tuple")
    for name in ["dwarf_fortress", "dfhack", "bridge"]:
        promotion.require_string(version.get(name), f"{path}.version_tuple.{name}", 128)
    require(version.get("protocol") == "1.0", f"{path}.version_tuple.protocol must be 1.0")
    platform = promotion.require_object(entry.get("platform"), f"{path}.platform")
    promotion.require_string(platform.get("system"), f"{path}.platform.system", 128)
    promotion.require_string(platform.get("machine"), f"{path}.platform.machine", 128)
    source = promotion.require_object(entry.get("source"), f"{path}.source")
    for name in ["dfmcp_commit", "dfhack_commit"]:
        promotion.require_commit(source.get(name), f"{path}.source.{name}")
    require(source.get("dfmcp_dirty") is False, f"{path}.source.dfmcp_dirty must be false")
    for name in [
        "plugin_sha256",
        "native_build_receipt_sha256",
        "live_acceptance_receipt_sha256",
        "live_acceptance_receipt_digest",
    ]:
        promotion.require_hash(source.get(name), f"{path}.source.{name}")
    gates = promotion.require_list(entry.get("gates"), f"{path}.gates")
    require([gate.get("gate") for gate in gates if isinstance(gate, dict)] == ["R1", "R2", "R3", "R4", "R5"], f"{path}.gates must contain R1-R5 in order")
    for gate_index, raw in enumerate(gates):
        gate = promotion.require_object(raw, f"{path}.gates[{gate_index}]")
        require(gate.get("status") == "passed", f"{path}.gates[{gate_index}] did not pass")
        if gate.get("gate") == "R1":
            promotion.require_hash(gate.get("receipt_sha256"), f"{path}.gates[{gate_index}].receipt_sha256")
        else:
            promotion.require_positive_int(gate.get("case_count"), f"{path}.gates[{gate_index}].case_count")
            promotion.require_hash(gate.get("evidence_digest"), f"{path}.gates[{gate_index}].evidence_digest")
    require_ordered_strings(entry.get("capabilities"), promotion.READ_ONLY_CAPABILITIES, f"{path}.capabilities")
    require(entry.get("mutation_capabilities") == [], f"{path}.mutation_capabilities must remain empty")
    require_ordered_strings(entry.get("observed_domains"), promotion.OBSERVED_DOMAINS, f"{path}.observed_domains")
    require_ordered_strings(entry.get("conditional_domains"), promotion.CONDITIONAL_DOMAINS, f"{path}.conditional_domains")
    require_ordered_strings(entry.get("omitted_domains"), promotion.OMITTED_DOMAINS, f"{path}.omitted_domains")
    locator = promotion.require_string(entry.get("evidence_locator"), f"{path}.evidence_locator", 1024)
    require(promotion.LOCATOR.fullmatch(locator) is not None, f"{path}.evidence_locator is malformed")
    require(".." not in Path(locator).parts, f"{path}.evidence_locator contains traversal")
    limitations = promotion.require_list(entry.get("limitations"), f"{path}.limitations")
    require(len(limitations) >= 4, f"{path}.limitations is incomplete")
    for limitation_index, value in enumerate(limitations):
        promotion.require_string(value, f"{path}.limitations[{limitation_index}]", 1024)


def check_source_contracts() -> None:
    source = PROMOTION_PATH.read_text(encoding="utf-8")
    for marker in [
        'receipt.get("status") != "qualified"',
        'plugin.get("mutation_rpc_methods") != []',
        'plugin.get("strings_inventory") != "passed"',
        'plugin.get("symbols_inventory") != "passed"',
        "the exact source/binary/version/platform tuple already has",
        "write_atomic",
        "--in-place",
        "--output",
    ]:
        require(marker in source, f"promotion script is missing contract marker {marker}")
    require("subprocess" not in source, "promotion boundary must not execute external commands")
    require("requests" not in source, "promotion boundary must not introduce an HTTP dependency")
    tests = TEST_PATH.read_text(encoding="utf-8")
    require(tests.count("def test_") >= 8, "compatibility promotion needs at least eight focused tests")
    documentation = DOC_PATH.read_text(encoding="utf-8")
    for marker in ["R1", "R2", "R3", "R4", "R5", "experimental", "exact tuple", "no mutation"]:
        require(marker.lower() in documentation.lower(), f"compatibility admission documentation omits {marker}")


def main() -> int:
    try:
        registry = promotion.read_object(REGISTRY_PATH, 8 * 1024 * 1024, "compatibility registry")
        entries = promotion.validate_registry(registry)
        for index, entry in enumerate(entries):
            validate_entry(entry, index)
        check_source_contracts()
    except (OSError, promotion.PromotionError, RegistryError) as exc:
        print(f"live compatibility registry: FAIL: {exc}", file=sys.stderr)
        return 1
    print(f"live compatibility registry: PASS ({len(entries)} exact admitted tuple(s))")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
