#!/usr/bin/env python3
"""Validate the canonical source-bundle creation, verification, and gate wiring."""

from __future__ import annotations

import ast
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = ROOT / "architecture/source_bundle_v1.json"
STABLE_READER = ROOT / "scripts/read_stable_repository_file.py"
CREATOR = ROOT / "scripts/create_source_bundle.py"
WRAPPER = ROOT / "scripts/create_source_bundle.sh"
VERIFIER = ROOT / "scripts/verify_source_bundle.py"
TESTS = ROOT / "scripts/test_source_bundle.py"
VERIFY = ROOT / "scripts/verify.sh"
QUALIFY = ROOT / "scripts/qualify_local.sh"
SERVER_CONTRACT = ROOT / "architecture/live_server_binary_receipt_v1.json"
DOC = ROOT / "docs/SOURCE_BUNDLE.md"


class ContractError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def functions(path: Path) -> set[str]:
    tree = ast.parse(path.read_text(encoding="utf-8"))
    return {
        node.name
        for node in ast.walk(tree)
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
    }


def check_contract() -> None:
    value = json.loads(CONTRACT.read_text(encoding="utf-8"))
    require(value.get("schema_version") == "dfmcp.source-bundle-contract/1", "contract schema drifted")
    require(value.get("manifest_schema") == "dfmcp.source-bundle/1", "manifest schema drifted")
    require(
        value.get("status") == "normative_release_source_bundle_contract",
        "contract status drifted",
    )
    archive = value.get("archive", {})
    require(archive.get("format") == "ustar_or_pax_tar_from_git_archive", "archive format drifted")
    require(archive.get("hash_algorithm") == "sha256", "archive hash algorithm drifted")
    require(archive.get("maximum_bytes") == 268435456, "archive byte bound drifted")
    require(archive.get("maximum_entries") == 65536, "archive member bound drifted")
    require(archive.get("maximum_entry_bytes") == 67108864, "archive entry bound drifted")
    require(
        archive.get("maximum_total_content_bytes") == 536870912,
        "archive total-content bound drifted",
    )
    require(
        archive.get("allowed_entry_types") == ["directory", "regular_file"],
        "archive entry types drifted",
    )
    for field in [
        "symbolic_links_allowed",
        "hard_links_allowed",
        "special_files_allowed",
        "duplicate_paths_allowed",
        "absolute_paths_allowed",
        "parent_traversal_allowed",
    ]:
        require(archive.get(field) is False, f"archive contract permits {field}")
    source = value.get("source", {})
    require(source.get("must_be_clean_git_commit") is True, "clean source is not required")
    require(source.get("submodules_allowed") is False, "submodules are permitted")
    require(source.get("symbolic_links_allowed") is False, "source symlinks are permitted")
    require(source.get("tracked_regular_files_only") is True, "non-regular source is permitted")
    require(source.get("entry_order") == "strict_utf8_path_byte_order", "entry order drifted")
    require(source.get("file_modes") == ["100644", "100755"], "source modes drifted")
    authority = value.get("authority", {})
    require(authority.get("executes_project_code") is False, "contract executes project code")
    require(authority.get("modifies_source") is False, "contract modifies source")
    require(authority.get("network_access") is False, "contract grants network access")
    require(authority.get("grants_capabilities") == [], "contract grants capability")
    require(authority.get("mutation_capabilities") == [], "contract grants mutation")


def check_stable_reader() -> None:
    names = functions(STABLE_READER)
    source = STABLE_READER.read_text(encoding="utf-8")
    require("read_stable_regular_file" in names, "stable reader function is missing")
    for marker in [
        "O_NOFOLLOW",
        "O_CLOEXEC",
        "os.lstat(path)",
        "os.fstat(descriptor)",
        "st_dev",
        "st_ino",
        "st_mtime_ns",
        "st_ctime_ns",
        "exceeded its byte bound while being read",
        "changed between path inspection and open",
        "changed while being read",
    ]:
        require(marker in source, f"stable reader omits {marker}")


def check_creator() -> None:
    names = functions(CREATOR)
    source = CREATOR.read_text(encoding="utf-8")
    for name in [
        "require_clean_source",
        "validate_output_location",
        "stream_git_archive",
        "build_manifest",
        "create_bundle",
    ]:
        require(name in names, f"source bundle creator omits {name}")
    for marker in [
        '"tar.umask=0022"',
        '"--format=tar"',
        "tempfile.mkdtemp",
        "verifier.verify(",
        "source,\n            True,",
        "os.replace(staging, destination)",
        "fsync_directory(destination.parent)",
        "source bundle output directory already exists",
        '"mutation_capabilities": [],',
    ]:
        require(marker in source, f"source bundle creator omits {marker}")
    require(
        source.index("verifier.verify(") < source.index("os.replace(staging, destination)"),
        "source bundle is published before independent verification",
    )
    require("git archive" not in source, "creator must use argument-vector Git invocation")


def check_wrapper() -> None:
    source = WRAPPER.read_text(encoding="utf-8")
    require('CREATOR="$ROOT/scripts/create_source_bundle.py"' in source, "wrapper omits creator")
    require('python3 "$CREATOR"' in source, "wrapper does not delegate to creator")
    require('[[ "$1" = /* ]]' in source, "wrapper does not reject relative output paths")
    for forbidden in [
        "git archive",
        "source-bundle-manifest.$$.tmp",
        "mv -- \"$ARCHIVE_TMP\"",
        "Building canonical source manifest",
    ]:
        require(forbidden not in source, f"wrapper reimplements creator logic: {forbidden}")


