#!/usr/bin/env python3
"""Validate source-bound qualification and floor-bound descriptor execution."""

from __future__ import annotations

import ast
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACT_PATH = ROOT / "architecture/live_server_binary_receipt_v1.json"
VERIFIER_PATH = ROOT / "scripts/verify_live_server_binary_receipt.py"
VERIFIER_TEST_PATH = ROOT / "scripts/test_live_server_binary_receipt.py"
LAUNCHER_PATH = ROOT / "scripts/serve_admitted_live.py"
LAUNCHER_TEST_PATH = ROOT / "scripts/test_admitted_live_launcher.py"
TICKET_TEST_PATH = ROOT / "scripts/test_live_admission_ticket.py"
MCP_LIB_PATH = ROOT / "crates/dfmcp-mcp/src/lib.rs"
RUST_ADMISSION_PATH = ROOT / "crates/dfmcp-mcp/src/admission.rs"
AGENT_TURN_PATH = ROOT / "crates/dfmcp-mcp/src/agent_turn.rs"
BINARY_ADMISSION_TEST_PATH = ROOT / "crates/dwarf-fortress-mcp/tests/live_admission.rs"
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
    gates = binding.get("required_local_qualification_gates", [])
    for gate in [
        "live-compatibility-floor",
        "live-compatibility-floor-tests",
        "live-admission-doctor",
        "live-admission-doctor-tests",
    ]:
        require(gate in gates, f"server artifact contract omits local gate {gate}")
    digests = binding.get("required_source_digests", {})
    for name, relative in {
        "compatibility_registry": "architecture/live_compatibility_registry_v1.json",
        "compatibility_resolver": "scripts/resolve_live_compatibility.py",
        "compatibility_floor_contract": "architecture/live_compatibility_floor_v1.json",
        "compatibility_floor": "scripts/live_compatibility_floor.py",
        "compatibility_floor_checker": "scripts/check_live_compatibility_floor.py",
        "compatibility_floor_tests": "scripts/test_live_compatibility_floor.py",
        "admission_doctor_contract": "architecture/live_admission_doctor_v1.json",
        "admission_doctor": "scripts/doctor_live_admission.py",
        "admission_doctor_checker": "scripts/check_live_admission_doctor.py",
        "admission_doctor_tests": "scripts/test_doctor_live_admission.py",
        "mcp_admission": "crates/dfmcp-mcp/src/admission.rs",
        "mcp_agent_turn": "crates/dfmcp-mcp/src/agent_turn.rs",
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
        "read_bytes_with_digest",
        "read_object_with_digest",
        "validate_local_qualification_receipt",
        "open_verified_binary",
        "validate_receipt",
        "verify",
    ]:
        require(name in names, f"server binary verifier is missing {name}")
    for marker in [
        "O_NOFOLLOW",
        "O_CLOEXEC",
        "group- or world-writable",
        "opened server artifact has no executable permission bit",
        "local qualification receipt",
        "source_digests",
        "mutation_capabilities",
        "st_dev",
        "st_ino",
        "receipt_file_sha256",
        '"compatibility_floor"',
        '"admission_doctor"',
        '"mcp_admission"',
        '"mcp_agent_turn"',
    ]:
        require(marker in source, f"server binary verifier is missing marker {marker}")
    require("os.access(" not in source, "server binary verifier must inspect the opened inode, not path access bits")

    tests = VERIFIER_TEST_PATH.read_text(encoding="utf-8")
    require(tests.count("def test_") >= 12, "server binary verifier needs at least twelve adversarial tests")
    for name in [
        "test_valid_receipt_opens_exact_qualified_inode",
        "test_duplicate_json_keys_are_rejected",
        "test_binary_without_execute_bit_is_rejected_on_opened_inode",
        "test_group_writable_binary_is_rejected",
        "test_symbolic_link_binary_is_rejected",
        "test_local_qualification_receipt_mismatch_is_rejected",
        "test_source_digest_drift_is_rejected",
        "test_mutation_capability_contamination_is_rejected",
        "test_normalized_receipt_digest_matches_same_parsed_bytes",
    ]:
        require(f"def {name}" in tests, f"server binary verifier tests omit {name}")


