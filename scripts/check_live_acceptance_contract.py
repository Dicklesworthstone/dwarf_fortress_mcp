#!/usr/bin/env python3
"""Validate the R2-R5 acceptance schema, verifier, tests, and qualification wiring."""

from __future__ import annotations

import ast
import json
import pathlib
import sys
from dataclasses import dataclass

ROOT = pathlib.Path(__file__).resolve().parents[1]
CONTRACT_PATH = ROOT / "architecture/live_read_acceptance_v1.json"
VERIFIER_PATH = ROOT / "scripts/verify_live_read_acceptance.py"
TEST_PATH = ROOT / "scripts/test_live_read_acceptance.py"
WRAPPER_PATH = ROOT / "scripts/qualify_live_read.sh"
BRIDGE_REGISTRY_PATH = ROOT / "architecture/dfhack_read_bridge_v1.json"

EXPECTED_CASES = {
    "R2": [
        "missing_token",
        "configured_token_short",
        "configured_token_long",
        "presented_token_short",
        "presented_token_long",
        "wrong_token",
        "correct_token",
        "nonce_short",
        "nonce_long",
        "nonce_mismatch",
        "protocol_mismatch",
        "secret_scan",
    ],
    "R3": [
        "baseline_names_included",
        "repeat_names_included",
        "page_size_1",
        "page_size_2",
        "page_size_7",
        "page_size_64",
        "page_size_256",
        "page_size_4096",
        "baseline_names_omitted",
        "repeat_names_omitted",
        "offset_at_total",
        "offset_beyond_total",
        "oversize_request",
        "running_multipage_rejected",
    ],
    "R4": [
        "restart_generation_changed",
        "old_client_rejected",
        "world_unloaded",
        "non_fortress_mode",
        "summary_drift",
        "partial_not_published",
        "fresh_handshake",
    ],
    "R5": ["cold_agent_turn"],
}
EXPECTED_PAGE_SIZES = [1, 2, 7, 64, 256, 4096]
EXPECTED_OMITTED = [
    "fortress.items",
    "fortress.jobs",
    "fortress.map",
    "fortress.economy",
    "fortress.welfare",
    "fortress.military",
    "fortress.history",
]
EXPECTED_SOURCE_BINDINGS = {
    "bridge_registry": "architecture/dfhack_read_bridge_v1.json",
    "bridge_proto": "bridge/dfhack-plugin/proto/DfmcpBridge.proto",
    "bridge_cpp": "bridge/dfhack-plugin/src/dfmcp_bridge.cpp",
    "rust_wire": "crates/dfmcp-adapter/src/dfhack_wire.rs",
    "live_capsule": "crates/dfmcp-adapter/src/live_observation.rs",
    "live_projection": "crates/dfmcp-adapter/src/live_projection.rs",
    "live_adapter": "crates/dfmcp-adapter/src/live_adapter.rs",
    "live_mcp_server": "crates/dfmcp-mcp/src/live_server.rs",
    "acceptance_contract": "architecture/live_read_acceptance_v1.json",
    "acceptance_verifier": "scripts/verify_live_read_acceptance.py",
}
ALLOWED_IMPORT_ROOTS = {
    "__future__",
    "argparse",
    "dataclasses",
    "hashlib",
    "json",
    "os",
    "pathlib",
    "re",
    "sys",
    "tempfile",
    "typing",
}


@dataclass(frozen=True)
class Failure:
    path: str
    message: str


def read(path: pathlib.Path, failures: list[Failure]) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as exc:
        failures.append(Failure(path.relative_to(ROOT).as_posix(), f"cannot read: {exc}"))
        return ""


def require(condition: bool, path: pathlib.Path, message: str, failures: list[Failure]) -> None:
    if not condition:
        failures.append(Failure(path.relative_to(ROOT).as_posix(), message))


