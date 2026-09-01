#!/usr/bin/env python3
"""Validate the agent-facing live-read capture plan and guidance command."""

from __future__ import annotations

import ast
import json
import pathlib
import sys
from dataclasses import dataclass
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
ACCEPTANCE = ROOT / "architecture/live_read_acceptance_v1.json"
PLAN = ROOT / "architecture/live_read_capture_plan_v1.json"
GUIDANCE = ROOT / "scripts/live_read_capture_guidance.py"
TESTS = ROOT / "scripts/test_live_read_capture_guidance.py"
ALLOWED_IMPORTS = {
    "__future__",
    "argparse",
    "json",
    "live_read_evidence_journal",
    "pathlib",
    "sys",
    "typing",
}
FORBIDDEN_ARGUMENTS = {
    "bash",
    "sh",
    "zsh",
    "cmd",
    "powershell",
    "pwsh",
    "-c",
    "RunCommand",
    "RunLua",
    "ApplyEffect",
    "SetPauseState",
    "SF_ALLOW_REMOTE",
}


@dataclass(frozen=True)
class Failure:
    path: str
    message: str


def relative(path: pathlib.Path) -> str:
    return path.relative_to(ROOT).as_posix()


def read(path: pathlib.Path, failures: list[Failure]) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as exc:
        failures.append(Failure(relative(path), f"cannot read: {exc}"))
        return ""


def parse_json(path: pathlib.Path, failures: list[Failure]) -> dict[str, Any] | None:
    source = read(path, failures)
    if not source:
        return None
    try:
        value = json.loads(source)
    except json.JSONDecodeError as exc:
        failures.append(Failure(relative(path), f"invalid JSON: {exc}"))
        return None
    if not isinstance(value, dict):
        failures.append(Failure(relative(path), "top-level JSON value must be an object"))
        return None
    return value


def require(condition: bool, path: pathlib.Path, message: str, failures: list[Failure]) -> None:
    if not condition:
        failures.append(Failure(relative(path), message))


def expected_cases(acceptance: dict[str, Any]) -> list[tuple[str, str]]:
    output: list[tuple[str, str]] = []
    for gate in acceptance.get("gate_order", []):
        gate_value = acceptance.get("gates", {}).get(gate, {})
        for item in gate_value.get("required_cases", []):
            if isinstance(item, dict):
                output.append((gate, item.get("case")))
    return output


