#!/usr/bin/env python3
"""Validate source-bound server qualification and descriptor-only admitted execution."""

from __future__ import annotations

import ast
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACT_PATH = ROOT / "architecture/live_server_binary_receipt_v1.json"
VERIFIER_PATH = ROOT / "scripts/verify_live_server_binary_receipt.py"
LAUNCHER_PATH = ROOT / "scripts/serve_admitted_live.py"
TEST_PATH = ROOT / "scripts/test_admitted_live_launcher.py"
DOC_PATH = ROOT / "docs/LIVE_COMPATIBILITY_ADMISSION.md"


class ContractError(ValueError):
    pass


def fail(message: str) -> None:
    raise ContractError(message)


def require(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


def functions(path: Path) -> set[str]:
    tree = ast.parse(path.read_text(encoding="utf-8"))
    return {
        node.name
        for node in ast.walk(tree)
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
    }


def check_contract() -> None:
    value = json.loads(CONTRACT_PATH.read_text(encoding="utf-8"))
    require(
        value.get("schema_version") == "dfmcp.live-server-binary-receipt-contract/1",
        "server artifact contract schema drifted",
    )
    require(
        value.get("receipt_schema") == "dfmcp.live-server-binary-qualification/1",
        "server receipt schema drifted",
    )
    binding = value.get("source_binding", {})
    require(binding.get("requires_clean_dfmcp_source") is True, "clean source is not required")
    require(
        binding.get("requires_passing_local_qualification_receipt") is True,
        "passing local qualification is not required",
    )
    digests = binding.get("required_source_digests", {})
    for name, relative in {
        "compatibility_registry": "architecture/live_compatibility_registry_v1.json",
        "compatibility_resolver": "scripts/resolve_live_compatibility.py",
        "artifact_verifier": "scripts/verify_live_server_binary_receipt.py",
        "admitted_launcher": "scripts/serve_admitted_live.py",
    }.items():
        require(digests.get(name) == relative, f"server artifact source binding omits {name}")
    require(value.get("authority", {}).get("mutation_capabilities") == [], "artifact contract admits mutation")


def check_verifier() -> None:
    source = VERIFIER_PATH.read_text(encoding="utf-8")
    names = functions(VERIFIER_PATH)
    for name in [
        "duplicate_rejecting_object",
        "bounded_tree",
        "validate_local_qualification_receipt",
        "open_verified_binary",
        "validate_receipt",
        "verify",
    ]:
        require(name in names, f"server binary verifier is missing {name}")
    for marker in [
        "O_NOFOLLOW",
        "group/world-writable",
        "local qualification receipt",
        "source_digests",
        "mutation_capabilities",
        "st_dev",
        "st_ino",
    ]:
        require(marker in source, f"server binary verifier is missing marker {marker}")


def check_launcher() -> None:
    source = LAUNCHER_PATH.read_text(encoding="utf-8")
    names = functions(LAUNCHER_PATH)
    for name in [
        "validate_token_environment",
        "validate_loader_environment",
        "build_launch_record",
        "prepare_launch",
        "admitted_environment",
        "execute_verified_descriptor",
    ]:
        require(name in names, f"admitted launcher is missing {name}")
    for marker in [
        "--server-receipt",
        "--local-qualification-receipt",
        "--expected-dfmcp-commit",
        "--require-entry-id",
        "compatibility_registry_digest",
        "local_qualification_receipt_sha256",
        "owner_uid",
        "authorized_to_exec",
        "os.execve(opened_binary.descriptor",
        "os.execve not in getattr(os, \"supports_fd\", set())",
        "dynamic-loader override variables are forbidden",
    ]:
        require(marker in source, f"admitted launcher is missing marker {marker}")
    for forbidden in ["--server-sha256", "os.fexecve", "subprocess", "shell=True"]:
        require(forbidden not in source, f"admitted launcher contains forbidden path {forbidden}")

    tests = TEST_PATH.read_text(encoding="utf-8")
    require(tests.count("def test_") >= 10, "admitted launcher needs at least ten focused tests")
    for name in [
        "test_exact_admitted_chain_binds_opened_inode_and_receipts",
        "test_loader_injection_environment_is_rejected",
        "test_required_entry_fence_is_mandatory_and_exact",
        "test_server_receipt_source_mismatch_closes_opened_descriptor",
        "test_no_path_fallback_when_descriptor_exec_is_unsupported",
    ]:
        require(f"def {name}" in tests, f"admitted launcher tests omit {name}")


def check_wiring_and_docs() -> None:
    for relative in ["scripts/verify.sh", "scripts/qualify_local.sh"]:
        source = (ROOT / relative).read_text(encoding="utf-8")
        for marker in [
            "scripts/check_live_server_artifact.py",
            "scripts/test_admitted_live_launcher.py",
        ]:
            require(marker in source, f"{relative} omits {marker}")
    documentation = DOC_PATH.read_text(encoding="utf-8")
    for marker in [
        "server binary receipt",
        "local qualification receipt",
        "registry generation",
        "entry ID",
        "descriptor",
        "dynamic loader",
        "no path fallback",
    ]:
        require(marker.lower() in documentation.lower(), f"admission documentation omits {marker}")


def main() -> int:
    try:
        check_contract()
        check_verifier()
        check_launcher()
        check_wiring_and_docs()
    except (OSError, SyntaxError, json.JSONDecodeError, ContractError) as exc:
        print(f"live server artifact admission: FAIL: {exc}", file=sys.stderr)
        return 1
    print("live server artifact admission: PASS (receipt-bound descriptor-only execution)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
