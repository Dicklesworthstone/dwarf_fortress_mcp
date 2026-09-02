#!/usr/bin/env python3
"""Verify one deterministic clean-commit source bundle and optional checkout."""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import re
import subprocess
import sys
import tarfile
from pathlib import Path, PurePosixPath
from typing import Any, NoReturn

from read_stable_repository_file import StableReadError, read_stable_regular_file

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONTRACT = ROOT / "architecture/source_bundle_v1.json"
CONTRACT_SCHEMA = "dfmcp.source-bundle-contract/1"
MANIFEST_SCHEMA = "dfmcp.source-bundle/1"
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
MAX_MANIFEST_BYTES = 16 * 1024 * 1024
MAX_STRING_BYTES = 16 * 1024
MAX_DEPTH = 64
MAX_COLLECTION_ITEMS = 131072
ALLOWED_MODES = {"100644", "100755"}
CLAIMS_NOT_ESTABLISHED = [
    "successful compilation",
    "test or qualification success",
    "DFHack compatibility",
    "compatibility-registry admission",
    "binary reproducibility",
    "release signature authenticity",
    "host-compromise resistance",
]


class SourceBundleError(ValueError):
    pass


def fail(message: str) -> NoReturn:
    raise SourceBundleError(message)


def duplicate_rejecting_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    output: dict[str, Any] = {}
    for key, value in pairs:
        if key in output:
            fail(f"JSON object repeats key {key!r}")
        output[key] = value
    return output


def canonical_json(value: Any) -> bytes:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


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
        if any(ord(character) < 0x20 for character in value):
            fail(f"{path} contains a control character")
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


def require_nonnegative_int(value: Any, path: str, maximum: int | None = None) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        fail(f"{path} must be a nonnegative integer")
    if maximum is not None and value > maximum:
        fail(f"{path} must be <= {maximum}")
    return value


def validate_basename(value: Any, path: str) -> str:
    text = require_string(value, path, 255)
    if text in {".", ".."} or Path(text).name != text or "/" in text or "\\" in text:
        fail(f"{path} must be a portable basename")
    return text


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


def read_json_object(path: Path, label: str) -> tuple[dict[str, Any], str]:
    stable = read_stable_regular_file(path, MAX_MANIFEST_BYTES, label)
    try:
        value = json.loads(
            stable.content.decode("utf-8"),
            object_pairs_hook=duplicate_rejecting_object,
        )
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        fail(f"cannot parse {label}: {exc}")
    if not isinstance(value, dict):
        fail(f"{label} must be a JSON object")
    bounded_tree(value)
    return value, stable.sha256


