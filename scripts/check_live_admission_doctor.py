#!/usr/bin/env python3
"""Validate deterministic, authority-free live-admission readiness diagnosis."""

from __future__ import annotations

import ast
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACT_PATH = ROOT / "architecture/live_admission_doctor_v1.json"
IMPLEMENTATION_PATH = ROOT / "scripts/doctor_live_admission.py"
TEST_PATH = ROOT / "scripts/test_doctor_live_admission.py"
DOC_PATH = ROOT / "docs/LIVE_COMPATIBILITY_ADMISSION.md"
EXPECTED_STAGES = [
    "registry",
    "compatibility_floor",
    "exact_tuple_resolution",
    "server_artifact",
]


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
        contract.get("schema_version") == "dfmcp.live-admission-doctor-contract/1",
        "live admission doctor contract schema drifted",
    )
    require(
        contract.get("report_schema") == "dfmcp.live-admission-doctor/1",
        "live admission doctor report schema drifted",
    )
    require(contract.get("stages") == EXPECTED_STAGES, "doctor stage order drifted")
    readiness = contract.get("readiness_states", {})
    require(
        set(readiness) == {
            "not_ready",
            "compatibility_ready",
            "artifact_preflight_ready",
        },
        "doctor readiness states drifted",
    )
    determinism = contract.get("determinism", {})
    require(determinism.get("timestamps_allowed") is False, "doctor contract permits timestamps")
    require(
        determinism.get("environment_secrets_allowed") is False,
        "doctor contract permits environment secrets",
    )
    authority = contract.get("authority", {})
    for field in [
        "executes_server",
        "connects_to_dfhack",
        "reads_bridge_token",
        "modifies_registry",
        "modifies_floor",
    ]:
        require(authority.get(field) is False, f"doctor contract permits {field}")
    require(authority.get("grants_capabilities") == [], "doctor contract grants capability")
    require(authority.get("mutation_capabilities") == [], "doctor contract grants mutation")


def check_implementation() -> None:
    source = IMPLEMENTATION_PATH.read_text(encoding="utf-8")
    names = function_names(source)
    for name in [
        "bounded_text",
        "recovery",
        "stage",
        "not_checked",
        "finish",
        "artifact_inputs",
        "diagnose",
    ]:
        require(name in names, f"live admission doctor is missing {name}")
    for marker in [
        'REPORT_SCHEMA = "dfmcp.live-admission-doctor/1"',
        "STAGE_ORDER",
        "compatibility_floor.read_floor",
        "compatibility_floor.verify_generation",
        "resolver.resolve",
        "binary_verifier.verify",
        '"reads_bridge_token": False',
        '"grants_capabilities": []',
        '"mutation_capabilities": []',
        "server artifact inputs are all-or-none",
        "report_digest",
    ]:
        require(marker in source, f"live admission doctor is missing marker {marker}")
    for forbidden in [
        "DFMCP_BRIDGE_TOKEN",
        "subprocess",
        "socket",
        "requests",
        "urllib",
        "time.time",
        "datetime",
        "os.environ",
        "os.getenv",
        "os.exec",
        "shell=True",
    ]:
        require(forbidden not in source, f"live admission doctor contains forbidden authority path {forbidden}")
    require("assert " not in source, "live admission doctor must not rely on optimization-sensitive asserts")


def check_tests_and_docs() -> None:
    tests = TEST_PATH.read_text(encoding="utf-8")
    require(tests.count("def test_") >= 14, "live admission doctor needs at least fourteen tests")
    for name in [
        "test_compatibility_ready_report_is_deterministic_and_digest_bound",
        "test_authority_section_is_explicitly_empty",
        "test_secret_environment_does_not_affect_report",
        "test_registry_floor_mismatch_fails_before_tuple_resolution",
        "test_permissive_floor_custody_fails_closed",
        "test_wrong_entry_fence_is_not_ready",
        "test_empty_registry_reports_exact_tuple_failure",
        "test_partial_artifact_inputs_are_rejected_as_usage_error",
        "test_artifact_preflight_ready_closes_opened_descriptor",
        "test_artifact_source_mismatch_is_not_ready",
        "test_artifact_platform_mismatch_is_not_ready",
        "test_invalid_registry_preserves_fixed_stage_order",
        "test_diagnostic_text_is_bounded_and_control_safe",
        "test_report_contains_no_timestamps_or_runtime_authority",
    ]:
        require(f"def {name}" in tests, f"live admission doctor tests omit {name}")
    documentation = DOC_PATH.read_text(encoding="utf-8")
    for marker in [
        "admission doctor",
        "compatibility_ready",
        "artifact_preflight_ready",
        "authority-free",
        "does not execute",
    ]:
        require(marker.lower() in documentation.lower(), f"admission documentation omits {marker}")


def main() -> int:
    try:
        check_contract()
        check_implementation()
        check_tests_and_docs()
    except (OSError, SyntaxError, json.JSONDecodeError, ContractError) as exc:
        print(f"live admission doctor: FAIL: {exc}", file=sys.stderr)
        return 1
    print("live admission doctor: PASS (deterministic authority-free preflight)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
