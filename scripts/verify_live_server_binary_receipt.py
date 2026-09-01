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
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONTRACT = ROOT / "architecture/live_server_binary_receipt_v1.json"
RECEIPT_SCHEMA = "dfmcp.live-server-binary-qualification/1"
LOCAL_RECEIPT_SCHEMA = "dfmcp.qualification-receipt.v1"
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
MAX_JSON_BYTES = 8 * 1024 * 1024
MAX_STRING_BYTES = 16 * 1024
MAX_COLLECTION_ITEMS = 4096
MAX_DEPTH = 64


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


def fail(message: str) -> None:
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


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sha256_descriptor(descriptor: int) -> str:
    duplicate = os.dup(descriptor)
    try:
        os.lseek(duplicate, 0, os.SEEK_SET)
        digest = hashlib.sha256()
        with os.fdopen(duplicate, "rb", closefd=True) as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
        duplicate = -1
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


def read_object(path: Path, label: str, maximum_bytes: int = MAX_JSON_BYTES) -> dict[str, Any]:
    try:
        size = path.stat().st_size
    except OSError as exc:
        fail(f"cannot stat {label}: {exc}")
    if size <= 0 or size > maximum_bytes:
        fail(f"{label} must contain 1..={maximum_bytes} bytes, got {size}")
    try:
        raw = path.read_bytes()
        value = json.loads(
            raw.decode("utf-8"), object_pairs_hook=duplicate_rejecting_object
        )
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        fail(f"cannot parse {label}: {exc}")
    if not isinstance(value, dict):
        fail(f"{label} must be a JSON object")
    bounded_tree(value)
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


def load_contract(path: Path) -> dict[str, Any]:
    contract = read_object(path, "live server binary receipt contract", 1024 * 1024)
    if contract.get("schema_version") != "dfmcp.live-server-binary-receipt-contract/1":
        fail("live server binary contract schema is unsupported")
    if contract.get("receipt_schema") != RECEIPT_SCHEMA:
        fail("live server binary receipt schema drifted")
    source_binding = require_object(contract.get("source_binding"), "contract.source_binding")
    if source_binding.get("requires_clean_dfmcp_source") is not True:
        fail("binary contract must require clean dfmcp source")
    if source_binding.get("requires_passing_local_qualification_receipt") is not True:
        fail("binary contract must require a passing local qualification receipt")
    required_digests = require_object(
        source_binding.get("required_source_digests"),
        "contract.source_binding.required_source_digests",
    )
    if not required_digests:
        fail("binary contract has no source-digest bindings")
    binary = require_object(contract.get("binary"), "contract.binary")
    if binary.get("hash_algorithm") != "sha256" or binary.get("profile") != "release":
        fail("binary contract must require a SHA-256 release artifact")
    if binary.get("symbolic_links_allowed") is not False:
        fail("binary contract must reject symbolic links")
    if binary.get("group_or_world_writable_allowed") is not False:
        fail("binary contract must reject group/world-writable artifacts")
    checks = require_list(contract.get("required_executable_checks"), "contract.required_executable_checks")
    if checks != ["contract", "doctor", "demo"]:
        fail("binary executable-check set or order drifted")
    authority = require_object(contract.get("authority"), "contract.authority")
    if authority.get("mutation_capabilities") != []:
        fail("binary contract must carry no mutation capabilities")
    return contract


def validate_local_qualification_receipt(
    path: Path, expected_commit: str, expected_sha256: str
) -> dict[str, Any]:
    if sha256_file(path) != expected_sha256:
        fail("local qualification receipt bytes do not match the server receipt binding")
    receipt = read_object(path, "local qualification receipt")
    if receipt.get("schema") != LOCAL_RECEIPT_SCHEMA:
        fail("local qualification receipt schema is unsupported")
    if receipt.get("status") != "passed":
        fail("local qualification receipt did not pass")
    source = require_object(receipt.get("source"), "local_receipt.source")
    if source.get("commit") != expected_commit:
        fail("local qualification receipt names a different source commit")
    if source.get("dirty") is not False:
        fail("local qualification receipt is not bound to a clean source tree")
    gates = require_list(receipt.get("gates"), "local_receipt.gates")
    if not gates:
        fail("local qualification receipt contains no gates")
    for index, raw in enumerate(gates):
        gate = require_object(raw, f"local_receipt.gates[{index}]")
        if gate.get("state") not in {"passed", "skipped"}:
            fail(f"local qualification gate {gate.get('name')!r} did not pass")
        if gate.get("state") == "skipped":
            fail("a release server receipt cannot depend on skipped local qualification gates")
    return receipt


def validate_relative_build_path(value: Any, path: str) -> str:
    text = require_string(value, path, 1024)
    candidate = Path(text)
    if candidate.is_absolute() or ".." in candidate.parts:
        fail(f"{path} must be a traversal-free relative path")
    if text.startswith("./") or "//" in text:
        fail(f"{path} is not in canonical relative-path form")
    return candidate.as_posix()