def load_contract(path: Path) -> dict[str, Any]:
    contract, _ = read_json_object(path, "source bundle contract")
    require_exact_keys(
        contract,
        {
            "schema_version",
            "manifest_schema",
            "status",
            "archive",
            "source",
            "manifest",
            "authority",
            "claims_not_established",
        },
        "contract",
    )
    if contract.get("schema_version") != CONTRACT_SCHEMA:
        fail("source bundle contract schema is unsupported")
    if contract.get("manifest_schema") != MANIFEST_SCHEMA:
        fail("source bundle manifest schema drifted")
    if contract.get("status") != "normative_release_source_bundle_contract":
        fail("source bundle contract status drifted")

    archive = require_object(contract.get("archive"), "contract.archive")
    require_exact_keys(
        archive,
        {
            "format",
            "hash_algorithm",
            "maximum_bytes",
            "maximum_entries",
            "maximum_entry_bytes",
            "maximum_total_content_bytes",
            "required_prefix",
            "allowed_entry_types",
            "symbolic_links_allowed",
            "hard_links_allowed",
            "special_files_allowed",
            "duplicate_paths_allowed",
            "absolute_paths_allowed",
            "parent_traversal_allowed",
        },
        "contract.archive",
    )
    if archive.get("format") != "ustar_or_pax_tar_from_git_archive":
        fail("source bundle archive format drifted")
    if archive.get("hash_algorithm") != "sha256":
        fail("source bundle hash algorithm drifted")
    for field in [
        "maximum_bytes",
        "maximum_entries",
        "maximum_entry_bytes",
        "maximum_total_content_bytes",
    ]:
        require_positive_int(archive.get(field), f"contract.archive.{field}")
    if archive.get("required_prefix") != "dwarf_fortress_mcp-<40-hex-commit>/":
        fail("source bundle required prefix drifted")
    if archive.get("allowed_entry_types") != ["directory", "regular_file"]:
        fail("source bundle allowed entry types drifted")
    for field in [
        "symbolic_links_allowed",
        "hard_links_allowed",
        "special_files_allowed",
        "duplicate_paths_allowed",
        "absolute_paths_allowed",
        "parent_traversal_allowed",
    ]:
        if archive.get(field) is not False:
            fail(f"source bundle contract unexpectedly permits {field}")

    source = require_object(contract.get("source"), "contract.source")
    require_exact_keys(
        source,
        {
            "must_be_clean_git_commit",
            "submodules_allowed",
            "symbolic_links_allowed",
            "tracked_regular_files_only",
            "entry_order",
            "file_modes",
            "content_digest",
        },
        "contract.source",
    )
    if source.get("must_be_clean_git_commit") is not True:
        fail("source bundle contract does not require a clean commit")
    if source.get("submodules_allowed") is not False:
        fail("source bundle contract permits submodules")
    if source.get("symbolic_links_allowed") is not False:
        fail("source bundle contract permits symbolic links")
    if source.get("tracked_regular_files_only") is not True:
        fail("source bundle contract permits non-regular tracked entries")
    if source.get("entry_order") != "strict_utf8_path_byte_order":
        fail("source bundle entry order drifted")
    if source.get("file_modes") != ["100644", "100755"]:
        fail("source bundle file mode set drifted")
    if source.get("content_digest") != "sha256_of_canonical_entries_array":
        fail("source bundle entries digest rule drifted")

    manifest = require_object(contract.get("manifest"), "contract.manifest")
    require_exact_keys(
        manifest,
        {
            "timestamps_allowed",
            "absolute_paths_allowed",
            "machine_local_paths_allowed",
            "digest_covers",
            "archive_name_must_be_basename",
        },
        "contract.manifest",
    )
    if manifest.get("timestamps_allowed") is not False:
        fail("source bundle manifest permits timestamps")
    if manifest.get("absolute_paths_allowed") is not False:
        fail("source bundle manifest permits absolute paths")
    if manifest.get("machine_local_paths_allowed") is not False:
        fail("source bundle manifest permits machine-local paths")
    if manifest.get("digest_covers") != "all fields except manifest_digest":
        fail("source bundle manifest digest rule drifted")
    if manifest.get("archive_name_must_be_basename") is not True:
        fail("source bundle archive basename rule drifted")

    authority = require_object(contract.get("authority"), "contract.authority")
    require_exact_keys(
        authority,
        {
            "executes_project_code",
            "modifies_source",
            "network_access",
            "grants_capabilities",
            "mutation_capabilities",
        },
        "contract.authority",
    )
    for field in ["executes_project_code", "modifies_source", "network_access"]:
        if authority.get(field) is not False:
            fail(f"source bundle contract grants {field}")
    if authority.get("grants_capabilities") != []:
        fail("source bundle contract grants capabilities")
    if authority.get("mutation_capabilities") != []:
        fail("source bundle contract grants mutation capability")
    if contract.get("claims_not_established") != CLAIMS_NOT_ESTABLISHED:
        fail("source bundle claims-not-established set drifted")
    return contract