def imported_roots(tree: ast.AST) -> set[str]:
    roots: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            roots.update(alias.name.split(".", 1)[0] for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.module:
            roots.add(node.module.split(".", 1)[0])
    return roots


def function_names(tree: ast.AST) -> set[str]:
    return {
        node.name
        for node in ast.walk(tree)
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
    }


def check_plan(failures: list[Failure]) -> None:
    acceptance = parse_json(ACCEPTANCE, failures)
    plan = parse_json(PLAN, failures)
    if acceptance is None or plan is None:
        return
    require(plan.get("schema_version") == "dfmcp.live-read-capture-plan/1", PLAN, "schema version drifted", failures)
    contract = plan.get("output_contract")
    require(isinstance(contract, dict), PLAN, "output_contract is missing", failures)
    if isinstance(contract, dict):
        for field in [
            "secrets_in_arguments",
            "arbitrary_dfhack_commands",
            "arbitrary_lua",
            "mutation_authority",
        ]:
            require(contract.get(field) is False, PLAN, f"output_contract.{field} must remain false", failures)
    raw_cases = plan.get("cases")
    require(isinstance(raw_cases, list), PLAN, "cases must be an array", failures)
    if not isinstance(raw_cases, list):
        return
    actual: list[tuple[str, str]] = []
    for index, item in enumerate(raw_cases):
        require(isinstance(item, dict), PLAN, f"case {index} is not an object", failures)
        if not isinstance(item, dict):
            continue
        gate = item.get("gate")
        case = item.get("case")
        actual.append((gate, case))
        kind = item.get("capture_kind")
        require(kind in {"probe", "scanner", "composite"}, PLAN, f"{gate}/{case} has invalid capture_kind", failures)
        require(isinstance(item.get("automatable"), bool), PLAN, f"{gate}/{case} automatable is not Boolean", failures)
        append_mode = item.get("append_mode")
        require(append_mode in {"append", "append-probe"}, PLAN, f"{gate}/{case} has invalid append_mode", failures)
        argv = item.get("argv")
        require(isinstance(argv, list) and len(argv) <= 64, PLAN, f"{gate}/{case} argv is invalid", failures)
        if not isinstance(argv, list):
            continue
        require(all(isinstance(value, str) and value for value in argv), PLAN, f"{gate}/{case} argv contains an invalid value", failures)
        joined = "\0".join(value for value in argv if isinstance(value, str))
        for forbidden in FORBIDDEN_ARGUMENTS:
            require(forbidden not in argv and forbidden not in joined, PLAN, f"{gate}/{case} argv contains forbidden fragment {forbidden}", failures)
        require("DFMCP_BRIDGE_TOKEN=" not in joined and "--token" not in joined and "--secret" not in joined, PLAN, f"{gate}/{case} argv exposes secret material", failures)
        if kind == "probe":
            require("dfmcp-live-probe" in argv and append_mode == "append-probe", PLAN, f"{gate}/{case} probe command or append mode drifted", failures)
        elif kind == "scanner":
            require("scripts/scan_live_read_secrets.py" in argv and append_mode == "append", PLAN, f"{gate}/{case} scanner command or append mode drifted", failures)
        elif kind == "composite":
            require(item.get("automatable") is False and append_mode == "append", PLAN, f"{gate}/{case} composite posture is unsafe", failures)
        preconditions = item.get("preconditions")
        require(isinstance(preconditions, list) and 1 <= len(preconditions) <= 16, PLAN, f"{gate}/{case} precondition list is invalid", failures)
        require(isinstance(item.get("operator_action"), str) and bool(item.get("operator_action")), PLAN, f"{gate}/{case} operator action is missing", failures)
    require(actual == expected_cases(acceptance), PLAN, "case order or set differs from the acceptance contract", failures)


def check_guidance(failures: list[Failure]) -> None:
    source = read(GUIDANCE, failures)
    if not source:
        return
    try:
        tree = ast.parse(source)
    except SyntaxError as exc:
        failures.append(Failure(relative(GUIDANCE), f"syntax error: {exc}"))
        return
    unexpected = imported_roots(tree) - ALLOWED_IMPORTS
    require(not unexpected, GUIDANCE, f"unexpected import roots: {sorted(unexpected)}", failures)
    present = function_names(tree)
    for name in [
        "read_plan",
        "validate_argv",
        "validate_plan",
        "safe_artifact_name",
        "substitute_argv",
        "next_guidance",
    ]:
        require(name in present, GUIDANCE, f"required function {name} is missing", failures)
    for marker in [
        "command_representation",
        "ready_to_execute",
        "capture_stdout_to",
        "append_argv",
        "required_inputs",
        "operator_action",
        "finalize_argv",
    ]:
        require(marker in source, GUIDANCE, f"guidance output omits {marker}", failures)
    for forbidden in ["shell=True", "os.system", "subprocess", "DFMCP_BRIDGE_TOKEN="]:
        require(forbidden not in source, GUIDANCE, f"guidance contains forbidden execution/secret token {forbidden}", failures)


def check_tests(failures: list[Failure]) -> None:
    source = read(TESTS, failures)
    if not source:
        return
    try:
        tree = ast.parse(source)
    except SyntaxError as exc:
        failures.append(Failure(relative(TESTS), f"syntax error: {exc}"))
        return
    present = {name for name in function_names(tree) if name.startswith("test_")}
    expected = {
        "test_plan_matches_every_acceptance_case_in_order",
        "test_shell_interpreter_and_secret_fragments_are_rejected",
        "test_duplicate_case_is_rejected",
        "test_case_order_drift_is_rejected",
        "test_probe_guidance_is_ready_and_uses_append_probe",
        "test_scanner_guidance_requires_artifact_root",
        "test_scanner_guidance_substitutes_explicit_artifact_root",
        "test_composite_case_is_explicitly_not_automatable",
        "test_complete_journal_returns_only_finalize_guidance",
    }
    require(expected <= present, TESTS, "capture-guidance adversarial test matrix is incomplete", failures)


def check_wiring(failures: list[Failure]) -> None:
    markers = [
        "scripts/check_live_capture_plan.py",
        "scripts/live_read_capture_guidance.py",
        "scripts/test_live_read_capture_guidance.py",
    ]
    for relative_path in ["scripts/verify.sh", "scripts/qualify_local.sh"]:
        path = ROOT / relative_path
        source = read(path, failures)
        for marker in markers:
            require(marker in source, path, f"qualification wiring omits {marker}", failures)
    qualify = read(ROOT / "scripts/qualify_local.sh", failures)
    for digest_name in [
        "live_capture_plan",
        "live_capture_plan_checker",
        "live_capture_guidance",
        "live_capture_guidance_tests",
    ]:
        require(digest_name in qualify, ROOT / "scripts/qualify_local.sh", f"qualification receipt omits {digest_name}", failures)


def main() -> int:
    failures: list[Failure] = []
    check_plan(failures)
    check_guidance(failures)
    check_tests(failures)
    check_wiring(failures)
    if failures:
        print(f"live capture plan: FAIL ({len(failures)} violation(s))", file=sys.stderr)
        for failure in failures:
            print(f"  {failure.path}: {failure.message}", file=sys.stderr)
        return 1
    print("live capture plan: PASS (exact case coverage, argv-only guidance, no authority or secrets)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
