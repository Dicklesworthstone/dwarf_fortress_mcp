#!/usr/bin/env python3
"""Validate HEAD-exact, source-stable local qualification receipt issuance."""

from __future__ import annotations

import ast
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = ROOT / "architecture/local_qualification_receipt_v1.json"
WRITER = ROOT / "scripts/write_local_qualification_receipt.py"
TESTS = ROOT / "scripts/test_local_qualification_receipt.py"
WRAPPER = ROOT / "scripts/qualify_local.sh"
VERIFY = ROOT / "scripts/verify.sh"
SERVER_CONTRACT = ROOT / "architecture/live_server_binary_receipt_v1.json"
SERVER_VERIFIER = ROOT / "scripts/verify_live_server_binary_receipt.py"
DOC = ROOT / "docs/LOCAL_QUALIFICATION_AND_RELEASE.md"


class ContractError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def function_names(path: Path) -> set[str]:
    tree = ast.parse(path.read_text(encoding="utf-8"))
    return {
        node.name
        for node in ast.walk(tree)
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
    }


def check_contract() -> None:
    value = json.loads(CONTRACT.read_text(encoding="utf-8"))
    require(
        value.get("schema_version") == "dfmcp.local-qualification-receipt-contract/1",
        "local qualification contract schema drifted",
    )
    require(
        value.get("receipt_schema") == "dfmcp.qualification-receipt.v1",
        "local qualification receipt schema drifted",
    )
    require(
        value.get("snapshot_schema") == "dfmcp.qualification-source-snapshot/1",
        "local qualification snapshot schema drifted",
    )
    require(
        value.get("status") == "normative_two_phase_local_qualification",
        "local qualification contract status drifted",
    )
    require(
        value.get("gate_contract") == "architecture/live_server_binary_receipt_v1.json",
        "local qualification gate contract drifted",
    )
    source = value.get("source", {})
    for field in [
        "clean_required_for_passed",
        "head_must_remain_exact",
        "tree_must_remain_exact",
        "worktree_status_must_remain_exact",
        "tracked_inventory_must_remain_exact",
        "working_tree_bytes_must_match_head_blobs_for_clean_passed",
        "working_tree_executable_semantics_must_match_head_on_unix",
        "git_environment_must_be_sanitized",
    ]:
        require(source.get(field) is True, f"local qualification contract weakens {field}")
    require(source.get("git_object_format") == "sha1", "Git object format drifted")
    require(source.get("tracked_entry_modes") == ["100644", "100755"], "tracked modes drifted")
    require(source.get("symbolic_links_allowed") is False, "source symlinks are allowed")
    require(source.get("gitlinks_allowed") is False, "source gitlinks are allowed")
    require(source.get("hash_algorithm") == "sha256", "source hash algorithm drifted")
    require(
        list(value.get("receipt_statuses", {}))
        == ["passed", "development_dirty", "static_only", "failed"],
        "local qualification status set or order drifted",
    )
    publication = value.get("publication", {})
    for field in [
        "run_directory_create_only",
        "run_directory_owner_must_match_effective_user_when_available",
        "snapshot_create_only",
        "receipt_create_only",
        "temporary_file_fsync",
        "atomic_no_replace_hard_link",
        "parent_directory_fsync",
        "reverify_source_after_receipt_publication",
        "invalid_evidence_removed_and_absence_verified",
    ]:
        require(publication.get(field) is True, f"local receipt publication weakens {field}")
    require(
        publication.get("run_directory_final_component_symbolic_links_allowed") is False,
        "qualification run directory may be a symbolic link",
    )
    for field, expected in [
        ("unix_run_directory_mode", "0700"),
        ("gate_journal_mode", "0600"),
        ("snapshot_mode", "0600"),
        ("receipt_mode", "0600"),
    ]:
        require(publication.get(field) == expected, f"local receipt publication drifts {field}")
    authority = value.get("authority", {})
    for field in ["executes_project_code", "modifies_source", "network_access"]:
        require(authority.get(field) is False, f"local receipt contract grants {field}")
    require(authority.get("grants_capabilities") == [], "local receipt grants capability")
    require(authority.get("mutation_capabilities") == [], "local receipt grants mutation")


