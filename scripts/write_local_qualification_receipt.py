#!/usr/bin/env python3
"""Issue source-stable local qualification snapshots and receipts."""

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
import tempfile
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
from typing import Any, NoReturn

from read_stable_repository_file import StableFile, StableReadError, read_stable_regular_file

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONTRACT = ROOT / "architecture/local_qualification_receipt_v1.json"
DEFAULT_GATE_CONTRACT = ROOT / "architecture/live_server_binary_receipt_v1.json"
CONTRACT_SCHEMA = "dfmcp.local-qualification-receipt-contract/1"
SNAPSHOT_SCHEMA = "dfmcp.qualification-source-snapshot/1"
RECEIPT_SCHEMA = "dfmcp.qualification-receipt.v1"
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
MAX_JSON_BYTES = 128 * 1024 * 1024
MAX_STATUS_BYTES = 8 * 1024 * 1024
MAX_GATE_BYTES = 2 * 1024 * 1024
MAX_TOOLCHAIN_BYTES = 64 * 1024


class QualificationReceiptError(ValueError):
    pass


def fail(message: str) -> NoReturn:
    raise QualificationReceiptError(message)


def canonical_json(value: Any) -> bytes:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")


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


def duplicate_rejecting_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    output: dict[str, Any] = {}
    for key, value in pairs:
        if key in output:
            fail(f"JSON object repeats key {key!r}")
        output[key] = value
    return output


def read_json_bytes(raw: bytes, label: str) -> dict[str, Any]:
    try:
        value = json.loads(
            raw.decode("utf-8"),
            object_pairs_hook=duplicate_rejecting_object,
        )
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        fail(f"cannot parse {label}: {exc}")
    if not isinstance(value, dict):
        fail(f"{label} must be a JSON object")
    return value


def read_json_object(path: Path, label: str, maximum_bytes: int = MAX_JSON_BYTES) -> dict[str, Any]:
    stable = read_stable_regular_file(path, maximum_bytes, label)
    return read_json_bytes(stable.content, label)


def require_exact_keys(value: dict[str, Any], expected: set[str], path: str) -> None:
    actual = set(value)
    if actual != expected:
        fail(f"{path} fields differ: expected {sorted(expected)}, got {sorted(actual)}")


def require_string(value: Any, path: str, maximum: int = 16 * 1024) -> str:
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