def check_verifier() -> None:
    names = functions(VERIFIER)
    source = VERIFIER.read_text(encoding="utf-8")
    for name in [
        "duplicate_rejecting_object",
        "bounded_tree",
        "load_contract",
        "validate_manifest",
        "validate_tar_member_name",
        "expected_archive_members",
        "validate_pax_headers",
        "verify_archive",
        "git_entries",
        "verify_checkout",
        "verify",
        "write_atomic",
    ]:
        require(name in names, f"source bundle verifier omits {name}")
    for marker in [
        "source archive repeats canonical member",
        "member order or canonical directory/file set differs",
        "source archive PAX comment is not the exact commit identity",
        "source archive contains unsupported PAX metadata",
        "has noncanonical owner names",
        "members do not share one deterministic commit time",
        "mode differs from the manifest",
        "nonzero trailing data",
        "source checkout is not clean",
        '"clean_required": require_clean',
        "source bundle verification output already exists",
    ]:
        require(marker in source, f"source bundle verifier omits {marker}")
    for forbidden in ["extractall(", ".extract(", "shell=True", "requests.", "urllib.request"]:
        require(forbidden not in source, f"source bundle verifier contains forbidden path {forbidden}")


def check_tests() -> None:
    source = TESTS.read_text(encoding="utf-8")
    require(source.count("def test_") >= 9, "source bundle needs at least nine focused tests")
    for name in [
        "test_clean_bundle_round_trip_is_deterministic",
        "test_dirty_source_and_existing_destination_fail_without_replacement",
        "test_tracked_symbolic_link_is_rejected_without_publication",
        "test_tracked_gitlink_is_rejected_without_publication",
        "test_reordered_and_semantically_duplicate_members_are_rejected",
        "test_noncanonical_member_metadata_is_rejected",
        "test_links_unmanifested_content_and_trailing_payload_are_rejected",
        "test_manifest_and_checkout_tampering_are_rejected",
        "test_verification_output_is_create_only",
    ]:
        require(f"def {name}" in source, f"source bundle tests omit {name}")
    for marker in [
        '"GIT_AUTHOR_DATE": "2001-02-03T04:05:06Z"',
        '"GIT_COMMITTER_DATE": "2001-02-03T04:05:06Z"',
        'f"160000,{nested_commit},vendor/submodule"',
        "tarfile.SYMTYPE",
        'trailing=b"evil"',
        '"SCHILY.xattr.user.test"',
    ]:
        require(marker in source, f"source bundle tests omit adversary {marker}")


def check_gate_wiring() -> None:
    verify = VERIFY.read_text(encoding="utf-8")
    for marker in [
        "python3 scripts/check_source_bundle.py",
        "python3 scripts/test_source_bundle.py",
        "scripts/create_source_bundle.py",
        "scripts/verify_source_bundle.py",
        "scripts/test_source_bundle.py",
        "scripts/check_source_bundle.py",
    ]:
        require(marker in verify, f"verify.sh omits source bundle gate marker {marker}")

    qualify = QUALIFY.read_text(encoding="utf-8")
    for marker in [
        "python3 scripts/validate_repo.py && python3 scripts/check_source_bundle.py",
        "python3 scripts/test_repository_integrity.py && python3 scripts/test_read_stable_repository_file.py && python3 scripts/test_source_bundle.py",
        "'source_bundle_contract':digest(root/'architecture/source_bundle_v1.json')",
        "'source_bundle_stable_reader':digest(root/'scripts/read_stable_repository_file.py')",
        "'source_bundle_creator':digest(root/'scripts/create_source_bundle.py')",
        "'source_bundle_wrapper':digest(root/'scripts/create_source_bundle.sh')",
        "'source_bundle_verifier':digest(root/'scripts/verify_source_bundle.py')",
        "'source_bundle_checker':digest(root/'scripts/check_source_bundle.py')",
        "'source_bundle_tests':digest(root/'scripts/test_source_bundle.py')",
        "'source_bundle_documentation':digest(root/'docs/SOURCE_BUNDLE.md')",
    ]:
        require(marker in qualify, f"qualify_local.sh omits source bundle marker {marker}")

    server_contract = json.loads(SERVER_CONTRACT.read_text(encoding="utf-8"))
    binding = server_contract.get("source_binding", {})
    gates = binding.get("required_local_qualification_gates", [])
    require(
        "source-bundle-contract" not in gates and "source-bundle-tests" not in gates,
        "source packaging incorrectly widened the live-server receipt gate schema",
    )
    digests = binding.get("required_source_digests", {})
    require(
        not any(name.startswith("source_bundle_") for name in digests),
        "source packaging incorrectly widened live-server executable identity",
    )


def check_docs() -> None:
    source = DOC.read_text(encoding="utf-8").lower()
    for marker in [
        "exact clean git commit",
        "tar.umask=0022",
        "sibling staging directory",
        "published atomically",
        "semantic duplicate",
        "pax",
        "does not prove compilation",
        "does not prove",
        "release packaging boundary",
        "live-server binary receipt",
    ]:
        require(marker in source, f"source bundle documentation omits {marker}")


def main() -> int:
    try:
        check_contract()
        check_stable_reader()
        check_creator()
        check_wrapper()
        check_verifier()
        check_tests()
        check_gate_wiring()
        check_docs()
    except (OSError, SyntaxError, json.JSONDecodeError, ContractError) as exc:
        print(f"source bundle contract: FAIL: {exc}", file=sys.stderr)
        return 1
    print(
        "source bundle contract: PASS "
        "(deterministic, hostile-safe, clean-source-bound, and qualification-wired)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