def check_writer() -> None:
    source = WRITER.read_text(encoding="utf-8")
    names = function_names(WRITER)
    for name in [
        "git_blob_object_id",
        "sanitized_git_environment",
        "run_git",
        "git_identity",
        "git_status",
        "load_contract",
        "load_required_gates",
        "executable_semantics_match",
        "collect_source_snapshot",
        "validate_snapshot",
        "validate_private_evidence_directory",
        "private_evidence_candidate",
        "validate_private_evidence_file",
        "read_private_evidence_file",
        "parse_gates",
        "write_atomic_create_only",
        "remove_published_evidence",
        "begin",
        "finish",
    ]:
        require(name in names, f"local qualification issuer omits {name}")
    for marker in [
        'SNAPSHOT_SCHEMA = "dfmcp.qualification-source-snapshot/1"',
        'RECEIPT_SCHEMA = "dfmcp.qualification-receipt.v1"',
        'if not key.startswith("GIT_")',
        'environment["GIT_CONFIG_NOSYSTEM"] = "1"',
        '"status", "--porcelain=v1", "-z", "--untracked-files=all"',
        '"ls-tree", "-rz", "--full-tree"',
        'mode not in {"100644", "100755"}',
        "git_blob_object_id(stable.content)",
        "head_equivalent = False",
        '"head_equivalent": head_equivalent',
        "allow_empty=True",
        "worktree status changed while collecting the qualification snapshot",
        "source snapshot changed during local qualification",
        'final_status = "development_dirty"',
        "a clean passing qualification receipt must be HEAD-equivalent",
        "qualification gate journal is not an exact prefix",
        "a completed qualification requires every canonical gate to pass",
        "qualification evidence directory must have exact owner-only mode 0700",
        "qualification evidence files must share one private run directory",
        "must have exact owner-read/write mode 0600",
        "os.fsync(handle.fileno())",
        "os.link(temporary, destination, follow_symlinks=False)",
        "qualification evidence already exists",
        "fsync_directory(parent)",
        "source changed while publishing the qualification receipt",
        "remove_published_evidence(published, \"qualification receipt\")",
    ]:
        require(marker in source, f"local qualification issuer omits {marker}")
    for forbidden in [
        "os.replace(temporary, destination)",
        "shell=True",
        "os.system(",
        "requests.",
        "urllib.request",
        "eval(",
        "exec(",
    ]:
        require(forbidden not in source, f"local qualification issuer contains {forbidden}")


def check_tests() -> None:
    source = TESTS.read_text(encoding="utf-8")
    require(source.count("def test_") >= 18, "local qualification issuer needs eighteen tests")
    for name in [
        "test_clean_snapshot_and_passed_receipt_are_exact_and_owner_private",
        "test_tracked_byte_drift_prevents_receipt_publication",
        "test_commit_drift_prevents_receipt_publication",
        "test_untracked_status_drift_prevents_receipt_publication",
        "test_dirty_source_requires_opt_in_and_downgrades_passed_status",
        "test_assume_unchanged_cannot_hide_head_divergent_bytes",
        "test_executable_mode_drift_cannot_hide_behind_core_filemode_false",
        "test_inherited_git_directory_override_is_ignored",
        "test_empty_tracked_file_is_bound_without_false_failure",
        "test_static_only_accepts_only_a_passing_canonical_prefix",
        "test_incomplete_or_reordered_passed_gates_are_rejected",
        "test_failed_receipt_accepts_a_failed_canonical_prefix",
        "test_snapshot_and_receipt_outputs_are_create_only",
        "test_destination_race_cannot_overwrite_an_existing_file",
        "test_private_run_directory_and_gate_journal_modes_are_required",
        "test_evidence_files_must_share_one_private_run_directory",
        "test_tracked_symbolic_link_is_rejected",
        "test_post_publication_source_change_removes_receipt",
    ]:
        require(f"def {name}" in source, f"local qualification tests omit {name}")