def require_positive_int(value: Any, path: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        fail(f"{path} must be a positive integer")
    return value


def require_nonnegative_int(value: Any, path: str, maximum: int | None = None) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        fail(f"{path} must be a nonnegative integer")
    if maximum is not None and value > maximum:
        fail(f"{path} must be <= {maximum}")
    return value


def validate_relative_path(value: Any, path: str) -> str:
    text = require_string(value, path, 4096)
    if "\\" in text or text.startswith("/") or text.endswith("/") or "//" in text:
        fail(f"{path} is not a canonical relative POSIX path")
    candidate = PurePosixPath(text)
    if candidate.is_absolute() or any(part in {"", ".", ".."} for part in candidate.parts):
        fail(f"{path} contains an absolute, empty, dot, or parent component")
    if candidate.as_posix() != text:
        fail(f"{path} is not in canonical POSIX form")
    return text


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


def run_git(source_root: Path, arguments: list[str], *, binary: bool = False) -> bytes | str:
    try:
        completed = subprocess.run(
            ["git", "-C", os.fspath(source_root), *arguments],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=sanitized_git_environment(),
        )
    except OSError as exc:
        fail(f"cannot execute Git: {exc}")
    if completed.returncode != 0:
        detail = completed.stderr.decode("utf-8", errors="replace")
        fail(f"Git command failed: {detail.strip()[:2048]}")
    if len(completed.stdout) > MAX_JSON_BYTES:
        fail("Git output exceeds its byte bound")
    if binary:
        return completed.stdout
    try:
        return completed.stdout.decode("utf-8")
    except UnicodeDecodeError as exc:
        fail(f"Git output is not valid UTF-8: {exc}")


def git_identity(source_root: Path) -> tuple[str, str]:
    commit_raw = run_git(source_root, ["rev-parse", "HEAD"])
    tree_raw = run_git(source_root, ["rev-parse", "HEAD^{tree}"])
    if not isinstance(commit_raw, str) or not isinstance(tree_raw, str):
        fail("Git identity unexpectedly returned binary output")
    return (
        require_commit(commit_raw.strip(), "git.commit"),
        require_commit(tree_raw.strip(), "git.tree"),
    )


def git_status(source_root: Path) -> bytes:
    raw = run_git(
        source_root,
        ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        binary=True,
    )
    if not isinstance(raw, bytes):
        fail("Git status unexpectedly returned text output")
    if len(raw) > MAX_STATUS_BYTES:
        fail("Git worktree status exceeds its byte bound")
    return raw


def load_contract(path: Path) -> dict[str, Any]:
    contract = read_json_object(path, "local qualification receipt contract", 1024 * 1024)
    require_exact_keys(
        contract,
        {
            "schema_version",
            "receipt_schema",
            "snapshot_schema",
            "status",
            "gate_contract",
            "source",
            "receipt_statuses",
            "publication",
            "authority",
            "limitations",
        },
        "contract",
    )
    if contract.get("schema_version") != CONTRACT_SCHEMA:
        fail("local qualification contract schema is unsupported")
    if contract.get("receipt_schema") != RECEIPT_SCHEMA:
        fail("local qualification receipt schema drifted")
    if contract.get("snapshot_schema") != SNAPSHOT_SCHEMA:
        fail("local qualification snapshot schema drifted")
    if contract.get("status") != "normative_two_phase_local_qualification":
        fail("local qualification contract status drifted")
    if contract.get("gate_contract") != "architecture/live_server_binary_receipt_v1.json":
        fail("local qualification gate contract drifted")

    source = contract.get("source")
    if not isinstance(source, dict):
        fail("contract.source must be an object")
    require_exact_keys(
        source,
        {
            "clean_required_for_passed",
            "head_must_remain_exact",
            "tree_must_remain_exact",
            "worktree_status_must_remain_exact",
            "tracked_inventory_must_remain_exact",
            "working_tree_bytes_must_match_head_blobs_for_clean_passed",
            "working_tree_executable_semantics_must_match_head_on_unix",
            "git_environment_must_be_sanitized",
            "git_object_format",
            "tracked_entry_modes",
            "symbolic_links_allowed",
            "gitlinks_allowed",
            "maximum_entries",
            "maximum_entry_bytes",
            "maximum_total_bytes",
            "hash_algorithm",
        },
        "contract.source",
    )
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
        if source.get(field) is not True:
            fail(f"contract.source.{field} must remain true")
    if source.get("git_object_format") != "sha1":
        fail("local qualification Git object format drifted")
    if source.get("tracked_entry_modes") != ["100644", "100755"]:
        fail("local qualification tracked mode set drifted")
    if source.get("symbolic_links_allowed") is not False:
        fail("local qualification contract permits symbolic links")
    if source.get("gitlinks_allowed") is not False:
        fail("local qualification contract permits gitlinks")
    for field in ["maximum_entries", "maximum_entry_bytes", "maximum_total_bytes"]:
        require_positive_int(source.get(field), f"contract.source.{field}")
    if source.get("hash_algorithm") != "sha256":
        fail("local qualification hash algorithm drifted")

    statuses = contract.get("receipt_statuses")
    if not isinstance(statuses, dict) or list(statuses) != [
        "passed",
        "development_dirty",
        "static_only",
        "failed",
    ]:
        fail("local qualification receipt status set or order drifted")

    publication = contract.get("publication")
    if not isinstance(publication, dict):
        fail("contract.publication must be an object")
    require_exact_keys(
        publication,
        {
            "run_directory_create_only",
            "run_directory_final_component_symbolic_links_allowed",
            "unix_run_directory_mode",
            "run_directory_owner_must_match_effective_user_when_available",
            "gate_journal_mode",
            "snapshot_create_only",
            "snapshot_mode",
            "receipt_create_only",
            "receipt_mode",
            "temporary_file_fsync",
            "atomic_no_replace_hard_link",
            "parent_directory_fsync",
            "reverify_source_after_receipt_publication",
            "invalid_evidence_removed_and_absence_verified",
        },
        "contract.publication",
    )
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
        if publication.get(field) is not True:
            fail(f"contract.publication.{field} must remain true")
    if publication.get("run_directory_final_component_symbolic_links_allowed") is not False:
        fail("local qualification evidence directory may be a symbolic link")
    for field, expected in [
        ("unix_run_directory_mode", "0700"),
        ("gate_journal_mode", "0600"),
        ("snapshot_mode", "0600"),
        ("receipt_mode", "0600"),
    ]:
        if publication.get(field) != expected:
            fail(f"contract.publication.{field} drifted")

    authority = contract.get("authority")
    if not isinstance(authority, dict):
        fail("contract.authority must be an object")
    for field in ["executes_project_code", "modifies_source", "network_access"]:
        if authority.get(field) is not False:
            fail(f"local qualification contract unexpectedly grants {field}")
    if authority.get("grants_capabilities") != [] or authority.get("mutation_capabilities") != []:
        fail("local qualification contract grants authority")
    return contract


def load_required_gates(path: Path) -> list[str]:
    contract = read_json_object(path, "live server binary receipt contract", 2 * 1024 * 1024)
    if contract.get("schema_version") != "dfmcp.live-server-binary-receipt-contract/1":
        fail("gate contract schema is unsupported")
    binding = contract.get("source_binding")
    if not isinstance(binding, dict):
        fail("gate contract source_binding must be an object")
    raw = binding.get("required_local_qualification_gates")
    if not isinstance(raw, list) or not raw:
        fail("gate contract has no required local qualification gates")
    gates: list[str] = []
    for index, value in enumerate(raw):
        gate = require_string(value, f"gate_contract.gates[{index}]", 128)
        if gate in gates:
            fail(f"gate contract repeats {gate!r}")
        gates.append(gate)
    return gates


def executable_semantics_match(git_mode: str, worktree_mode: int) -> bool:
    if os.name != "posix":
        return True
    return bool(worktree_mode & 0o111) == (git_mode == "100755")


def collect_source_snapshot(
    source_root: Path,
    contract: dict[str, Any],
    expected_commit: str,
) -> dict[str, Any]:
    source = source_root.resolve(strict=True)
    if not source.is_dir():
        fail("source root is not a directory")
    expected = require_commit(expected_commit, "expected_commit")
    before_commit, before_tree = git_identity(source)
    if before_commit != expected:
        fail("source HEAD differs from the expected qualification commit")
    before_status = git_status(source)
    listing = run_git(source, ["ls-tree", "-rz", "--full-tree", before_commit], binary=True)
    if not isinstance(listing, bytes):
        fail("Git tree listing unexpectedly returned text output")

    source_policy = contract["source"]
    maximum_entries = require_positive_int(source_policy["maximum_entries"], "maximum_entries")
    maximum_entry_bytes = require_positive_int(
        source_policy["maximum_entry_bytes"], "maximum_entry_bytes"
    )
    maximum_total_bytes = require_positive_int(
        source_policy["maximum_total_bytes"], "maximum_total_bytes"
    )
    entries: list[dict[str, Any]] = []
    total_bytes = 0
    head_equivalent = True
    for record in listing.split(b"\0"):
        if not record:
            continue
        if len(entries) >= maximum_entries:
            fail("tracked source inventory exceeds its entry-count bound")
        try:
            metadata, raw_path = record.split(b"\t", 1)
            raw_mode, raw_kind, raw_object = metadata.split(b" ", 2)
            mode = raw_mode.decode("ascii")
            kind = raw_kind.decode("ascii")
            object_id = raw_object.decode("ascii")
            relative = raw_path.decode("utf-8")
        except (ValueError, UnicodeDecodeError) as exc:
            fail(f"cannot parse tracked source entry: {exc}")
        path = validate_relative_path(relative, "tracked_source.path")
        if kind != "blob" or mode not in {"100644", "100755"}:
            fail(
                f"tracked source entry {path!r} has unsupported mode {mode!r} and kind {kind!r}"
            )
        object_id = require_commit(object_id, f"tracked_source[{path}].git_blob")
        candidate = source / PurePosixPath(path)
        resolved = candidate.resolve(strict=True)
        if resolved != candidate.absolute() or source not in resolved.parents:
            fail(f"tracked source entry {path!r} traverses a symbolic-link component")
        stable = read_stable_regular_file(
            candidate,
            maximum_entry_bytes,
            f"tracked source entry {path}",
            allow_empty=True,
        )
        actual_object_id = git_blob_object_id(stable.content)
        mode_matches = executable_semantics_match(mode, stable.mode)
        if actual_object_id != object_id or not mode_matches:
            head_equivalent = False
        total_bytes += stable.size
        if total_bytes > maximum_total_bytes:
            fail("tracked source inventory exceeds its total byte bound")
        entries.append(
            {
                "path": path,
                "mode": mode,
                "git_blob": object_id,
                "worktree_mode": stable.mode,
                "bytes": stable.size,
                "sha256": stable.sha256,
            }
        )
    entries.sort(key=lambda entry: entry["path"].encode("utf-8"))
    if not entries:
        fail("tracked source inventory is empty")
    after_commit, after_tree = git_identity(source)
    after_status = git_status(source)
    if after_commit != before_commit or after_tree != before_tree:
        fail("source commit or tree changed while collecting the qualification snapshot")
    if after_status != before_status:
        fail("worktree status changed while collecting the qualification snapshot")
    dirty = bool(before_status) or not head_equivalent
    unsigned: dict[str, Any] = {
        "schema": SNAPSHOT_SCHEMA,
        "source": {
            "commit": before_commit,
            "tree": before_tree,
            "dirty": dirty,
            "head_equivalent": head_equivalent,
            "status_sha256": sha256_bytes(before_status),
        },
        "entries": entries,
        "entries_digest": sha256_bytes(canonical_json(entries)),
    }
    return {**unsigned, "snapshot_digest": sha256_bytes(canonical_json(unsigned))}


def validate_snapshot(value: dict[str, Any]) -> dict[str, Any]:
    require_exact_keys(
        value,
        {"schema", "source", "entries", "entries_digest", "snapshot_digest"},
        "snapshot",
    )
    if value.get("schema") != SNAPSHOT_SCHEMA:
        fail("qualification snapshot schema is unsupported")
    source = value.get("source")
    if not isinstance(source, dict):
        fail("snapshot.source must be an object")
    require_exact_keys(
        source,
        {"commit", "tree", "dirty", "head_equivalent", "status_sha256"},
        "snapshot.source",
    )
    require_commit(source.get("commit"), "snapshot.source.commit")
    require_commit(source.get("tree"), "snapshot.source.tree")
    if not isinstance(source.get("dirty"), bool):
        fail("snapshot.source.dirty must be Boolean")
    if not isinstance(source.get("head_equivalent"), bool):
        fail("snapshot.source.head_equivalent must be Boolean")
    if source["head_equivalent"] is False and source["dirty"] is not True:
        fail("a HEAD-divergent snapshot must be marked dirty")
    require_hash(source.get("status_sha256"), "snapshot.source.status_sha256")
    entries = value.get("entries")
    if not isinstance(entries, list) or not entries:
        fail("snapshot.entries must be a nonempty array")
    previous: bytes | None = None
    normalized: list[dict[str, Any]] = []
    for index, raw in enumerate(entries):
        if not isinstance(raw, dict):
            fail(f"snapshot.entries[{index}] must be an object")
        require_exact_keys(
            raw,
            {"path", "mode", "git_blob", "worktree_mode", "bytes", "sha256"},
            f"snapshot.entries[{index}]",
        )
        path = validate_relative_path(raw.get("path"), f"snapshot.entries[{index}].path")
        encoded = path.encode("utf-8")
        if previous is not None and encoded <= previous:
            fail("snapshot entries are not in strict UTF-8 path-byte order")
        previous = encoded
        mode = require_string(raw.get("mode"), f"snapshot.entries[{index}].mode", 6)
        if mode not in {"100644", "100755"}:
            fail(f"snapshot.entries[{index}].mode is unsupported")
        normalized.append(
            {
                "path": path,
                "mode": mode,
                "git_blob": require_commit(
                    raw.get("git_blob"), f"snapshot.entries[{index}].git_blob"
                ),
                "worktree_mode": require_nonnegative_int(
                    raw.get("worktree_mode"),
                    f"snapshot.entries[{index}].worktree_mode",
                    0o7777,
                ),
                "bytes": require_nonnegative_int(
                    raw.get("bytes"), f"snapshot.entries[{index}].bytes"
                ),
                "sha256": require_hash(
                    raw.get("sha256"), f"snapshot.entries[{index}].sha256"
                ),
            }
        )
    if value.get("entries_digest") != sha256_bytes(canonical_json(normalized)):
        fail("qualification snapshot entries do not reproduce entries_digest")
    unsigned = dict(value)
    declared = unsigned.pop("snapshot_digest", None)
    require_hash(declared, "snapshot.snapshot_digest")
    if declared != sha256_bytes(canonical_json(unsigned)):
        fail("qualification snapshot fields do not reproduce snapshot_digest")
    return value


def validate_private_evidence_directory(path: Path) -> Path:
    if not path.is_absolute():
        fail("qualification evidence directory must be absolute")
    absolute = Path(os.path.abspath(path))
    try:
        before = os.lstat(absolute)
    except OSError as exc:
        fail(f"cannot inspect qualification evidence directory {absolute}: {exc}")
    if stat.S_ISLNK(before.st_mode) or not stat.S_ISDIR(before.st_mode):
        fail("qualification evidence directory must be a real directory, not a symbolic link")
    resolved = absolute.resolve(strict=True)
    try:
        after = os.stat(resolved, follow_symlinks=False)
    except OSError as exc:
        fail(f"cannot inspect resolved qualification evidence directory {resolved}: {exc}")
    if not stat.S_ISDIR(after.st_mode):
        fail("resolved qualification evidence path is not a directory")
    if os.name == "posix":
        if stat.S_IMODE(after.st_mode) != 0o700:
            fail("qualification evidence directory must have exact owner-only mode 0700")
        if hasattr(os, "geteuid") and after.st_uid != os.geteuid():
            fail("qualification evidence directory is not owned by the effective user")
    return resolved


def private_evidence_candidate(path: Path, expected_parent: Path | None = None) -> tuple[Path, Path]:
    if not path.is_absolute():
        fail("qualification evidence path must be absolute")
    if not path.name or path.name in {".", ".."} or any(ord(ch) < 0x20 for ch in path.name):
        fail("qualification evidence filename is invalid")
    parent = validate_private_evidence_directory(path.parent)
    if expected_parent is not None and parent != expected_parent:
        fail("qualification evidence files must share one private run directory")
    return parent / path.name, parent


def validate_private_evidence_file(stable: StableFile, label: str) -> None:
    if os.name == "posix":
        if stable.mode != 0o600:
            fail(f"{label} must have exact owner-read/write mode 0600")
        if hasattr(os, "geteuid") and stable.owner_uid != os.geteuid():
            fail(f"{label} is not owned by the effective user")


def read_private_evidence_file(
    path: Path,
    maximum_bytes: int,
    label: str,
    expected_parent: Path | None = None,
) -> StableFile:
    candidate, _ = private_evidence_candidate(path, expected_parent)
    stable = read_stable_regular_file(candidate, maximum_bytes, label)
    validate_private_evidence_file(stable, label)
    return stable


def parse_gates(
    path: Path,
    required: list[str],
    requested_status: str,
    expected_parent: Path | None = None,
) -> list[dict[str, Any]]:
    stable = read_private_evidence_file(
        path,
        MAX_GATE_BYTES,
        "qualification gate journal",
        expected_parent,
    )
    try:
        text = stable.content.decode("utf-8")
    except UnicodeDecodeError as exc:
        fail(f"qualification gate journal is not UTF-8: {exc}")
    gates: list[dict[str, Any]] = []
    for line_number, line in enumerate(text.splitlines(), start=1):
        fields = line.split("\t")
        if len(fields) != 3:
            fail(f"qualification gate journal line {line_number} must have exactly three fields")
        name = require_string(fields[0], f"gates[{line_number}].name", 128)
        state = require_string(fields[1], f"gates[{line_number}].state", 32)
        if state not in {"passed", "failed", "skipped"}:
            fail(f"qualification gate {name!r} has unsupported state {state!r}")
        detail = fields[2] or None
        if detail is not None:
            require_string(detail, f"gates[{line_number}].detail", 4096)
        gates.append({"name": name, "state": state, "detail": detail})
    names = [gate["name"] for gate in gates]
    if names != required[: len(names)]:
        fail("qualification gate journal is not an exact prefix of the canonical gate order")
    if requested_status == "passed":
        if names != required or any(gate["state"] != "passed" for gate in gates):
            fail("a completed qualification requires every canonical gate to pass")
    elif requested_status == "static_only":
        if not gates or any(gate["state"] != "passed" for gate in gates):
            fail("a static-only receipt requires a nonempty passing canonical gate prefix")
    elif requested_status == "failed":
        if gates and all(gate["state"] == "passed" for gate in gates) and names == required:
            fail("a failed receipt cannot contain a complete all-passing gate sequence")
    else:
        fail(f"unsupported requested qualification status {requested_status!r}")
    return gates


def command_output(arguments: list[str]) -> str | None:
    try:
        completed = subprocess.run(
            arguments,
            check=False,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )
    except OSError:
        return None
    if completed.returncode != 0:
        return None
    encoded = completed.stdout.strip().encode("utf-8")
    if len(encoded) > MAX_TOOLCHAIN_BYTES:
        return None
    return encoded.decode("utf-8")


def fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def write_atomic_create_only(path: Path, value: dict[str, Any]) -> Path:
    destination, parent = private_evidence_candidate(path)
    payload = json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False).encode("utf-8") + b"\n"
    temporary: str | None = None
    published = False
    try:
        descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=parent)
        if hasattr(os, "fchmod"):
            os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "wb", closefd=True) as handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        try:
            os.link(temporary, destination, follow_symlinks=False)
        except FileExistsError:
            fail(f"qualification evidence already exists: {destination}")
        except OSError as exc:
            fail(f"cannot publish qualification evidence without replacement: {exc}")
        published = True
        os.unlink(temporary)
        temporary = None
        fsync_directory(parent)
        stable = read_stable_regular_file(destination, len(payload), "published qualification evidence")
        validate_private_evidence_file(stable, "published qualification evidence")
        if stable.content != payload:
            fail("published qualification evidence bytes differ from the prepared payload")
        return destination
    except BaseException:
        if published:
            try:
                os.unlink(destination)
                fsync_directory(parent)
            except OSError:
                pass
        raise
    finally:
        if temporary is not None:
            try:
                os.unlink(temporary)
                fsync_directory(parent)
            except OSError:
                pass