def validate_manifest(
    path: Path,
    contract: dict[str, Any],
) -> tuple[dict[str, Any], str]:
    manifest, file_sha256 = read_json_object(path, "source bundle manifest")
    require_exact_keys(
        manifest,
        {
            "schema",
            "status",
            "repository",
            "commit",
            "tree",
            "archive",
            "entries",
            "entries_digest",
            "claims_not_established",
            "manifest_digest",
        },
        "manifest",
    )
    if manifest.get("schema") != MANIFEST_SCHEMA:
        fail("source bundle manifest schema is unsupported")
    if manifest.get("status") != "created":
        fail("source bundle manifest status is not created")
    if manifest.get("repository") != "dwarf_fortress_mcp":
        fail("source bundle manifest names the wrong repository")
    commit = require_commit(manifest.get("commit"), "manifest.commit")
    tree = require_commit(manifest.get("tree"), "manifest.tree")

    archive = require_object(manifest.get("archive"), "manifest.archive")
    require_exact_keys(
        archive,
        {"name", "format", "prefix", "bytes", "sha256"},
        "manifest.archive",
    )
    archive_name = validate_basename(archive.get("name"), "manifest.archive.name")
    if archive.get("format") != "tar":
        fail("source bundle manifest archive format is not tar")
    prefix = require_string(archive.get("prefix"), "manifest.archive.prefix", 128)
    expected_prefix = f"dwarf_fortress_mcp-{commit}/"
    if prefix != expected_prefix:
        fail("source bundle manifest prefix does not match its commit")
    archive_limit = require_positive_int(
        contract["archive"]["maximum_bytes"],
        "contract.archive.maximum_bytes",
    )
    archive_bytes = require_positive_int(
        archive.get("bytes"),
        "manifest.archive.bytes",
        archive_limit,
    )
    archive_sha256 = require_hash(archive.get("sha256"), "manifest.archive.sha256")

    raw_entries = require_list(manifest.get("entries"), "manifest.entries")
    maximum_entries = require_positive_int(
        contract["archive"]["maximum_entries"],
        "contract.archive.maximum_entries",
    )
    if not raw_entries or len(raw_entries) > maximum_entries:
        fail(f"manifest.entries must contain 1..={maximum_entries} entries")
    maximum_entry_bytes = require_positive_int(
        contract["archive"]["maximum_entry_bytes"],
        "contract.archive.maximum_entry_bytes",
    )
    maximum_total_bytes = require_positive_int(
        contract["archive"]["maximum_total_content_bytes"],
        "contract.archive.maximum_total_content_bytes",
    )
    entries: list[dict[str, Any]] = []
    previous_path_bytes: bytes | None = None
    total_bytes = 0
    for index, raw in enumerate(raw_entries):
        entry = require_object(raw, f"manifest.entries[{index}]")
        require_exact_keys(
            entry,
            {"path", "mode", "bytes", "sha256"},
            f"manifest.entries[{index}]",
        )
        entry_path = validate_relative_path(
            entry.get("path"),
            f"manifest.entries[{index}].path",
        )
        path_bytes = entry_path.encode("utf-8")
        if previous_path_bytes is not None and path_bytes <= previous_path_bytes:
            fail("source bundle entries are not in strict UTF-8 path-byte order")
        previous_path_bytes = path_bytes
        mode = require_string(entry.get("mode"), f"manifest.entries[{index}].mode", 6)
        if mode not in ALLOWED_MODES:
            fail(f"manifest.entries[{index}].mode is unsupported")
        size = require_nonnegative_int(
            entry.get("bytes"),
            f"manifest.entries[{index}].bytes",
            maximum_entry_bytes,
        )
        total_bytes += size
        if total_bytes > maximum_total_bytes:
            fail("source bundle manifest exceeds the total content-byte bound")
        entries.append(
            {
                "path": entry_path,
                "mode": mode,
                "bytes": size,
                "sha256": require_hash(
                    entry.get("sha256"),
                    f"manifest.entries[{index}].sha256",
                ),
            }
        )
    declared_entries_digest = require_hash(
        manifest.get("entries_digest"),
        "manifest.entries_digest",
    )
    if declared_entries_digest != sha256_bytes(canonical_json(entries)):
        fail("source bundle entries do not reproduce entries_digest")
    if manifest.get("claims_not_established") != contract.get("claims_not_established"):
        fail("source bundle manifest claims-not-established set drifted")
    declared_manifest_digest = require_hash(
        manifest.get("manifest_digest"),
        "manifest.manifest_digest",
    )
    unsigned = dict(manifest)
    del unsigned["manifest_digest"]
    if declared_manifest_digest != sha256_bytes(canonical_json(unsigned)):
        fail("source bundle manifest fields do not reproduce manifest_digest")
    return (
        {
            "schema": MANIFEST_SCHEMA,
            "status": "created",
            "repository": "dwarf_fortress_mcp",
            "commit": commit,
            "tree": tree,
            "archive": {
                "name": archive_name,
                "format": "tar",
                "prefix": prefix,
                "bytes": archive_bytes,
                "sha256": archive_sha256,
            },
            "entries": entries,
            "entries_digest": declared_entries_digest,
            "claims_not_established": list(contract["claims_not_established"]),
            "manifest_digest": declared_manifest_digest,
        },
        file_sha256,
    )


