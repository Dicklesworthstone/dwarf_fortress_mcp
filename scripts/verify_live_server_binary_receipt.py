#!/usr/bin/env python3
"""Verify one source-bound release server receipt and its exact binary bytes."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import stat
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Any, NoReturn

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONTRACT = ROOT / "architecture/live_server_binary_receipt_v1.json"
RECEIPT_SCHEMA = "dfmcp.live-server-binary-qualification/1"
LOCAL_RECEIPT_SCHEMA = "dfmcp.qualification-receipt.v1"
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
MAX_JSON_BYTES = 128 * 1024 * 1024
MAX_HASHED_FILE_BYTES = 256 * 1024 * 1024
MAX_SOURCE_ENTRIES = 65_536
MAX_SOURCE_TOTAL_BYTES = 1024 * 1024 * 1024
MAX_STRING_BYTES = 16 * 1024
MAX_COLLECTION_ITEMS = 65_536
MAX_DEPTH = 64

EXPECTED_LOCAL_QUALIFICATION_GATES = [
    "repository-integrity",
    "local-qualification-receipt",
    "implementation-status",
    "static-contracts",
    "agent-contract",
    "dfhack-read-bridge-contract",
    "bridge-auth-order",
    "live-mcp-contract",
    "compiled-live-read-stack-contract",
    "live-acceptance-contract",
    "live-capture-plan",
    "live-compatibility-registry",
    "live-compatibility-resolution",
    "live-compatibility-floor",
    "live-admission-doctor",
    "live-server-artifact-admission",
    "dependency-policy",
    "repository-integrity-tests",
    "local-qualification-receipt-tests",
    "implementation-status-tests",
    "live-acceptance-tests",
    "live-acceptance-journal-tests",
    "live-acceptance-secret-scanner-tests",
    "live-capture-guidance-tests",
    "live-compatibility-promotion-tests",
    "live-compatibility-resolution-tests",
    "live-compatibility-floor-tests",
    "live-admission-doctor-tests",
    "live-server-binary-qualification-tests",
    "live-server-binary-receipt-tests",
    "admitted-live-launcher-tests",
    "python-syntax",
    "shell-syntax",
    "cargo-metadata",
    "rustfmt",
    "clippy",
    "tests",
    "release-tests",
    "rustdoc",
    "contract",
    "doctor",
    "demo",
    "live-probe-help",
]
EXPECTED_SOURCE_DIGESTS = {
    "cargo_lock": "Cargo.lock",
    "workspace_manifest": "Cargo.toml",
    "binary_main": "crates/dwarf-fortress-mcp/src/main.rs",
    "binary_admission_tests": "crates/dwarf-fortress-mcp/tests/live_admission.rs",
    "mcp_crate_root": "crates/dfmcp-mcp/src/lib.rs",
    "mcp_admission": "crates/dfmcp-mcp/src/admission.rs",
    "mcp_agent_turn": "crates/dfmcp-mcp/src/agent_turn.rs",
    "mcp_live_server": "crates/dfmcp-mcp/src/live_server.rs",
    "adapter_live_connect": "crates/dfmcp-adapter/src/live_connect.rs",
    "adapter_live_bootstrap": "crates/dfmcp-adapter/src/live_bootstrap.rs",
    "adapter_live_observation": "crates/dfmcp-adapter/src/live_observation.rs",
    "adapter_live_projection": "crates/dfmcp-adapter/src/live_projection.rs",
    "stable_repository_reader": "scripts/read_stable_repository_file.py",
    "local_qualification_contract": "architecture/local_qualification_receipt_v1.json",
    "local_qualification_writer": "scripts/write_local_qualification_receipt.py",
    "local_qualification_checker": "scripts/check_local_qualification_receipt.py",
    "local_qualification_tests": "scripts/test_local_qualification_receipt.py",
    "local_qualification_wrapper": "scripts/qualify_local.sh",
    "implementation_status_contract": "architecture/implementation_status_v1.json",
    "implementation_status_checker": "scripts/check_implementation_status.py",
    "implementation_status_tests": "scripts/test_implementation_status.py",
    "verification_wrapper": "scripts/verify.sh",
    "compatibility_registry": "architecture/live_compatibility_registry_v1.json",
    "compatibility_resolver": "scripts/resolve_live_compatibility.py",
    "compatibility_floor_contract": "architecture/live_compatibility_floor_v1.json",
    "compatibility_floor": "scripts/live_compatibility_floor.py",
    "compatibility_floor_checker": "scripts/check_live_compatibility_floor.py",
    "compatibility_floor_tests": "scripts/test_live_compatibility_floor.py",
    "admission_doctor_contract": "architecture/live_admission_doctor_v1.json",
    "admission_ticket_contract": "architecture/live_admission_ticket_v2.json",
    "admission_doctor": "scripts/doctor_live_admission.py",
    "admission_doctor_checker": "scripts/check_live_admission_doctor.py",
    "admission_doctor_tests": "scripts/test_doctor_live_admission.py",
    "artifact_contract": "architecture/live_server_binary_receipt_v1.json",
    "artifact_qualification": "scripts/qualify_live_server_binary.sh",
    "artifact_qualification_tests": "scripts/test_qualify_live_server_binary.py",
    "artifact_verifier": "scripts/verify_live_server_binary_receipt.py",
    "artifact_checker": "scripts/check_live_server_artifact.py",
    "artifact_verifier_tests": "scripts/test_live_server_binary_receipt.py",
    "admitted_launcher": "scripts/serve_admitted_live.py",
    "admitted_launcher_tests": "scripts/test_admitted_live_launcher.py",
    "admission_ticket_tests": "scripts/test_live_admission_ticket.py",
}


class VerificationError(ValueError):
    pass


@dataclass(frozen=True)
class OpenBinary:
    descriptor: int
    path: Path
    sha256: str
    size: int
    device: int
    inode: int
    mode: int
    owner_uid: int


def fail(message: str) -> NoReturn:
    raise VerificationError(message)


def duplicate_rejecting_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    output: dict[str, Any] = {}
    for key, value in pairs:
        if key in output:
            fail(f"JSON object repeats key {key!r}")
        output[key] = value
    return output


def canonical_json(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode(
        "utf-8"
    )


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def git_blob_object_id(value: bytes) -> str:
    header = f"blob {len(value)}\0".encode("ascii")
    try:
        digest = hashlib.sha1(usedforsecurity=False)
    except TypeError:
        digest = hashlib.sha1()
    digest.update(header)
    digest.update(value)
    return digest.hexdigest()


def same_identity(left: os.stat_result, right: os.stat_result) -> bool:
    return (
        left.st_dev == right.st_dev
        and left.st_ino == right.st_ino
        and left.st_size == right.st_size
        and left.st_mode == right.st_mode
        and left.st_uid == right.st_uid
        and left.st_gid == right.st_gid
        and left.st_mtime_ns == right.st_mtime_ns
        and left.st_ctime_ns == right.st_ctime_ns
    )


def open_stable_regular(
    path: Path,
    maximum_bytes: int,
    label: str,
    *,
    allow_empty: bool = False,
) -> tuple[int, os.stat_result]:
    raw = os.fspath(path)
    if not raw or len(os.fsencode(raw)) > 4096:
        fail(f"{label} path is empty or exceeds its byte bound")
    if any(ord(character) < 0x20 for character in raw):
        fail(f"{label} path contains a control character")
    if not hasattr(os, "O_NOFOLLOW"):
        fail("this platform cannot enforce no-follow artifact opening")
    try:
        before = os.lstat(path)
    except OSError as exc:
        fail(f"cannot inspect {label}: {exc}")
    if stat.S_ISLNK(before.st_mode) or not stat.S_ISREG(before.st_mode):
        fail(f"{label} must be a regular non-symbolic-link file")
    minimum = 0 if allow_empty else 1
    if before.st_size < minimum or before.st_size > maximum_bytes:
        fail(f"{label} must contain {minimum}..={maximum_bytes} bytes")
    flags = os.O_RDONLY | os.O_NOFOLLOW
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    try:
        descriptor = os.open(path, flags)
    except OSError as exc:
        fail(f"cannot open {label} without following symbolic links: {exc}")
    try:
        metadata = os.fstat(descriptor)
        if not same_identity(before, metadata):
            fail(f"{label} changed between path inspection and open")
        return descriptor, metadata
    except BaseException:
        os.close(descriptor)
        raise


def read_stable_bytes_with_metadata(
    path: Path,
    label: str,
    maximum_bytes: int = MAX_JSON_BYTES,
    *,
    allow_empty: bool = False,
) -> tuple[bytes, str, os.stat_result]:
    descriptor, before = open_stable_regular(
        path,
        maximum_bytes,
        label,
        allow_empty=allow_empty,
    )
    try:
        digest = hashlib.sha256()
        chunks: list[bytes] = []
        total = 0
        while True:
            remaining = maximum_bytes + 1 - total
            if remaining <= 0:
                fail(f"{label} grew beyond its byte bound while being read")
            chunk = os.read(descriptor, min(1024 * 1024, remaining))
            if not chunk:
                break
            total += len(chunk)
            if total > maximum_bytes:
                fail(f"{label} grew beyond its byte bound while being read")
            digest.update(chunk)
            chunks.append(chunk)
        after = os.fstat(descriptor)
        if not same_identity(before, after) or total != after.st_size:
            fail(f"{label} changed while being read")
        return b"".join(chunks), digest.hexdigest(), after
    finally:
        os.close(descriptor)


def read_bytes_with_digest(
    path: Path,
    label: str,
    maximum_bytes: int = MAX_JSON_BYTES,
    *,
    allow_empty: bool = False,
) -> tuple[bytes, str]:
    raw, digest, _ = read_stable_bytes_with_metadata(
        path,
        label,
        maximum_bytes,
        allow_empty=allow_empty,
    )
    return raw, digest


def sha256_file(path: Path) -> str:
    _, digest = read_bytes_with_digest(
        path,
        "source-bound file",
        MAX_HASHED_FILE_BYTES,
        allow_empty=True,
    )
    return digest


def sha256_descriptor(descriptor: int) -> str:
    before = os.fstat(descriptor)
    duplicate = os.dup(descriptor)
    try:
        os.lseek(duplicate, 0, os.SEEK_SET)
        digest = hashlib.sha256()
        with os.fdopen(duplicate, "rb", closefd=True) as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
        duplicate = -1
        after = os.fstat(descriptor)
        if not same_identity(before, after):
            fail("opened artifact changed while being hashed")
        return digest.hexdigest()
    finally:
        if duplicate >= 0:
            os.close(duplicate)


def bounded_tree(value: Any, path: str = "$", depth: int = 1) -> None:
    if depth > MAX_DEPTH:
        fail(f"{path} exceeds the maximum JSON depth")
    if value is None or isinstance(value, (bool, int)):
        return
    if isinstance(value, float):
        fail(f"{path} contains a noncanonical floating-point value")
    if isinstance(value, str):
        encoded = value.encode("utf-8")
        if len(encoded) > MAX_STRING_BYTES:
            fail(f"{path} exceeds the string byte bound")
        if any(ord(character) < 0x20 and character not in "\t\n\r" for character in value):
            fail(f"{path} contains a forbidden control character")
        return
    if isinstance(value, list):
        if len(value) > MAX_COLLECTION_ITEMS:
            fail(f"{path} exceeds the collection bound")
        for index, item in enumerate(value):
            bounded_tree(item, f"{path}[{index}]", depth + 1)
        return
    if isinstance(value, dict):
        if len(value) > MAX_COLLECTION_ITEMS:
            fail(f"{path} exceeds the object-member bound")
        for key, item in value.items():
            if not isinstance(key, str):
                fail(f"{path} contains a non-string key")
            bounded_tree(key, f"{path}.<key>", depth + 1)
            bounded_tree(item, f"{path}.{key}", depth + 1)
        return
    fail(f"{path} contains unsupported JSON type {type(value).__name__}")


def parse_json_object_bytes(raw: bytes, label: str) -> dict[str, Any]:
    try:
        value = json.loads(
            raw.decode("utf-8"),
            object_pairs_hook=duplicate_rejecting_object,
        )
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        fail(f"cannot parse {label}: {exc}")
    if not isinstance(value, dict):
        fail(f"{label} must be a JSON object")
    bounded_tree(value)
    return value


def read_object_with_digest(
    path: Path,
    label: str,
    maximum_bytes: int = MAX_JSON_BYTES,
) -> tuple[dict[str, Any], str]:
    raw, digest = read_bytes_with_digest(path, label, maximum_bytes)
    return parse_json_object_bytes(raw, label), digest


def read_object(
    path: Path,
    label: str,
    maximum_bytes: int = MAX_JSON_BYTES,
) -> dict[str, Any]:
    value, _ = read_object_with_digest(path, label, maximum_bytes)
    return value


def require_exact_keys(value: dict[str, Any], expected: set[str], path: str) -> None:
    actual = set(value)
    if actual != expected:
        fail(f"{path} fields differ: expected {sorted(expected)}, got {sorted(actual)}")


def require_object(value: Any, path: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail(f"{path} must be an object")
    return value


def require_list(value: Any, path: str) -> list[Any]:
    if not isinstance(value, list):
        fail(f"{path} must be an array")
    return value


def require_string(value: Any, path: str, maximum: int = MAX_STRING_BYTES) -> str:
    if not isinstance(value, str) or not value or len(value.encode("utf-8")) > maximum:
        fail(f"{path} must contain 1..={maximum} UTF-8 bytes")
    if any(ord(character) < 0x20 for character in value):
        fail(f"{path} contains a control character")
    return value


def require_hash(value: Any, path: str) -> str:
    text = require_string(value, path, 64)
    if HEX64.fullmatch(text) is None:
        fail(f"{path} must be a lowercase SHA-256 digest")
    return text


def require_commit(value: Any, path: str) -> str:
    text = require_string(value, path, 40)
    if HEX40.fullmatch(text) is None:
        fail(f"{path} must be a lowercase 40-character Git object ID")
    return text


def require_positive_int(value: Any, path: str, maximum: int | None = None) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        fail(f"{path} must be a positive integer")
    if maximum is not None and value > maximum:
        fail(f"{path} must be <= {maximum}")
    return value


def validate_relative_path(value: Any, path: str) -> str:
    text = require_string(value, path, 4096)
    if "\\" in text or text.startswith("/") or text.endswith("/") or "//" in text:
        fail(f"{path} must be a canonical repository-relative POSIX path")
    candidate = PurePosixPath(text)
    if candidate.is_absolute() or any(part in {"", ".", ".."} for part in candidate.parts):
        fail(f"{path} contains an absolute, empty, dot, or parent component")
    if candidate.as_posix() != text:
        fail(f"{path} is not in canonical relative-path form")
    return text


def load_contract(path: Path) -> dict[str, Any]:
    contract = read_object(
        path,
        "live server binary receipt contract",
        1024 * 1024,
    )
    require_exact_keys(
        contract,
        {
            "schema_version",
            "receipt_schema",
            "status",
            "source_binding",
            "binary",
            "required_executable_checks",
            "authority",
            "claims_not_established",
        },
        "contract",
    )
    if contract.get("schema_version") != "dfmcp.live-server-binary-receipt-contract/1":
        fail("live server binary contract schema is unsupported")
    if contract.get("receipt_schema") != RECEIPT_SCHEMA:
        fail("live server binary receipt schema drifted")
    if contract.get("status") != "normative_runtime_artifact_contract":
        fail("live server binary contract status drifted")

    source_binding = require_object(contract.get("source_binding"), "contract.source_binding")
    require_exact_keys(
        source_binding,
        {
            "requires_clean_dfmcp_source",
            "requires_passing_local_qualification_receipt",
            "local_qualification_receipt_schema",
            "required_local_qualification_gates",
            "required_source_digests",
        },
        "contract.source_binding",
    )
    if source_binding.get("requires_clean_dfmcp_source") is not True:
        fail("binary contract must require clean dfmcp source")
    if source_binding.get("requires_passing_local_qualification_receipt") is not True:
        fail("binary contract must require a passing local qualification receipt")
    if source_binding.get("local_qualification_receipt_schema") != LOCAL_RECEIPT_SCHEMA:
        fail("binary contract local qualification schema drifted")
    gates = require_list(
        source_binding.get("required_local_qualification_gates"),
        "contract.source_binding.required_local_qualification_gates",
    )
    if gates != EXPECTED_LOCAL_QUALIFICATION_GATES:
        fail("binary contract local qualification gate set or order drifted")
    required_digests = require_object(
        source_binding.get("required_source_digests"),
        "contract.source_binding.required_source_digests",
    )
    if required_digests != EXPECTED_SOURCE_DIGESTS:
        fail("binary contract source-digest mapping drifted")
    for name, relative in required_digests.items():
        validate_relative_path(relative, f"contract.source_binding.required_source_digests.{name}")

    binary = require_object(contract.get("binary"), "contract.binary")
    require_exact_keys(
        binary,
        {
            "name",
            "profile",
            "hash_algorithm",
            "must_be_regular_file",
            "symbolic_links_allowed",
            "group_or_world_writable_allowed",
            "maximum_bytes",
        },
        "contract.binary",
    )
    if binary.get("name") != "dwarf-fortress-mcp":
        fail("binary contract names the wrong executable")
    if binary.get("profile") != "release" or binary.get("hash_algorithm") != "sha256":
        fail("binary contract must require a SHA-256 release artifact")
    if binary.get("must_be_regular_file") is not True:
        fail("binary contract must require a regular file")
    if binary.get("symbolic_links_allowed") is not False:
        fail("binary contract must reject symbolic links")
    if binary.get("group_or_world_writable_allowed") is not False:
        fail("binary contract must reject group/world-writable artifacts")
    require_positive_int(binary.get("maximum_bytes"), "contract.binary.maximum_bytes")

    checks = require_list(
        contract.get("required_executable_checks"),
        "contract.required_executable_checks",
    )
    if checks != ["contract", "doctor", "demo"]:
        fail("binary executable-check set or order drifted")
    authority = require_object(contract.get("authority"), "contract.authority")
    require_exact_keys(
        authority,
        {"mode", "mutation_capabilities", "note"},
        "contract.authority",
    )
    if authority.get("mode") != "authenticated_live_read_only":
        fail("binary contract authority mode drifted")
    if authority.get("mutation_capabilities") != []:
        fail("binary contract must carry no mutation capabilities")
    require_string(authority.get("note"), "contract.authority.note", 4096)
    claims = require_list(
        contract.get("claims_not_established"),
        "contract.claims_not_established",
    )
    for index, claim in enumerate(claims):
        require_string(claim, f"contract.claims_not_established[{index}]", 4096)
    return contract


def sanitized_git_environment() -> dict[str, str]:
    environment = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith("GIT_")
    }
    environment["GIT_OPTIONAL_LOCKS"] = "0"
    environment["GIT_CONFIG_NOSYSTEM"] = "1"
    environment["LC_ALL"] = "C"
    return environment


def run_git(source_root: Path, arguments: list[str]) -> bytes:
    try:
        completed = subprocess.run(
            ["git", "-C", os.fspath(source_root), *arguments],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=sanitized_git_environment(),
        )
    except OSError as exc:
        fail(f"cannot execute Git while verifying local qualification source: {exc}")
    if completed.returncode != 0:
        detail = completed.stderr.decode("utf-8", errors="replace")
        fail(f"Git source verification failed: {detail.strip()[:2048]}")
    if len(completed.stdout) > MAX_JSON_BYTES:
        fail("Git source verification output exceeds its byte bound")
    return completed.stdout


def git_text(source_root: Path, arguments: list[str], label: str) -> str:
    try:
        return run_git(source_root, arguments).decode("utf-8").strip()
    except UnicodeDecodeError as exc:
        fail(f"{label} is not valid UTF-8: {exc}")


def executable_semantics_match(git_mode: str, worktree_mode: int) -> bool:
    if os.name != "posix":
        return True
    return bool(worktree_mode & 0o111) == (git_mode == "100755")


def validate_private_evidence_directory(path: Path, label: str) -> Path:
    if not path.is_absolute():
        fail(f"{label} directory must be absolute")
    absolute = Path(os.path.abspath(path))
    try:
        before = os.lstat(absolute)
    except OSError as exc:
        fail(f"cannot inspect {label} directory {absolute}: {exc}")
    if stat.S_ISLNK(before.st_mode) or not stat.S_ISDIR(before.st_mode):
        fail(f"{label} directory must be a real directory, not a symbolic link")
    resolved = absolute.resolve(strict=True)
    try:
        after = os.stat(resolved, follow_symlinks=False)
    except OSError as exc:
        fail(f"cannot inspect resolved {label} directory {resolved}: {exc}")
    if not stat.S_ISDIR(after.st_mode):
        fail(f"resolved {label} path is not a directory")
    if os.name == "posix":
        if stat.S_IMODE(after.st_mode) != 0o700:
            fail(f"{label} directory must have exact owner-only mode 0700")
        if hasattr(os, "geteuid") and after.st_uid != os.geteuid():
            fail(f"{label} directory is not owned by the effective user")
    return resolved


def read_private_local_receipt(path: Path) -> tuple[dict[str, Any], str]:
    if not path.is_absolute():
        fail("local qualification receipt path must be absolute")
    parent = validate_private_evidence_directory(
        path.parent,
        "local qualification receipt",
    )
    candidate = parent / path.name
    raw, digest, metadata = read_stable_bytes_with_metadata(
        candidate,
        "local qualification receipt",
        MAX_JSON_BYTES,
    )
    if os.name == "posix":
        if stat.S_IMODE(metadata.st_mode) != 0o600:
            fail("local qualification receipt must have exact owner-read/write mode 0600")
        if hasattr(os, "geteuid") and metadata.st_uid != os.geteuid():
            fail("local qualification receipt is not owned by the effective user")
    return parse_json_object_bytes(raw, "local qualification receipt"), digest


def collect_head_equivalent_source_inventory(
    source_root: Path,
    expected_commit: str,
) -> tuple[str, dict[str, str]]:
    source = source_root.resolve(strict=True)
    if not source.is_dir():
        fail("local qualification source root is not a directory")
    top_level = Path(
        git_text(source, ["rev-parse", "--show-toplevel"], "Git top-level path")
    ).resolve(strict=True)
    if top_level != source:
        fail("local qualification source root is not the exact Git top-level directory")
    object_format = git_text(source, ["rev-parse", "--show-object-format"], "Git object format")
    if object_format != "sha1":
        fail("local qualification source repository does not use the admitted SHA-1 object format")
    commit = require_commit(
        git_text(source, ["rev-parse", "HEAD"], "Git HEAD"),
        "source.git_commit",
    )
    expected = require_commit(expected_commit, "expected_commit")
    if commit != expected:
        fail("local qualification receipt source commit differs from current source HEAD")
    tree = require_commit(
        git_text(source, ["rev-parse", "HEAD^{tree}"], "Git HEAD tree"),
        "source.git_tree",
    )
    status = run_git(
        source,
        ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )
    if status:
        fail("local qualification receipt source worktree is not clean")
    listing = run_git(source, ["ls-tree", "-rz", "--full-tree", commit])
    entries: list[tuple[str, str]] = []
    total_bytes = 0
    for record in listing.split(b"\0"):
        if not record:
            continue
        if len(entries) >= MAX_SOURCE_ENTRIES:
            fail("local qualification source inventory exceeds its entry-count bound")
        try:
            metadata, raw_path = record.split(b"\t", 1)
            raw_mode, raw_kind, raw_object = metadata.split(b" ", 2)
            mode = raw_mode.decode("ascii")
            kind = raw_kind.decode("ascii")
            object_id = raw_object.decode("ascii")
            relative = raw_path.decode("utf-8")
        except (ValueError, UnicodeDecodeError) as exc:
            fail(f"cannot parse local qualification source entry: {exc}")
        path = validate_relative_path(relative, "local_receipt.digests.path")
        if kind != "blob" or mode not in {"100644", "100755"}:
            fail(
                f"local qualification source entry {path!r} has unsupported mode {mode!r} and kind {kind!r}"
            )
        expected_blob = require_commit(object_id, f"local source entry {path}.git_blob")
        candidate = source / PurePosixPath(path)
        resolved = candidate.resolve(strict=True)
        if resolved != candidate.absolute() or source not in resolved.parents:
            fail(f"local qualification source entry {path!r} traverses a symbolic-link component")
        raw, digest, metadata_after = read_stable_bytes_with_metadata(
            candidate,
            f"local qualification source entry {path}",
            MAX_HASHED_FILE_BYTES,
            allow_empty=True,
        )
        if git_blob_object_id(raw) != expected_blob:
            fail(f"local qualification source bytes differ from HEAD for {path}")
        if not executable_semantics_match(mode, stat.S_IMODE(metadata_after.st_mode)):
            fail(f"local qualification source executable semantics differ from HEAD for {path}")
        total_bytes += len(raw)
        if total_bytes > MAX_SOURCE_TOTAL_BYTES:
            fail("local qualification source inventory exceeds its total byte bound")
        entries.append((path, digest))
    entries.sort(key=lambda item: item[0].encode("utf-8"))
    if not entries:
        fail("local qualification source inventory is empty")
    if require_commit(
        git_text(source, ["rev-parse", "HEAD"], "Git HEAD after source verification"),
        "source.git_commit_after",
    ) != commit:
        fail("local qualification source HEAD changed while being verified")
    if require_commit(
        git_text(source, ["rev-parse", "HEAD^{tree}"], "Git tree after source verification"),
        "source.git_tree_after",
    ) != tree:
        fail("local qualification source tree changed while being verified")
    if run_git(source, ["status", "--porcelain=v1", "-z", "--untracked-files=all"]):
        fail("local qualification source worktree changed while being verified")
    return tree, dict(entries)


def validate_local_qualification_receipt(
    path: Path,
    source_root: Path,
    expected_commit: str,
    expected_sha256: str,
    expected_gates: list[str] | None = None,
) -> dict[str, Any]:
    receipt, actual_sha256 = read_private_local_receipt(path)
    if actual_sha256 != expected_sha256:
        fail("local qualification receipt bytes do not match the server receipt binding")
    require_exact_keys(
        receipt,
        {
            "schema",
            "status",
            "started_at",
            "finished_at",
            "source",
            "host",
            "toolchain",
            "digests",
            "gates",
        },
        "local_receipt",
    )
    if receipt.get("schema") != LOCAL_RECEIPT_SCHEMA:
        fail("local qualification receipt schema is unsupported")
    if receipt.get("status") != "passed":
        fail("local qualification receipt did not pass")
    require_string(receipt.get("started_at"), "local_receipt.started_at", 128)
    require_string(receipt.get("finished_at"), "local_receipt.finished_at", 128)
    source = require_object(receipt.get("source"), "local_receipt.source")
    require_exact_keys(
        source,
        {"commit", "dirty", "head_equivalent", "tree", "snapshot_digest"},
        "local_receipt.source",
    )
    commit = require_commit(source.get("commit"), "local_receipt.source.commit")
    if commit != expected_commit:
        fail("local qualification receipt names a different source commit")
    if source.get("dirty") is not False:
        fail("local qualification receipt is not bound to a clean source tree")
    if source.get("head_equivalent") is not True:
        fail("local qualification receipt is not bound to HEAD-equivalent source")
    declared_tree = require_commit(source.get("tree"), "local_receipt.source.tree")
    require_hash(source.get("snapshot_digest"), "local_receipt.source.snapshot_digest")
    require_object(receipt.get("host"), "local_receipt.host")
    require_object(receipt.get("toolchain"), "local_receipt.toolchain")

    digests = require_object(receipt.get("digests"), "local_receipt.digests")
    if not digests or len(digests) > MAX_SOURCE_ENTRIES:
        fail("local qualification receipt digest inventory is empty or exceeds its bound")
    declared_paths = list(digests)
    if declared_paths != sorted(declared_paths, key=lambda value: value.encode("utf-8")):
        fail("local qualification receipt digest paths are not in canonical UTF-8 byte order")
    normalized_digests: dict[str, str] = {}
    for raw_path, raw_digest in digests.items():
        relative = validate_relative_path(raw_path, "local_receipt.digests.path")
        normalized_digests[relative] = require_hash(
            raw_digest,
            f"local_receipt.digests.{relative}",
        )
    current_tree, current_digests = collect_head_equivalent_source_inventory(
        source_root,
        commit,
    )
    if declared_tree != current_tree:
        fail("local qualification receipt tree differs from current source HEAD")
    if normalized_digests != current_digests:
        missing = sorted(set(current_digests) - set(normalized_digests))
        extra = sorted(set(normalized_digests) - set(current_digests))
        changed = sorted(
            path
            for path in set(current_digests) & set(normalized_digests)
            if current_digests[path] != normalized_digests[path]
        )
        details = []
        if missing:
            details.append("missing=" + ",".join(missing[:8]))
        if extra:
            details.append("extra=" + ",".join(extra[:8]))
        if changed:
            details.append("changed=" + ",".join(changed[:8]))
        fail(
            "local qualification receipt digest inventory differs from current HEAD-equivalent source"
            + (": " + "; ".join(details) if details else "")
        )

    required_gates = (
        EXPECTED_LOCAL_QUALIFICATION_GATES
        if expected_gates is None
        else expected_gates
    )
    gates = require_list(receipt.get("gates"), "local_receipt.gates")
    if len(gates) != len(required_gates):
        fail("local qualification receipt gate count drifted")
    for index, expected_name in enumerate(required_gates):
        gate = require_object(gates[index], f"local_receipt.gates[{index}]")
        require_exact_keys(
            gate,
            {"name", "state", "detail"},
            f"local_receipt.gates[{index}]",
        )
        if gate.get("name") != expected_name:
            fail("local qualification receipt gate set or order drifted")
        if gate.get("state") != "passed":
            fail(f"local qualification gate {expected_name!r} did not pass")
        detail = gate.get("detail")
        if detail is not None:
            require_string(detail, f"local_receipt.gates[{index}].detail", 4096)
    return receipt


def validate_receipt(
    receipt_path: Path,
    contract_path: Path,
    source_root: Path,
    local_qualification_receipt: Path,
    expected_commit: str | None = None,
    require_current_platform: bool = True,
) -> dict[str, Any]:
    contract = load_contract(contract_path)
    receipt, receipt_file_sha256 = read_object_with_digest(
        receipt_path,
        "live server binary receipt",
    )
    require_exact_keys(
        receipt,
        {
            "schema",
            "status",
            "receipt_digest",
            "source",
            "platform",
            "toolchain",
            "binary",
            "executable_checks",
            "source_digests",
            "mutation_capabilities",
            "claims_not_established",
        },
        "server_receipt",
    )
    if receipt.get("schema") != RECEIPT_SCHEMA or receipt.get("status") != "qualified":
        fail("server receipt is not a qualified V1 receipt")
    declared_receipt_digest = require_hash(
        receipt.get("receipt_digest"),
        "server_receipt.receipt_digest",
    )
    unsigned = dict(receipt)
    del unsigned["receipt_digest"]
    if sha256_bytes(canonical_json(unsigned)) != declared_receipt_digest:
        fail("server receipt fields do not reproduce receipt_digest")

    source = require_object(receipt.get("source"), "server_receipt.source")
    require_exact_keys(
        source,
        {"dfmcp_commit", "dfmcp_dirty", "local_qualification_receipt_sha256"},
        "server_receipt.source",
    )
    commit = require_commit(
        source.get("dfmcp_commit"),
        "server_receipt.source.dfmcp_commit",
    )
    if expected_commit is not None and commit != require_commit(
        expected_commit,
        "expected_commit",
    ):
        fail("server receipt source commit differs from the expected deployment commit")
    if source.get("dfmcp_dirty") is not False:
        fail("server receipt is not bound to a clean source tree")
    local_receipt_sha = require_hash(
        source.get("local_qualification_receipt_sha256"),
        "server_receipt.source.local_qualification_receipt_sha256",
    )
    local_receipt = validate_local_qualification_receipt(
        local_qualification_receipt,
        source_root,
        commit,
        local_receipt_sha,
        contract["source_binding"]["required_local_qualification_gates"],
    )

    platform_value = require_object(receipt.get("platform"), "server_receipt.platform")
    require_exact_keys(platform_value, {"system", "machine"}, "server_receipt.platform")
    system = require_string(
        platform_value.get("system"),
        "server_receipt.platform.system",
        128,
    )
    machine = require_string(
        platform_value.get("machine"),
        "server_receipt.platform.machine",
        128,
    )
    if require_current_platform and (
        system != platform.system() or machine != platform.machine()
    ):
        fail("server receipt platform differs from the current execution platform")

    toolchain = require_object(receipt.get("toolchain"), "server_receipt.toolchain")
    require_exact_keys(toolchain, {"rustc_vv", "cargo"}, "server_receipt.toolchain")
    require_string(
        toolchain.get("rustc_vv"),
        "server_receipt.toolchain.rustc_vv",
        16 * 1024,
    )
    require_string(
        toolchain.get("cargo"),
        "server_receipt.toolchain.cargo",
        1024,
    )

    binary = require_object(receipt.get("binary"), "server_receipt.binary")
    require_exact_keys(
        binary,
        {"name", "profile", "relative_path", "bytes", "sha256"},
        "server_receipt.binary",
    )
    contract_binary = contract["binary"]
    if binary.get("name") != contract_binary["name"] or binary.get("profile") != "release":
        fail("server receipt identifies the wrong binary or build profile")
    relative_path = validate_relative_path(
        binary.get("relative_path"),
        "server_receipt.binary.relative_path",
    )
    if PurePosixPath(relative_path).name not in {
        "dwarf-fortress-mcp",
        "dwarf-fortress-mcp.exe",
    }:
        fail("server receipt relative path has the wrong executable filename")
    binary_bytes = require_positive_int(
        binary.get("bytes"),
        "server_receipt.binary.bytes",
        require_positive_int(
            contract_binary.get("maximum_bytes"),
            "contract.binary.maximum_bytes",
        ),
    )
    binary_sha256 = require_hash(
        binary.get("sha256"),
        "server_receipt.binary.sha256",
    )

    checks = require_list(
        receipt.get("executable_checks"),
        "server_receipt.executable_checks",
    )
    expected_checks = contract["required_executable_checks"]
    if len(checks) != len(expected_checks):
        fail("server receipt executable-check count drifted")
    normalized_checks: list[dict[str, Any]] = []
    for index, expected_name in enumerate(expected_checks):
        check = require_object(
            checks[index],
            f"server_receipt.executable_checks[{index}]",
        )
        require_exact_keys(
            check,
            {"name", "status", "stdout_sha256", "stderr_sha256"},
            f"server_receipt.executable_checks[{index}]",
        )
        if check.get("name") != expected_name or check.get("status") != "passed":
            fail(f"server executable check {expected_name} did not pass in canonical order")
        normalized_checks.append(
            {
                "name": expected_name,
                "status": "passed",
                "stdout_sha256": require_hash(
                    check.get("stdout_sha256"),
                    f"server_receipt.executable_checks[{index}].stdout_sha256",
                ),
                "stderr_sha256": require_hash(
                    check.get("stderr_sha256"),
                    f"server_receipt.executable_checks[{index}].stderr_sha256",
                ),
            }
        )

    source_digests = require_object(
        receipt.get("source_digests"),
        "server_receipt.source_digests",
    )
    required_digests = contract["source_binding"]["required_source_digests"]
    if set(source_digests) != set(required_digests):
        fail("server receipt source-digest key set drifted")
    normalized_digests: dict[str, str] = {}
    for name, relative in required_digests.items():
        declared = require_hash(
            source_digests.get(name),
            f"server_receipt.source_digests.{name}",
        )
        canonical_relative = validate_relative_path(
            relative,
            f"contract.source_binding.required_source_digests.{name}",
        )
        actual = sha256_file(source_root / PurePosixPath(canonical_relative))
        if declared != actual:
            fail(f"server receipt source digest differs for {canonical_relative}")
        normalized_digests[name] = declared

    final_tree, final_inventory = collect_head_equivalent_source_inventory(
        source_root,
        commit,
    )
    if final_tree != local_receipt["source"]["tree"]:
        fail("server receipt source tree changed after local receipt verification")
    if final_inventory != local_receipt["digests"]:
        fail("server receipt source inventory changed after local receipt verification")

    if receipt.get("mutation_capabilities") != []:
        fail("server receipt must carry no mutation capabilities")
    if receipt.get("claims_not_established") != contract.get("claims_not_established"):
        fail("server receipt claims-not-established set drifted")

    return {
        "receipt_sha256": receipt_file_sha256,
        "receipt_digest": declared_receipt_digest,
        "source": {
            "dfmcp_commit": commit,
            "dfmcp_dirty": False,
            "local_qualification_receipt_sha256": local_receipt_sha,
        },
        "platform": {"system": system, "machine": machine},
        "binary": {
            "name": binary["name"],
            "profile": "release",
            "relative_path": relative_path,
            "bytes": binary_bytes,
            "sha256": binary_sha256,
        },
        "executable_checks": normalized_checks,
        "source_digests": normalized_digests,
        "mutation_capabilities": [],
    }


def validate_open_metadata(metadata: os.stat_result) -> None:
    if not stat.S_ISREG(metadata.st_mode):
        fail("opened server artifact is not a regular file")
    if metadata.st_mode & (stat.S_IWGRP | stat.S_IWOTH):
        fail("server artifact is group- or world-writable")
    if metadata.st_mode & (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH) == 0:
        fail("opened server artifact has no executable permission bit")
    if hasattr(os, "geteuid"):
        permitted_owners = {0, os.geteuid()}
        if metadata.st_uid not in permitted_owners:
            fail("server artifact is not owned by root or the launching effective user")


def open_verified_binary(binary_path: Path, expected: dict[str, Any]) -> OpenBinary:
    raw = os.fspath(binary_path)
    if not raw or len(os.fsencode(raw)) > 4096:
        fail("server artifact path is empty or exceeds its byte bound")
    if any(ord(character) < 0x20 for character in raw):
        fail("server artifact path contains a control character")
    absolute = binary_path if binary_path.is_absolute() else Path.cwd() / binary_path
    parent = absolute.parent.resolve(strict=True)
    candidate = parent / absolute.name
    if not hasattr(os, "O_NOFOLLOW"):
        fail("this platform cannot enforce no-follow executable opening")
    flags = os.O_RDONLY | os.O_NOFOLLOW
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    try:
        descriptor = os.open(candidate, flags)
    except OSError as exc:
        fail(f"cannot open server artifact without following symbolic links: {exc}")
    try:
        before = os.fstat(descriptor)
        validate_open_metadata(before)
        if before.st_size != expected["bytes"]:
            fail(
                f"server artifact size {before.st_size} differs from receipt size {expected['bytes']}"
            )
        digest = sha256_descriptor(descriptor)
        after = os.fstat(descriptor)
        if not same_identity(before, after):
            fail("server artifact changed while being verified")
        if digest != expected["sha256"]:
            fail("server artifact SHA-256 differs from the qualified receipt")
        return OpenBinary(
            descriptor=descriptor,
            path=candidate,
            sha256=digest,
            size=after.st_size,
            device=after.st_dev,
            inode=after.st_ino,
            mode=stat.S_IMODE(after.st_mode),
            owner_uid=after.st_uid,
        )
    except BaseException:
        os.close(descriptor)
        raise


def verify(
    receipt_path: Path,
    binary_path: Path,
    contract_path: Path,
    source_root: Path,
    local_qualification_receipt: Path,
    expected_commit: str | None = None,
) -> tuple[dict[str, Any], OpenBinary]:
    normalized = validate_receipt(
        receipt_path,
        contract_path,
        source_root,
        local_qualification_receipt,
        expected_commit,
    )
    opened = open_verified_binary(binary_path, normalized["binary"])
    return normalized, opened


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("receipt", type=Path)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--source-root", type=Path, default=ROOT)
    parser.add_argument("--local-qualification-receipt", type=Path, required=True)
    parser.add_argument("--expected-dfmcp-commit")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    opened: OpenBinary | None = None
    try:
        normalized, opened = verify(
            args.receipt,
            args.binary,
            args.contract,
            args.source_root,
            args.local_qualification_receipt,
            args.expected_dfmcp_commit,
        )
    except (OSError, VerificationError) as exc:
        print(f"live server binary receipt: FAIL: {exc}", file=sys.stderr)
        return 1
    finally:
        if opened is not None:
            os.close(opened.descriptor)
    print(
        "live server binary receipt: PASS "
        f"({normalized['source']['dfmcp_commit']}, {normalized['binary']['sha256']})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