def remove_published_evidence(path: Path, label: str) -> None:
    candidate, parent = private_evidence_candidate(path)
    try:
        os.unlink(candidate)
    except FileNotFoundError:
        return
    except OSError as exc:
        fail(f"cannot remove invalid {label}: {exc}")
    fsync_directory(parent)
    try:
        os.lstat(candidate)
    except FileNotFoundError:
        return
    except OSError as exc:
        fail(f"cannot verify invalid {label} removal: {exc}")
    fail(f"invalid {label} still exists after removal")


def begin(
    source_root: Path,
    contract_path: Path,
    snapshot_path: Path,
    expected_commit: str,
    allow_dirty: bool,
) -> dict[str, Any]:
    contract = load_contract(contract_path)
    private_evidence_candidate(snapshot_path)
    snapshot = collect_source_snapshot(source_root, contract, expected_commit)
    if snapshot["source"]["dirty"] and not allow_dirty:
        fail(
            "clean local qualification refuses a dirty, untracked, mode-divergent, or HEAD-divergent worktree"
        )
    published = write_atomic_create_only(snapshot_path, snapshot)
    current = collect_source_snapshot(source_root, contract, expected_commit)
    if current != snapshot:
        remove_published_evidence(published, "qualification source snapshot")
        fail("source changed while publishing the qualification snapshot")
    return {
        "schema": "dfmcp.qualification-source-snapshot-result/1",
        "status": "captured",
        "commit": snapshot["source"]["commit"],
        "tree": snapshot["source"]["tree"],
        "dirty": snapshot["source"]["dirty"],
        "head_equivalent": snapshot["source"]["head_equivalent"],
        "entry_count": len(snapshot["entries"]),
        "entries_digest": snapshot["entries_digest"],
        "snapshot_digest": snapshot["snapshot_digest"],
        "snapshot": os.fspath(published),
    }


