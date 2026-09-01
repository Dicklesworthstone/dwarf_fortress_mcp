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
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, NoReturn

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONTRACT = ROOT / "architecture/live_server_binary_receipt_v1.json"
RECEIPT_SCHEMA = "dfmcp.live-server-binary-qualification/1"
LOCAL_RECEIPT_SCHEMA = "dfmcp.qualification-receipt.v1"
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
MAX_JSON_BYTES = 8 * 1024 * 1024
MAX_HASHED_FILE_BYTES = 256 * 1024 * 1024
MAX_STRING_BYTES = 16 * 1024
MAX_COLLECTION_ITEMS = 4096
MAX_DEPTH = 64

EXPECTED_LOCAL_QUALIFICATION_GATES = [
    "repository-integrity",
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


def same_identity(left: os.stat_result, right: os.stat_result) -> bool:
    return (
        left.st_dev == right.st_dev
        and left.st_ino == right.st_ino
        and left.st_size == right.st_size
        and left.st_mode == right.st_mode
        and left.st_uid == right.st_uid
        and left.st_mtime_ns == right.st_mtime_ns
        and left.st_ctime_ns == right.st_ctime_ns
    )


def open_stable_regular(
    path: Path,
    maximum_bytes: int,
    label: str,
) -> tuple[int, os.stat_result]:
    raw = os.fspath(path)
    if not raw or len(os.fsencode(raw)) > 4096:
        fail(f"{label} path is empty or exceeds its byte bound")
    if any(ord(character) < 0x20 for character in raw):
        fail(f"{label} path contains a control character")
    if not hasattr(os, "O_NOFOLLOW"):
        fail("this platform cannot enforce no-follow artifact opening")
    flags = os.O_RDONLY | os.O_NOFOLLOW
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    try:
        descriptor = os.open(path, flags)
    except OSError as exc:
        fail(f"cannot open {label}: {exc}")
    try:
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode):
            fail(f"{label} must be a regular file")
        if metadata.st_size <= 0 or metadata.st_size > maximum_bytes:
            fail(
                f"{label} must contain 1..={maximum_bytes} bytes, got {metadata.st_size}"
            )
        return descriptor, metadata
    except BaseException:
        os.close(descriptor)
        raise


def read_bytes_with_digest(
    path: Path,
    label: str,
    maximum_bytes: int = MAX_JSON_BYTES,
) -> tuple[bytes, str]:
    descriptor, before = open_stable_regular(path, maximum_bytes, label)
    try:
        digest = hashlib.sha256()
        chunks: list[bytes] = []
        total = 0
        while True:
            chunk = os.read(
                descriptor,
                min(1024 * 1024, maximum_bytes + 1 - total),
            )
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
        return b"".join(chunks), digest.hexdigest()
    finally:
        os.close(descriptor)


def sha256_file(path: Path) -> str:
    _, digest = read_bytes_with_digest(
        path,
        "source-bound file",
        MAX_HASHED_FILE_BYTES,
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


def read_object_with_digest(
    path: Path,
    label: str,
    maximum_bytes: int = MAX_JSON_BYTES,
) -> tuple[dict[str, Any], str]:
    raw, digest = read_bytes_with_digest(path, label, maximum_bytes)
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
    return value, digest


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
        fail(f"{path} must be a lowercase 40-character Git commit")
    return text


def require_positive_int(value: Any, path: str, maximum: int | None = None) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        fail(f"{path} must be a positive integer")
    if maximum is not None and value > maximum:
        fail(f"{path} must be <= {maximum}")
    return value


def validate_relative_path(value: Any, path: str) -> str:
    text = require_string(value, path, 1024)
    candidate = Path(text)
    if candidate.is_absolute() or ".." in candidate.parts:
        fail(f"{path} must be a traversal-free relative path")
    if text.startswith("./") or "//" in text or candidate.as_posix() != text:
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


def validate_local_qualification_receipt(
    path: Path,
    expected_commit: str,
    expected_sha256: str,
    expected_gates: list[str] | None = None,
) -> dict[str, Any]:
    receipt, actual_sha256 = read_object_with_digest(
        path,
        "local qualification receipt",
    )
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
    require_exact_keys(source, {"commit", "dirty"}, "local_receipt.source")
    if source.get("commit") != expected_commit:
        fail("local qualification receipt names a different source commit")
    if source.get("dirty") is not False:
        fail("local qualification receipt is not bound to a clean source tree")
    require_object(receipt.get("host"), "local_receipt.host")
    require_object(receipt.get("toolchain"), "local_receipt.toolchain")
    require_object(receipt.get("digests"), "local_receipt.digests")

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
    validate_local_qualification_receipt(
        local_qualification_receipt,
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
    if Path(relative_path).name not in {
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
        actual = sha256_file(source_root / canonical_relative)
        if declared != actual:
            fail(f"server receipt source digest differs for {canonical_relative}")
        normalized_digests[name] = declared

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