def validate_receipt(
    receipt_path: Path,
    contract_path: Path,
    source_root: Path,
    local_qualification_receipt: Path,
    expected_commit: str | None = None,
    require_current_platform: bool = True,
) -> dict[str, Any]:
    contract = load_contract(contract_path)
    receipt = read_object(receipt_path, "live server binary receipt")
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
        receipt.get("receipt_digest"), "server_receipt.receipt_digest"
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
    commit = require_commit(source.get("dfmcp_commit"), "server_receipt.source.dfmcp_commit")
    if expected_commit is not None and commit != require_commit(expected_commit, "expected_commit"):
        fail("server receipt source commit differs from the expected deployment commit")
    if source.get("dfmcp_dirty") is not False:
        fail("server receipt is not bound to a clean source tree")
    local_receipt_sha = require_hash(
        source.get("local_qualification_receipt_sha256"),
        "server_receipt.source.local_qualification_receipt_sha256",
    )
    validate_local_qualification_receipt(
        local_qualification_receipt, commit, local_receipt_sha
    )

    platform_value = require_object(receipt.get("platform"), "server_receipt.platform")
    require_exact_keys(platform_value, {"system", "machine"}, "server_receipt.platform")
    system = require_string(platform_value.get("system"), "server_receipt.platform.system", 128)
    machine = require_string(platform_value.get("machine"), "server_receipt.platform.machine", 128)
    if require_current_platform and (system != platform.system() or machine != platform.machine()):
        fail("server receipt platform differs from the current execution platform")

    toolchain = require_object(receipt.get("toolchain"), "server_receipt.toolchain")
    require_exact_keys(toolchain, {"rustc_vv", "cargo"}, "server_receipt.toolchain")
    require_string(toolchain.get("rustc_vv"), "server_receipt.toolchain.rustc_vv", 16 * 1024)
    require_string(toolchain.get("cargo"), "server_receipt.toolchain.cargo", 1024)

    binary = require_object(receipt.get("binary"), "server_receipt.binary")
    require_exact_keys(
        binary,
        {"name", "profile", "relative_path", "bytes", "sha256"},
        "server_receipt.binary",
    )
    contract_binary = contract["binary"]
    if binary.get("name") != contract_binary["name"] or binary.get("profile") != "release":
        fail("server receipt identifies the wrong binary or build profile")
    relative_path = validate_relative_build_path(
        binary.get("relative_path"), "server_receipt.binary.relative_path"
    )
    if Path(relative_path).name not in {"dwarf-fortress-mcp", "dwarf-fortress-mcp.exe"}:
        fail("server receipt relative path has the wrong executable filename")
    binary_bytes = require_positive_int(
        binary.get("bytes"),
        "server_receipt.binary.bytes",
        require_positive_int(contract_binary.get("maximum_bytes"), "contract.binary.maximum_bytes"),
    )
    binary_sha256 = require_hash(binary.get("sha256"), "server_receipt.binary.sha256")

    checks = require_list(receipt.get("executable_checks"), "server_receipt.executable_checks")
    expected_checks = contract["required_executable_checks"]
    if len(checks) != len(expected_checks):
        fail("server receipt executable-check count drifted")
    normalized_checks: list[dict[str, Any]] = []
    for index, expected_name in enumerate(expected_checks):
        check = require_object(checks[index], f"server_receipt.executable_checks[{index}]")
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

    source_digests = require_object(receipt.get("source_digests"), "server_receipt.source_digests")
    required_digests = contract["source_binding"]["required_source_digests"]
    if set(source_digests) != set(required_digests):
        fail("server receipt source-digest key set drifted")
    normalized_digests: dict[str, str] = {}
    for name, relative in required_digests.items():
        declared = require_hash(
            source_digests.get(name), f"server_receipt.source_digests.{name}"
        )
        path = source_root / relative
        if not path.is_file():
            fail(f"server receipt source binding is missing file {relative}")
        actual = sha256_file(path)
        if declared != actual:
            fail(f"server receipt source digest differs for {relative}")
        normalized_digests[name] = declared

    if receipt.get("mutation_capabilities") != []:
        fail("server receipt must carry no mutation capabilities")
    if receipt.get("claims_not_established") != contract.get("claims_not_established"):
        fail("server receipt claims-not-established set drifted")

    return {
        "receipt_sha256": sha256_file(receipt_path),
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
    flags = os.O_RDONLY
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(candidate, flags)
    except OSError as exc:
        fail(f"cannot open server artifact without following symbolic links: {exc}")
    try:
        metadata = os.fstat(descriptor)
        validate_open_metadata(metadata)
        size = metadata.st_size
        if size != expected["bytes"]:
            fail(f"server artifact size {size} differs from receipt size {expected['bytes']}")
        digest = sha256_descriptor(descriptor)
        if digest != expected["sha256"]:
            fail("server artifact SHA-256 differs from the qualified receipt")
        if not os.access(candidate, os.X_OK):
            fail("server artifact path is not executable")
        return OpenBinary(
            descriptor=descriptor,
            path=candidate,
            sha256=digest,
            size=size,
            device=metadata.st_dev,
            inode=metadata.st_ino,
            mode=stat.S_IMODE(metadata.st_mode),
            owner_uid=metadata.st_uid,
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
