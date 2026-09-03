#!/usr/bin/env python3
"""Validate tracked-source custody from local gates through server qualification."""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
FILES = {
    "contract": "architecture/release_source_custody_v1.json",
    "local_contract": "architecture/local_qualification_receipt_v1.json",
    "local_issuer": "scripts/write_local_qualification_receipt.py",
    "local_tests": "scripts/test_local_qualification_receipt.py",
    "local_wrapper": "scripts/qualify_local.sh",
    "server_contract": "architecture/live_server_binary_receipt_v1.json",
    "server_verifier": "scripts/verify_live_server_binary_receipt.py",
    "server_verifier_tests": "scripts/test_live_server_binary_receipt.py",
    "server_qualifier": "scripts/qualify_live_server_binary.sh",
    "server_qualifier_tests": "scripts/test_qualify_live_server_binary.py",
    "verify": "scripts/verify.sh",
    "docs": "docs/LOCAL_QUALIFICATION_AND_RELEASE.md",
}


class CustodyError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise CustodyError(message)


def source(name: str) -> str:
    return (ROOT / FILES[name]).read_text(encoding="utf-8")


def value(name: str) -> dict[str, Any]:
    parsed = json.loads(source(name))
    require(isinstance(parsed, dict), f"{FILES[name]} must be a JSON object")
    return parsed


def markers(name: str, required: list[str]) -> str:
    text = source(name)
    for marker in required:
        require(marker in text, f"{FILES[name]} omits {marker}")
    return text


def tests(name: str, minimum: int, required: list[str]) -> None:
    text = source(name)
    require(text.count("def test_") >= minimum, f"{FILES[name]} needs at least {minimum} tests")
    for item in required:
        require(f"def {item}" in text, f"{FILES[name]} omits {item}")


def check_contract() -> None:
    contract = value("contract")
    require(contract.get("schema_version") == "dfmcp.release-source-custody/1", "custody schema drifted")
    require(contract.get("status") == "normative_tracked_source_and_artifact_custody", "custody status drifted")
    local = contract.get("local_qualification", {})
    expected_local = {
        "contract": FILES["local_contract"],
        "issuer": FILES["local_issuer"],
        "wrapper": FILES["local_wrapper"],
        "checker": "scripts/check_local_qualification_receipt.py",
        "adversarial_tests": FILES["local_tests"],
        "receipt_schema": "dfmcp.qualification-receipt.v1",
        "clean_success_status": "passed",
        "non_release_statuses": ["development_dirty", "static_only", "failed"],
    }
    require(local == expected_local, "local qualification custody mapping drifted")
    server = contract.get("server_artifact", {})
    expected_server = {
        "contract": FILES["server_contract"],
        "verifier": FILES["server_verifier"],
        "qualifier": FILES["server_qualifier"],
        "verifier_tests": FILES["server_verifier_tests"],
        "qualifier_tests": FILES["server_qualifier_tests"],
        "receipt_schema": "dfmcp.live-server-binary-qualification/1",
    }
    require(server == expected_server, "server artifact custody mapping drifted")
    tracked = contract.get("tracked_source", {})
    for field in [
        "complete_tracked_inventory_required",
        "working_tree_bytes_must_match_head_blobs",
        "working_tree_executable_semantics_must_match_head_on_unix",
        "porcelain_status_must_remain_exact",
        "inherited_git_environment_is_sanitized",
        "assume_unchanged_and_skip_worktree_must_not_hide_drift",
        "empty_tracked_files_are_bound",
        "symbolic_links_and_gitlinks_are_rejected",
    ]:
        require(tracked.get(field) is True, f"tracked-source contract weakens {field}")
    require(tracked.get("git_object_format") == "sha1", "Git object format drifted")
    require(tracked.get("hash_algorithm") == "sha256", "source hash drifted")
    custody = contract.get("custody", {})
    require(custody.get("run_directory_mode") == "0700", "run directory mode drifted")
    require(custody.get("evidence_file_mode") == "0600", "evidence mode drifted")
    require(custody.get("effective_user_ownership_required_when_available") is True, "owner check weakened")
    require(custody.get("final_component_symbolic_links_allowed") is False, "evidence symlinks allowed")
    require(custody.get("publication") == "same-directory hard-link no-replace after file fsync", "publication drifted")
    require(custody.get("parent_directory_fsync_required") is True, "directory fsync weakened")
    require(custody.get("invalid_published_evidence_removed_and_absence_verified") is True, "cleanup weakened")
    require(len(contract.get("verification_sequence", [])) == 8, "custody sequence drifted")
    gates = contract.get("gate_wiring", {})
    require(gates.get("static_checker_gate") == "live-server-artifact-admission", "static gate drifted")
    require(gates.get("adversarial_test_gate") == "live-server-binary-qualification-tests", "test gate drifted")
    require(gates.get("top_level_verification") == FILES["verify"], "top-level gate drifted")
    authority = contract.get("authority", {})
    for field in ["executes_live_bridge", "creates_compatibility_entry", "advances_monotonic_floor", "authorizes_process"]:
        require(authority.get(field) is False, f"custody contract grants {field}")
    require(authority.get("grants_capabilities") == [], "custody grants capability")
    require(authority.get("mutation_capabilities") == [], "custody grants mutation")