def validate_tar_member_name(name: str, prefix: str) -> tuple[str, bool]:
    if not name or len(name.encode("utf-8")) > 8192:
        fail("source archive contains an empty or overlong member path")
    if "\\" in name or name.startswith("/") or "//" in name:
        fail(f"source archive member path is not canonical: {name!r}")
    if any(ord(character) < 0x20 for character in name):
        fail("source archive member path contains a control character")
    parts = PurePosixPath(name).parts
    if any(part in {"", ".", ".."} for part in parts):
        fail(f"source archive member path contains dot or parent traversal: {name!r}")
    if not name.startswith(prefix):
        fail(f"source archive member lies outside the required prefix: {name!r}")
    if name == prefix.rstrip("/") or name == prefix:
        return "", True
    relative = name[len(prefix) :]
    is_directory_syntax = relative.endswith("/")
    relative = relative.rstrip("/")
    if not relative:
        return "", True
    validate_relative_path(relative, "source_archive.member.relative_path")
    return relative, is_directory_syntax


def verify_archive(
    archive_path: Path,
    manifest: dict[str, Any],
    contract: dict[str, Any],
) -> dict[str, Any]:
    maximum_archive_bytes = require_positive_int(
        contract["archive"]["maximum_bytes"],
        "contract.archive.maximum_bytes",
    )
    stable = read_stable_regular_file(
        archive_path,
        maximum_archive_bytes,
        "source bundle archive",
    )
    expected_archive = manifest["archive"]
    if archive_path.name != expected_archive["name"]:
        fail("source archive filename differs from the manifest basename")
    if stable.size != expected_archive["bytes"]:
        fail("source archive byte length differs from the manifest")
    if stable.sha256 != expected_archive["sha256"]:
        fail("source archive SHA-256 differs from the manifest")

    expected_entries = {entry["path"]: entry for entry in manifest["entries"]}
    observed_files: set[str] = set()
    observed_member_names: set[str] = set()
    observed_directories: set[str] = set()
    maximum_entries = require_positive_int(
        contract["archive"]["maximum_entries"],
        "contract.archive.maximum_entries",
    )
    maximum_entry_bytes = require_positive_int(
        contract["archive"]["maximum_entry_bytes"],
        "contract.archive.maximum_entry_bytes",
    )
    maximum_total_bytes = require_positive_int(
        contract["archive"]["maximum_total_content_bytes"],
        "contract.archive.maximum_total_content_bytes",
    )
    total_content_bytes = 0
    member_count = 0
    prefix = manifest["archive"]["prefix"]

    try:
        archive = tarfile.open(fileobj=io.BytesIO(stable.content), mode="r:")
    except tarfile.TarError as exc:
        fail(f"cannot parse source archive: {exc}")
    with archive:
        try:
            for member in archive:
                member_count += 1
                if member_count > maximum_entries:
                    fail("source archive exceeds the member-count bound")
                if member.name in observed_member_names:
                    fail(f"source archive repeats member path {member.name!r}")
                observed_member_names.add(member.name)
                relative, directory_syntax = validate_tar_member_name(member.name, prefix)
                if member.issym():
                    fail(f"source archive contains symbolic link {member.name!r}")
                if member.islnk():
                    fail(f"source archive contains hard link {member.name!r}")
                if member.isdev() or member.isfifo():
                    fail(f"source archive contains special file {member.name!r}")
                if member.isdir():
                    if not directory_syntax and relative:
                        fail(f"source archive directory lacks canonical trailing slash: {member.name!r}")
                    observed_directories.add(relative)
                    continue
                if not member.isfile():
                    fail(f"source archive contains unsupported member type {member.name!r}")
                if directory_syntax or not relative:
                    fail(f"source archive regular file has invalid path {member.name!r}")
                expected = expected_entries.get(relative)
                if expected is None:
                    fail(f"source archive contains unmanifested file {relative!r}")
                if relative in observed_files:
                    fail(f"source archive repeats file {relative!r}")
                if member.size < 0 or member.size > maximum_entry_bytes:
                    fail(f"source archive member {relative!r} exceeds its size bound")
                if member.size != expected["bytes"]:
                    fail(f"source archive member {relative!r} size differs from the manifest")
                expected_mode = 0o755 if expected["mode"] == "100755" else 0o644
                if member.mode & 0o777 != expected_mode:
                    fail(f"source archive member {relative!r} mode differs from the manifest")
                extracted = archive.extractfile(member)
                if extracted is None:
                    fail(f"cannot read source archive member {relative!r}")
                digest = hashlib.sha256()
                read_bytes = 0
                while True:
                    chunk = extracted.read(min(1024 * 1024, maximum_entry_bytes + 1 - read_bytes))
                    if not chunk:
                        break
                    read_bytes += len(chunk)
                    if read_bytes > maximum_entry_bytes:
                        fail(f"source archive member {relative!r} grew beyond its bound")
                    digest.update(chunk)
                if read_bytes != member.size:
                    fail(f"source archive member {relative!r} ended at the wrong length")
                if digest.hexdigest() != expected["sha256"]:
                    fail(f"source archive member {relative!r} SHA-256 differs from the manifest")
                total_content_bytes += read_bytes
                if total_content_bytes > maximum_total_bytes:
                    fail("source archive exceeds the total content-byte bound")
                observed_files.add(relative)
        except (tarfile.TarError, OSError) as exc:
            fail(f"cannot read source archive: {exc}")

    missing = sorted(set(expected_entries) - observed_files)
    if missing:
        fail("source archive omits manifested files: " + ", ".join(missing[:16]))
    for directory in observed_directories:
        if not directory:
            continue
        prefix_path = directory + "/"
        if not any(path.startswith(prefix_path) for path in expected_entries):
            fail(f"source archive contains orphan directory {directory!r}")
    return {
        "file_sha256": stable.sha256,
        "bytes": stable.size,
        "member_count": member_count,
        "regular_file_count": len(observed_files),
        "total_content_bytes": total_content_bytes,
    }