def check_contract(failures: list[Failure]) -> None:
    source = read(CONTRACT_PATH, failures)
    if not source:
        return
    try:
        contract = json.loads(source)
    except json.JSONDecodeError as exc:
        failures.append(Failure(CONTRACT_PATH.relative_to(ROOT).as_posix(), f"invalid JSON: {exc}"))
        return
    require(contract.get("schema_version") == "dfmcp.live-read-acceptance/1", CONTRACT_PATH, "schema version drifted", failures)
    require(contract.get("event_schema") == "dfmcp.live-read-acceptance-event/1", CONTRACT_PATH, "event schema drifted", failures)
    require(contract.get("receipt_schema") == "dfmcp.live-read-acceptance-receipt/1", CONTRACT_PATH, "receipt schema drifted", failures)
    require(contract.get("gate_order") == ["R2", "R3", "R4", "R5"], CONTRACT_PATH, "gate order drifted", failures)
    limits = contract.get("limits")
    require(isinstance(limits, dict) and all(isinstance(value, int) and value > 0 for value in limits.values()), CONTRACT_PATH, "all explicit limits must be positive integers", failures)
    gates = contract.get("gates")
    require(isinstance(gates, dict) and set(gates) == set(EXPECTED_CASES), CONTRACT_PATH, "gate set drifted", failures)
    if isinstance(gates, dict):
        for gate, expected in EXPECTED_CASES.items():
            cases = gates.get(gate, {}).get("required_cases", [])
            names = [item.get("case") for item in cases if isinstance(item, dict)]
            require(names == expected, CONTRACT_PATH, f"{gate} case order or set drifted", failures)
    require(gates.get("R3", {}).get("page_sizes") == EXPECTED_PAGE_SIZES if isinstance(gates, dict) else False, CONTRACT_PATH, "R3 page-size matrix drifted", failures)
    require(gates.get("R5", {}).get("required_omitted_domains") == EXPECTED_OMITTED if isinstance(gates, dict) else False, CONTRACT_PATH, "R5 omitted-domain matrix drifted", failures)
    binding = contract.get("source_binding", {}).get("required_source_digests")
    require(binding == EXPECTED_SOURCE_BINDINGS, CONTRACT_PATH, "source-digest binding drifted", failures)
    forbidden = contract.get("forbidden_event_material", {})
    require("token" in forbidden.get("keys", []) and "bearer_token" in forbidden.get("keys", []), CONTRACT_PATH, "secret-bearing key denylist is incomplete", failures)


def imported_roots(tree: ast.AST) -> set[str]:
    roots: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            roots.update(alias.name.split(".", 1)[0] for alias in node.names)
        elif isinstance(node, ast.ImportFrom):
            if node.module:
                roots.add(node.module.split(".", 1)[0])
    return roots


def check_verifier(failures: list[Failure]) -> None:
    source = read(VERIFIER_PATH, failures)
    if not source:
        return
    try:
        tree = ast.parse(source)
    except SyntaxError as exc:
        failures.append(Failure(VERIFIER_PATH.relative_to(ROOT).as_posix(), f"syntax error: {exc}"))
        return
    unexpected = imported_roots(tree) - ALLOWED_IMPORT_ROOTS
    require(not unexpected, VERIFIER_PATH, f"verifier imports non-fundamental modules: {sorted(unexpected)}", failures)
    functions = {node.name for node in tree.body if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))}
    for name in [
        "bounded_tree",
        "reject_secret_material",
        "validate_native_build_receipt",
        "validate_r2",
        "validate_r3",
        "validate_r4",
        "validate_r5",
        "build_receipt",
        "verify_acceptance",
        "write_atomic",
    ]:
        require(name in functions, VERIFIER_PATH, f"missing verifier boundary {name}", failures)
    for marker in [
        "maximum_stream_bytes",
        "maximum_event_bytes",
        "maximum_events",
        "expected-dfmcp-commit",
        "native-build-receipt",
        "allow-synthetic",
        "allow-dirty-development",
        "receipt_digest",
        "synthetic-contract-fixture",
        "development-evidence",
        "qualified",
    ]:
        require(marker in source, VERIFIER_PATH, f"missing verifier marker {marker}", failures)