def check_implementation() -> None:
    local_contract = value("local_contract")
    for field in [
        "working_tree_bytes_must_match_head_blobs_for_clean_passed",
        "working_tree_executable_semantics_must_match_head_on_unix",
        "git_environment_must_be_sanitized",
    ]:
        require(local_contract.get("source", {}).get(field) is True, f"local contract weakens {field}")
    publication = local_contract.get("publication", {})
    require(publication.get("atomic_no_replace_hard_link") is True, "local publication is replaceable")
    require(publication.get("unix_run_directory_mode") == "0700", "local run mode drifted")
    require(publication.get("receipt_mode") == "0600", "local receipt mode drifted")

    issuer = markers(
        "local_issuer",
        [
            "def git_blob_object_id(",
            "def sanitized_git_environment(",
            "def collect_source_snapshot(",
            "def validate_private_evidence_directory(",
            "def write_atomic_create_only(",
            '"status", "--porcelain=v1", "-z", "--untracked-files=all"',
            '"ls-tree", "-rz", "--full-tree"',
            "git_blob_object_id(stable.content)",
            "allow_empty=True",
            '"head_equivalent": head_equivalent',
            "qualification evidence directory must have exact owner-only mode 0700",
            "must have exact owner-read/write mode 0600",
            "os.link(temporary, destination, follow_symlinks=False)",
            "source changed while publishing the qualification receipt",
        ],
    )
    require("os.replace(temporary, destination)" not in issuer, "local issuer can replace evidence")
    tests(
        "local_tests",
        18,
        [
            "test_assume_unchanged_cannot_hide_head_divergent_bytes",
            "test_executable_mode_drift_cannot_hide_behind_core_filemode_false",
            "test_inherited_git_directory_override_is_ignored",
            "test_empty_tracked_file_is_bound_without_false_failure",
            "test_destination_race_cannot_overwrite_an_existing_file",
            "test_private_run_directory_and_gate_journal_modes_are_required",
            "test_post_publication_source_change_removes_receipt",
        ],
    )
    markers(
        "local_wrapper",
        [
            "umask 077",
            "export PYTHONDONTWRITEBYTECODE=1",
            'RECEIPT_WRITER="$ROOT/scripts/write_local_qualification_receipt.py"',
            'mkdir -m 0700 "$OUT_DIR"',
            'chmod 0600 "$GATES_FILE"',
            'python3 "$RECEIPT_WRITER" begin',
            'python3 "$RECEIPT_WRITER" finish',
        ],
    )

    server_contract = value("server_contract")
    binding = server_contract.get("source_binding", {})
    require(binding.get("requires_clean_dfmcp_source") is True, "server contract accepts dirty source")
    require(binding.get("requires_passing_local_qualification_receipt") is True, "server contract omits local receipt")
    markers(
        "server_verifier",
        [
            "def read_private_local_receipt(",
            "def collect_head_equivalent_source_inventory(",
            "def validate_local_qualification_receipt(",
            "directory must have exact owner-only mode 0700",
            "must have exact owner-read/write mode 0600",
            "local qualification receipt is not bound to HEAD-equivalent source",
            "local qualification receipt digest inventory differs from current HEAD-equivalent source",
            "local qualification source bytes differ from HEAD",
            "local qualification source executable semantics differ from HEAD",
            "server receipt source inventory changed after local receipt verification",
        ],
    )
    tests(
        "server_verifier_tests",
        22,
        [
            "test_local_receipt_requires_private_directory_and_file_modes",
            "test_non_head_equivalent_local_receipt_is_rejected",
            "test_local_receipt_inventory_path_or_digest_drift_is_rejected",
            "test_assume_unchanged_cannot_hide_source_drift_after_local_qualification",
            "test_core_filemode_false_cannot_hide_source_mode_drift",
        ],
    )
    qualifier = markers(
        "server_qualifier",
        [
            "umask 077",
            "export PYTHONDONTWRITEBYTECODE=1",
            '[[ ! -e "$OUT_DIR" && ! -L "$OUT_DIR" ]]',
            'mkdir -m 0700 "$OUT_DIR"',
            'mkdir -m 0700 "$OUT_DIR/logs"',
            "validate_local_receipt",
            'info "Revalidating source after build and executable checks"',
            "os.link(temporary,destination,follow_symlinks=False)",
            "cleanup_invalid_evidence",
            "Independently re-verifying the receipt, source inventory, and opened binary inode",
        ],
    )
    require(qualifier.count("validate_local_receipt") >= 3, "server qualifier lacks repeated replay")
    require("os.replace(temporary,destination)" not in qualifier, "server qualifier can replace evidence")
    tests(
        "server_qualifier_tests",
        9,
        [
            "test_existing_output_directory_is_rejected_before_build",
            "test_assume_unchanged_source_drift_is_rejected_before_build",
            "test_non_private_local_receipt_is_rejected_before_build",
            "test_source_drift_during_build_prevents_receipt",
        ],
    )