def check_launcher() -> None:
    source = LAUNCHER_PATH.read_text(encoding="utf-8")
    names = functions(LAUNCHER_PATH)
    for name in [
        "validate_token_environment",
        "validate_loader_environment",
        "read_launch_generation",
        "build_launch_record",
        "reverify_opened_binary",
        "reverify_launch_generation",
        "prepare_launch",
        "ensure_private_ticket_directory",
        "build_admission_ticket",
        "write_admission_ticket",
        "remove_admission_ticket",
        "admitted_environment",
        "execute_verified_descriptor",
    ]:
        require(name in names, f"admitted launcher is missing {name}")
    for marker in [
        "--compatibility-floor",
        "--server-receipt",
        "--local-qualification-receipt",
        "--expected-dfmcp-commit",
        "--require-entry-id",
        "compatibility_floor.read_floor",
        "compatibility_floor.verify_generation",
        "compatibility_floor_digest",
        "compatibility_floor_file_sha256",
        "compatibility_floor_sequence",
        "binary_verifier.sha256_descriptor",
        "compatibility registry or monotonic floor changed during admitted launch preparation",
        "opened server binary bytes changed after qualification",
        "os.execve(opened_binary.descriptor",
        "os.execve not in getattr(os, \"supports_fd\", set())",
        "dynamic-loader override variables are forbidden",
    ]:
        require(marker in source, f"admitted launcher is missing marker {marker}")
    for forbidden in ["--server-sha256", "os.fexecve", "subprocess", "shell=True"]:
        require(forbidden not in source, f"admitted launcher contains forbidden path {forbidden}")

    tests = LAUNCHER_TEST_PATH.read_text(encoding="utf-8")
    require(tests.count("def test_") >= 14, "admitted launcher needs at least fourteen focused tests")
    for name in [
        "test_exact_admitted_chain_binds_floor_inode_and_receipts",
        "test_registry_floor_mismatch_is_rejected_before_binary_verification",
        "test_permissive_floor_custody_is_rejected_before_binary_verification",
        "test_required_entry_fence_is_mandatory_and_exact",
        "test_server_receipt_source_mismatch_closes_opened_descriptor",
        "test_generation_change_after_prepare_is_rejected",
        "test_same_size_binary_mutation_is_detected_before_exec",
        "test_no_path_fallback_when_descriptor_exec_is_unsupported",
    ]:
        require(f"def {name}" in tests, f"admitted launcher tests omit {name}")

    ticket_tests = TICKET_TEST_PATH.read_text(encoding="utf-8")
    require(ticket_tests.count("def test_") >= 6, "admission ticket issuance needs at least six tests")
    for name in [
        "test_ticket_fields_and_digest_are_deterministic",
        "test_ticket_file_and_directory_are_owner_private",
        "test_admitted_environment_binds_floor_ticket_and_secret_only_in_environment",
        "test_permissive_existing_ticket_directory_is_rejected",
        "test_executable_metadata_drift_is_rejected_before_ticket_issue",
        "test_same_size_executable_byte_drift_is_rejected_before_ticket_issue",
    ]:
        require(f"def {name}" in ticket_tests, f"admission ticket tests omit {name}")