def run_git(source_root: Path, arguments: list[str], *, binary: bool = False) -> bytes | str:
    try:
        completed = subprocess.run(
            ["git", "-C", os.fspath(source_root), *arguments],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=not binary,
        )
    except OSError as exc:
        fail(f"cannot execute Git for source reconciliation: {exc}")
    if completed.returncode != 0:
        stderr = completed.stderr
        if isinstance(stderr, bytes):
            detail = stderr.decode("utf-8", errors="replace")
        else:
            detail = stderr
        fail(f"Git source reconciliation failed: {detail.strip()[:1024]}")
    return completed.stdout


def git_entries(source_root: Path, commit: str) -> tuple[str, list[dict[str, Any]]]:
    tree_raw = run_git(source_root, ["rev-parse", f"{commit}^{{tree}}"])
    if not isinstance(tree_raw, str):
        fail("Git tree identity unexpectedly returned binary output")
    tree = require_commit(tree_raw.strip(), "git.tree")
    listing = run_git(source_root, ["ls-tree", "-rz", "--full-tree", commit], binary=True)
    if not isinstance(listing, bytes):
        fail("Git tree listing unexpectedly returned text output")
    entries: list[dict[str, Any]] = []
    total_bytes = 0
    for index, record in enumerate(listing.split(b"\0")):
        if not record:
            continue
        try:
            metadata, raw_path = record.split(b"\t", 1)
            mode_raw, kind_raw, object_raw = metadata.split(b" ", 2)
            mode = mode_raw.decode("ascii")
            kind = kind_raw.decode("ascii")
            object_id = object_raw.decode("ascii")
            entry_path = raw_path.decode("utf-8")
        except (ValueError, UnicodeDecodeError) as exc:
            fail(f"cannot parse Git tree entry {index}: {exc}")
        if kind != "blob" or mode not in ALLOWED_MODES:
            fail(
                f"Git source contains unsupported tracked entry {entry_path!r} "
                f"with mode {mode!r} and kind {kind!r}"
            )
        require_commit(object_id, f"git.entries[{index}].object_id")
        canonical_path = validate_relative_path(entry_path, f"git.entries[{index}].path")
        content = run_git(source_root, ["cat-file", "blob", object_id], binary=True)
        if not isinstance(content, bytes):
            fail("Git blob unexpectedly returned text output")
        total_bytes += len(content)
        if total_bytes > 1024 * 1024 * 1024:
            fail("Git source tree exceeds the one-GiB reconciliation bound")
        entries.append(
            {
                "path": canonical_path,
                "mode": mode,
                "bytes": len(content),
                "sha256": sha256_bytes(content),
            }
        )
    entries.sort(key=lambda entry: entry["path"].encode("utf-8"))
    if not entries:
        fail("Git source tree contains no regular files")
    return tree, entries