def check_wiring_and_docs() -> None:
    local = source("local_wrapper")
    for marker in [
        "architecture/release_source_custody_v1.json",
        "python3 scripts/check_release_source_custody.py",
        "python3 scripts/test_release_source_custody.py",
    ]:
        require(marker in local, f"local qualification omits {marker}")
    verify = source("verify")
    for marker in [
        "architecture/release_source_custody_v1.json",
        "python3 scripts/check_release_source_custody.py",
        "python3 scripts/test_release_source_custody.py",
        "scripts/check_release_source_custody.py",
        "scripts/test_release_source_custody.py",
    ]:
        require(marker in verify, f"top-level verification omits {marker}")
    docs = source("docs").lower()
    for marker in [
        "two-phase source snapshot",
        "complete tracked-file digest inventory",
        "head-equivalent",
        "assume-unchanged",
        "executable-bit",
        "0700",
        "0600",
        "no-replace",
        "source changed during qualification",
        "does not establish",
    ]:
        require(marker in docs, f"qualification documentation omits {marker}")


def main() -> int:
    try:
        check_contract()
        check_implementation()
        check_wiring_and_docs()
    except (OSError, json.JSONDecodeError, CustodyError) as exc:
        print(f"release source custody: FAIL: {exc}", file=sys.stderr)
        return 1
    print("release source custody: PASS (HEAD-equivalent inventory, private no-replace receipts, and independent replay)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