def finish(
    source_root: Path,
    contract_path: Path,
    gate_contract_path: Path,
    snapshot_path: Path,
    gates_path: Path,
    output_path: Path,
    expected_commit: str,
    started_at: str,
    requested_status: str,
) -> dict[str, Any]:
    contract = load_contract(contract_path)
    expected = require_commit(expected_commit, "expected_commit")
    started = require_string(started_at, "started_at", 128)
    output_candidate, private_parent = private_evidence_candidate(output_path)
    snapshot_stable = read_private_evidence_file(
        snapshot_path,
        MAX_JSON_BYTES,
        "qualification source snapshot",
        private_parent,
    )
    snapshot = validate_snapshot(
        read_json_bytes(snapshot_stable.content, "qualification source snapshot")
    )
    if snapshot["source"]["commit"] != expected:
        fail("qualification snapshot commit differs from the expected commit")
    current = collect_source_snapshot(source_root, contract, expected)
    if current != snapshot:
        fail("source snapshot changed during local qualification")
    required_gates = load_required_gates(gate_contract_path)
    gates = parse_gates(gates_path, required_gates, requested_status, private_parent)
    dirty = snapshot["source"]["dirty"]
    head_equivalent = snapshot["source"]["head_equivalent"]
    final_status = "development_dirty" if requested_status == "passed" and dirty else requested_status
    if final_status == "passed" and (dirty or not head_equivalent):
        fail("a clean passing qualification receipt must be HEAD-equivalent")
    digests = {entry["path"]: entry["sha256"] for entry in snapshot["entries"]}
    receipt = {
        "schema": RECEIPT_SCHEMA,
        "status": final_status,
        "started_at": started,
        "finished_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "source": {
            "commit": expected,
            "dirty": dirty,
            "head_equivalent": head_equivalent,
            "tree": snapshot["source"]["tree"],
            "snapshot_digest": snapshot["snapshot_digest"],
        },
        "host": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "python": platform.python_version(),
        },
        "toolchain": {
            "rustc_vv": command_output(["rustc", "-vV"]),
            "cargo": command_output(["cargo", "--version"]),
        },
        "digests": digests,
        "gates": gates,
    }
    current_before_publish = collect_source_snapshot(source_root, contract, expected)
    if current_before_publish != snapshot:
        fail("source changed while preparing the qualification receipt")
    published = write_atomic_create_only(output_candidate, receipt)
    current_after_publish = collect_source_snapshot(source_root, contract, expected)
    if current_after_publish != snapshot:
        remove_published_evidence(published, "qualification receipt")
        fail("source changed while publishing the qualification receipt")
    return receipt


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    begin_parser = subparsers.add_parser("begin")
    begin_parser.add_argument("--source-root", type=Path, default=ROOT)
    begin_parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    begin_parser.add_argument("--snapshot", type=Path, required=True)
    begin_parser.add_argument("--expected-commit", required=True)
    begin_parser.add_argument("--allow-dirty", action="store_true")
    finish_parser = subparsers.add_parser("finish")
    finish_parser.add_argument("--source-root", type=Path, default=ROOT)
    finish_parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    finish_parser.add_argument("--gate-contract", type=Path, default=DEFAULT_GATE_CONTRACT)
    finish_parser.add_argument("--snapshot", type=Path, required=True)
    finish_parser.add_argument("--gates", type=Path, required=True)
    finish_parser.add_argument("--output", type=Path, required=True)
    finish_parser.add_argument("--expected-commit", required=True)
    finish_parser.add_argument("--started-at", required=True)
    finish_parser.add_argument(
        "--requested-status",
        choices=["passed", "static_only", "failed"],
        required=True,
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        if args.command == "begin":
            result = begin(
                args.source_root,
                args.contract,
                args.snapshot,
                args.expected_commit,
                args.allow_dirty,
            )
        else:
            result = finish(
                args.source_root,
                args.contract,
                args.gate_contract,
                args.snapshot,
                args.gates,
                args.output,
                args.expected_commit,
                args.started_at,
                args.requested_status,
            )
    except (OSError, StableReadError, QualificationReceiptError) as exc:
        print(f"local qualification receipt: FAIL: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True, separators=(",", ":"), ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