def verify_checkout(
    source_root: Path,
    manifest: dict[str, Any],
    *,
    require_clean: bool,
) -> dict[str, Any]:
    if not source_root.is_absolute():
        fail("source-root path must be absolute")
    head_raw = run_git(source_root, ["rev-parse", "HEAD"])
    if not isinstance(head_raw, str):
        fail("Git HEAD unexpectedly returned binary output")
    head = require_commit(head_raw.strip(), "git.head")
    if head != manifest["commit"]:
        fail("source checkout HEAD differs from the bundle manifest commit")
    if require_clean:
        status = run_git(source_root, ["status", "--porcelain=v1", "--untracked-files=all"])
        if not isinstance(status, str):
            fail("Git status unexpectedly returned binary output")
        if status:
            fail("source checkout is not clean")
    tree, entries = git_entries(source_root, manifest["commit"])
    if tree != manifest["tree"]:
        fail("source checkout tree differs from the bundle manifest tree")
    if entries != manifest["entries"]:
        fail("source checkout entries differ from the bundle manifest")
    return {
        "head": head,
        "tree": tree,
        "clean": require_clean,
        "entry_count": len(entries),
        "entries_digest": sha256_bytes(canonical_json(entries)),
    }


def verify(
    manifest_path: Path,
    archive_path: Path,
    contract_path: Path = DEFAULT_CONTRACT,
    source_root: Path | None = None,
    require_clean_source: bool = False,
) -> dict[str, Any]:
    contract = load_contract(contract_path)
    manifest, manifest_file_sha256 = validate_manifest(manifest_path, contract)
    archive = verify_archive(archive_path, manifest, contract)
    checkout = None
    if source_root is not None:
        checkout = verify_checkout(
            source_root,
            manifest,
            require_clean=require_clean_source,
        )
    unsigned = {
        "schema": "dfmcp.source-bundle-verification/1",
        "status": "verified",
        "manifest": {
            "file_sha256": manifest_file_sha256,
            "content_digest": manifest["manifest_digest"],
            "commit": manifest["commit"],
            "tree": manifest["tree"],
            "entries_digest": manifest["entries_digest"],
            "entry_count": len(manifest["entries"]),
        },
        "archive": archive,
        "checkout": checkout,
        "authority": {
            "executes_project_code": False,
            "modifies_source": False,
            "network_access": False,
            "grants_capabilities": [],
            "mutation_capabilities": [],
        },
        "claims_not_established": list(contract["claims_not_established"]),
    }
    return {
        **unsigned,
        "verification_digest": sha256_bytes(canonical_json(unsigned)),
    }


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--source-root", type=Path)
    parser.add_argument("--require-clean-source", action="store_true")
    parser.add_argument("--output", type=Path)
    return parser.parse_args(argv)


def write_atomic(path: Path, value: dict[str, Any]) -> None:
    parent = path.parent
    parent.mkdir(parents=True, exist_ok=True)
    payload = json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    temporary = parent / f".{path.name}.{os.getpid()}.tmp"
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    descriptor = os.open(temporary, flags, 0o600)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
        directory = os.open(parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    except BaseException:
        try:
            temporary.unlink()
        except OSError:
            pass
        raise


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        result = verify(
            args.manifest,
            args.archive,
            args.contract,
            args.source_root,
            args.require_clean_source,
        )
        if args.output is None:
            print(json.dumps(result, sort_keys=True, separators=(",", ":"), ensure_ascii=False))
        else:
            write_atomic(args.output, result)
    except (OSError, StableReadError, SourceBundleError) as exc:
        print(f"source bundle verification: FAIL: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