def check_wrapper() -> None:
    source = WRAPPER.read_text(encoding="utf-8")
    for marker in [
        "umask 077",
        'RECEIPT_WRITER="$ROOT/scripts/write_local_qualification_receipt.py"',
        'LOCAL_RECEIPT_CONTRACT="$ROOT/architecture/local_qualification_receipt_v1.json"',
        'SOURCE_SNAPSHOT="$OUT_DIR/source-snapshot.json"',
        'RECEIPT="$OUT_DIR/qualification-receipt.json"',
        'mkdir -m 0700 "$OUT_DIR"',
        'chmod 0600 "$GATES_FILE"',
        'python3 "$RECEIPT_WRITER" begin',
        'python3 "$RECEIPT_WRITER" finish',
        'begin_arguments+=(--allow-dirty)',
        '"--requested-status" "$final_status"',
        "write_receipt static_only",
        "write_receipt passed",
        "scripts/check_local_qualification_receipt.py",
        "scripts/test_local_qualification_receipt.py",
        "scripts/check_implementation_status.py",
        "scripts/test_implementation_status.py",
    ]:
        require(marker in source, f"qualify_local.sh omits {marker}")
    for forbidden in [
        "FINAL_STATUS=\"$final_status\" GATES_FILE=",
        "'digests':{",
        "out.write_text(json.dumps(receipt",
        "record rust-toolchain skipped",
        'mkdir -p "$OUT_DIR"',
    ]:
        require(forbidden not in source, f"qualify_local.sh retains obsolete receipt path {forbidden}")


def check_gate_and_server_binding() -> None:
    verify = VERIFY.read_text(encoding="utf-8")
    for marker in [
        "python3 scripts/check_local_qualification_receipt.py",
        "python3 scripts/test_local_qualification_receipt.py",
        "python3 scripts/check_implementation_status.py",
        "python3 scripts/test_implementation_status.py",
        "scripts/write_local_qualification_receipt.py",
        "scripts/check_local_qualification_receipt.py",
        "scripts/test_local_qualification_receipt.py",
        "scripts/check_implementation_status.py",
        "scripts/test_implementation_status.py",
    ]:
        require(marker in verify, f"verify.sh omits local receipt marker {marker}")

    contract = json.loads(SERVER_CONTRACT.read_text(encoding="utf-8"))
    mapping = contract.get("source_binding", {}).get("required_source_digests", {})
    expected = {
        "local_qualification_contract": "architecture/local_qualification_receipt_v1.json",
        "local_qualification_writer": "scripts/write_local_qualification_receipt.py",
        "local_qualification_checker": "scripts/check_local_qualification_receipt.py",
        "local_qualification_tests": "scripts/test_local_qualification_receipt.py",
        "local_qualification_wrapper": "scripts/qualify_local.sh",
        "implementation_status_contract": "architecture/implementation_status_v1.json",
        "implementation_status_checker": "scripts/check_implementation_status.py",
        "implementation_status_tests": "scripts/test_implementation_status.py",
        "verification_wrapper": "scripts/verify.sh",
    }
    for name, relative in expected.items():
        require(mapping.get(name) == relative, f"server receipt source map omits {name}")

    gates = contract.get("source_binding", {}).get("required_local_qualification_gates", [])
    for gate in [
        "local-qualification-receipt",
        "implementation-status",
        "local-qualification-receipt-tests",
        "implementation-status-tests",
    ]:
        require(gate in gates, f"server receipt gate contract omits {gate}")

    verifier = SERVER_VERIFIER.read_text(encoding="utf-8")
    for name, relative in expected.items():
        marker = f'"{name}": "{relative}"'
        require(marker in verifier, f"server receipt verifier omits {marker}")
    for gate in [
        "local-qualification-receipt",
        "implementation-status",
        "local-qualification-receipt-tests",
        "implementation-status-tests",
    ]:
        require(f'"{gate}"' in verifier, f"server receipt verifier omits gate {gate}")
    for marker in [
        '"head_equivalent"',
        '"tree"',
        '"snapshot_digest"',
        "local qualification receipt is not bound to HEAD-equivalent source",
    ]:
        require(marker in verifier, f"server receipt verifier omits {marker}")


def check_docs() -> None:
    source = DOC.read_text(encoding="utf-8").lower()
    for marker in [
        "two-phase source snapshot",
        "development_dirty",
        "static_only",
        "complete tracked-file digest inventory",
        "head-equivalent",
        "assume-unchanged",
        "executable-bit",
        "0700",
        "0600",
        "atomic no-replace",
        "source changed during qualification",
        "create-only",
        "does not establish",
    ]:
        require(marker in source, f"local qualification documentation omits {marker}")


def main() -> int:
    try:
        check_contract()
        check_writer()
        check_tests()
        check_wrapper()
        check_gate_and_server_binding()
        check_docs()
    except (OSError, SyntaxError, json.JSONDecodeError, ContractError) as exc:
        print(f"local qualification receipt: FAIL: {exc}", file=sys.stderr)
        return 1
    print(
        "local qualification receipt: PASS "
        "(HEAD-exact source, private custody, no-replace publication, exact gates, explicit downgrade)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