def check_tests(failures: list[Failure]) -> None:
    source = read(TEST_PATH, failures)
    if not source:
        return
    try:
        tree = ast.parse(source)
    except SyntaxError as exc:
        failures.append(Failure(TEST_PATH.relative_to(ROOT).as_posix(), f"syntax error: {exc}"))
        return
    tests = [node.name for node in ast.walk(tree) if isinstance(node, ast.FunctionDef) and node.name.startswith("test_")]
    require(len(tests) >= 9, TEST_PATH, "acceptance verifier needs at least nine adversarial tests", failures)
    for name in [
        "test_valid_evidence_is_qualified_and_deterministic",
        "test_missing_case_fails_closed",
        "test_pagination_digest_drift_is_rejected",
        "test_secret_bearing_key_is_rejected",
        "test_restart_generation_must_change",
        "test_partial_capsule_cannot_publish",
        "test_agent_turn_cannot_advertise_mutation",
        "test_duplicate_event_identity_is_rejected",
        "test_oversized_event_line_is_rejected",
    ]:
        require(name in tests, TEST_PATH, f"missing adversarial test {name}", failures)


def check_wrapper(failures: list[Failure]) -> None:
    source = read(WRAPPER_PATH, failures)
    for marker in [
        "git rev-parse HEAD",
        "git status --porcelain=v1",
        "--native-build-receipt",
        "--expected-dfmcp-commit",
        "--allow-dirty-development",
        "live-read-acceptance-receipt.json",
        "SHA256SUMS",
    ]:
        require(marker in source, WRAPPER_PATH, f"qualification wrapper is missing {marker}", failures)
    require("--allow-synthetic" not in source, WRAPPER_PATH, "live wrapper must never admit synthetic evidence", failures)


def check_registry(failures: list[Failure]) -> None:
    source = read(BRIDGE_REGISTRY_PATH, failures)
    if not source:
        return
    try:
        registry = json.loads(source)
    except json.JSONDecodeError as exc:
        failures.append(Failure(BRIDGE_REGISTRY_PATH.relative_to(ROOT).as_posix(), f"invalid JSON: {exc}"))
        return
    evidence = registry.get("acceptance_evidence")
    require(isinstance(evidence, dict), BRIDGE_REGISTRY_PATH, "bridge registry omits acceptance_evidence", failures)
    if isinstance(evidence, dict):
        require(evidence.get("contract") == "architecture/live_read_acceptance_v1.json", BRIDGE_REGISTRY_PATH, "bridge registry points at the wrong acceptance contract", failures)
        require(evidence.get("verifier") == "scripts/verify_live_read_acceptance.py", BRIDGE_REGISTRY_PATH, "bridge registry points at the wrong verifier", failures)
        require(evidence.get("qualification_wrapper") == "scripts/qualify_live_read.sh", BRIDGE_REGISTRY_PATH, "bridge registry points at the wrong qualification wrapper", failures)


def check_wiring(failures: list[Failure]) -> None:
    markers = [
        "scripts/check_repository_integrity.py",
        "scripts/check_bridge_auth_order.py",
        "scripts/check_live_acceptance_contract.py",
        "scripts/test_repository_integrity.py",
        "scripts/test_live_read_acceptance.py",
        "scripts/qualify_live_read.sh",
    ]
    for relative in ["scripts/verify.sh", "scripts/qualify_local.sh"]:
        path = ROOT / relative
        source = read(path, failures)
        for marker in markers:
            require(marker in source, path, f"qualification wiring omits {marker}", failures)


def main() -> int:
    failures: list[Failure] = []
    check_contract(failures)
    check_verifier(failures)
    check_tests(failures)
    check_wrapper(failures)
    check_registry(failures)
    check_wiring(failures)
    if failures:
        print(f"live acceptance contract: FAIL ({len(failures)} violation(s))", file=sys.stderr)
        for failure in failures:
            print(f"  {failure.path}: {failure.message}", file=sys.stderr)
        return 1
    print("live acceptance contract: PASS (bounded R2-R5 schema, verifier, tests, and wiring)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