def check_rust_admission() -> None:
    source = RUST_ADMISSION_PATH.read_text(encoding="utf-8")
    library = MCP_LIB_PATH.read_text(encoding="utf-8")
    for marker in [
        "dfmcp.live-admission-ticket/1",
        "DFMCP_ADMISSION_TICKET",
        "OnceLock<AdmissionProvenance>",
        "current_exe()",
        "std::process::id()",
        "compatibility_floor_digest",
        "compatibility_floor_file_sha256",
        "compatibility_floor_sequence",
        "server_binary_device",
        "server_binary_inode",
        "server_binary_owner_uid",
        "MAX_EXECUTABLE_BYTES",
        "symlink_metadata(executable_path)",
        "Digest32::of_bytes(&bytes).to_hex()",
        "current executable SHA-256 does not match the admitted server binary",
        "remove_file(path)",
        "mutation_capabilities.is_empty()",
        "std::process::exit(1)",
        "valid_ticket_is_consumed_exactly_once",
        "expired_and_cross_process_tickets_fail_closed",
        "mutation_capability_and_inode_drift_are_rejected",
        "floor_digest_and_same_size_binary_drift_are_rejected",
        "permissive_or_symbolic_ticket_paths_are_rejected",
    ]:
        require(marker in source, f"Rust live admission is missing marker {marker}")
    require(source.count("#[test]") >= 5, "Rust live admission needs at least five focused tests")
    require("pub mod admission;" in library, "dfmcp-mcp does not compile the admission boundary")
    require("mod live_server;" in library, "raw live server module is not private")
    require(
        "pub use admission::{AdmissionProvenance, current_admission_provenance, run_live_stdio};"
        in library,
        "dfmcp-mcp does not export only the admitted live runner",
    )
    require("pub mod live_server;" not in library, "raw live server module remains publicly reachable")
    require(
        "pub use live_server::run_live_stdio;" not in library,
        "raw live server runner remains publicly re-exported",
    )

    agent_turn = AGENT_TURN_PATH.read_text(encoding="utf-8")
    for marker in [
        "provenance.compatibility_floor_digest()",
        "provenance.compatibility_floor_file_sha256()",
        "provenance.compatibility_floor_sequence()",
    ]:
        require(marker in agent_turn, f"live Agent Turn omits admitted provenance marker {marker}")

    binary_tests = BINARY_ADMISSION_TEST_PATH.read_text(encoding="utf-8")
    require(binary_tests.count("#[test]") >= 2, "binary admission needs at least two process tests")
    for marker in [
        "direct_serve_live_without_ticket_fails_closed",
        "nonexistent_ticket_path_fails_before_live_server_startup",
        "CARGO_BIN_EXE_dwarf-fortress-mcp",
        "env_remove(\"DFMCP_ADMISSION_TICKET\")",
        "scripts/serve_admitted_live.py",
        "cannot inspect admission ticket",
    ]:
        require(marker in binary_tests, f"binary admission tests omit marker {marker}")


def check_wiring_and_docs() -> None:
    for relative in ["scripts/verify.sh", "scripts/qualify_local.sh"]:
        source = (ROOT / relative).read_text(encoding="utf-8")
        for marker in [
            "scripts/check_live_compatibility_floor.py",
            "scripts/test_live_compatibility_floor.py",
            "scripts/check_live_admission_doctor.py",
            "scripts/test_doctor_live_admission.py",
            "scripts/check_live_server_artifact.py",
            "scripts/test_live_server_binary_receipt.py",
            "scripts/test_admitted_live_launcher.py",
            "scripts/test_live_admission_ticket.py",
            "crates/dfmcp-mcp/src/admission.rs",
            "crates/dfmcp-mcp/src/agent_turn.rs",
            "crates/dwarf-fortress-mcp/tests/live_admission.rs",
        ]:
            require(marker in source, f"{relative} omits {marker}")
    documentation = DOC_PATH.read_text(encoding="utf-8")
    for marker in [
        "server binary receipt",
        "local qualification receipt",
        "registry generation",
        "entry ID",
        "monotonic floor",
        "anti-rollback",
        "descriptor",
        "dynamic loader",
        "no path fallback",
        "single-use admission ticket",
        "process ID",
        "executable inode",
    ]:
        require(marker.lower() in documentation.lower(), f"admission documentation omits {marker}")


def main() -> int:
    try:
        check_contract()
        check_verifier()
        check_launcher()
        check_rust_admission()
        check_wiring_and_docs()
    except (OSError, SyntaxError, json.JSONDecodeError, ContractError) as exc:
        print(f"live server artifact admission: FAIL: {exc}", file=sys.stderr)
        return 1
    print("live server artifact admission: PASS (floor, receipt, ticket, and descriptor bound)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
