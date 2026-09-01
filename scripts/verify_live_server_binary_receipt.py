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
    "live-server-artifact-admission",
    "dependency-policy",
    "repository-integrity-tests",
    "live-acceptance-tests",
    "live-acceptance-journal-tests",
    "live-acceptance-secret-scanner-tests",
    "live-capture-guidance-tests",
    "live-compatibility-promotion-tests",
    "live-compatibility-resolution-tests",
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
    "mcp_live_server": "crates/dfmcp-mcp/src/live_server.rs",
    "adapter_live_connect": "crates/dfmcp-adapter/src/live_connect.rs",
    "adapter_live_bootstrap": "crates/dfmcp-adapter/src/live_bootstrap.rs",
    "adapter_live_observation": "crates/dfmcp-adapter/src/live_observation.rs",
    "adapter_live_projection": "crates/dfmcp-adapter/src/live_projection.rs",
    "compatibility_registry": "architecture/live_compatibility_registry_v1.json",
    "compatibility_resolver": "scripts/resolve_live_compatibility.py",
    "artifact_contract": "architecture/live_server_binary_receipt_v1.json",
    "artifact_qualification": "scripts/qualify_live_server_binary.sh",
    "artifact_qualification_tests": "scripts/test_qualify_live_server_binary.py",
    "artifact_verifier": "scripts/verify_live_server_binary_receipt.py",
    "artifact_checker": "scripts/check_live_server_artifact.py",
    "artifact_verifier_tests": "scripts/test_live_server_binary_receipt.py",
    "admitted_launcher": "scripts/serve_admitted_live.py",
    "admitted_launcher_tests": "scripts/test_admitted_live_launcher.py",
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


def _open_stable_regular(path: Path, maximum_bytes: int, label: str) -> tuple[int, os.stat_result]:
    raw = os.fspath(path)
    if not raw or len(os.fsencode(raw)) > 4096:
        fail(f"{label} path is empty or exceeds its byte bound")
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
            fail(f"{label} must contain 1..={maximum_bytes} bytes, got {metadata.st_size}")
        return descriptor, metadata
    except BaseException:
        os.close(descriptor)
        raise


def read_bytes_with_digest(
    path: Path, label: str, maximum_bytes: int = MAX_JSON_BYTES
) -> tuple[bytes, str]:
    descriptor, before = _open_stable_regular(path, maximum_bytes, label)
    try:
        digest = hashlib.sha256()
        chunks: list[bytes] = []
        total = 0
        while True:
            chunk = os.read(descriptor, min(1024 * 1024, maximum_bytes + 1 - total))
            if not chunk:
                break
            total += len(chunk)
            if total > maximum_bytes:
                fail(f"{label} grew beyond its byte bound while being read")
            digest.update(chunk)
            chunks.append(chunk)
        after = os.fstat(descriptor)
        if (
            before.st_dev != after.st_dev
            or before.st_ino != after.st_ino
            or before.st_size != after.st_size
            or total != before.st_size
        ):
            fail(f"{label} changed while being read")
        return b"".join(chunks), digest.hexdigest()
    finally:
        os.close(descriptor)


def sha256_file(path: Path) -> str:
    _, digest = read_bytes_with_digest(path, "source-bound file")
    return digest


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


def read_object_with_digest(
    path: Path, label: str, maximum_bytes: int = MAX_JSON_BYTES
) -> tuple[dict[str, Any], str]:
    raw, digest = read_bytes_with_digest(path, label, maximum_bytes)
    try:
        value = json.loads(
            raw.decode("utf-8"), object_pairs_hook=duplicate_rejecting_object
        )
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        fail(f"cannot parse {label}: {exc}")
    if not isinstance(value, dict):
        fail(f"{label} must be a JSON object")
    bounded_tree(value)
    return value, digest


def read_object(path: Path, label: str, maximum_bytes: int = MAX_JSON_BYTES) -> dict[str, Any]:
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


def load_contract(path: Path) -> dict[str, Any]:
    contract = read_object(path, "live server binary receipt contract", 1024 * 1024)
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
        candidate = Path(relative)
        if candidate.is_absolute() or ".." in candidate.parts or relative.startswith("./"):
            fail(f"binary contract source path {name!s¿vÙÈZ®ËkºwµçYXÙZ\˜š[˜\K˜]\È‹ˆ™\]Z\™WÜÜÚ]]™WÚ[
ÛÛ˜XİØš[˜\K™Ù]
›X^[][WØ]\ÈŠJKˆ
Bˆš[˜\WÜÚLMˆH™\]Z\™WÚ\Ú
š[˜\K™Ù]
œÚLMˆŠKœÙ\™\—Ü™XÙZ\˜š[˜\KœÚLMˆŠB‚ˆÚXÚÜÈH™\]Z\™WÛ\İ
™XÙZ\™Ù]
™^Xİ]X›WØÚXÚÜÈŠKœÙ\™\—Ü™XÙZ\™^Xİ]X›WØÚXÚÜÈŠBˆ^XİYØÚXÚÜÈHÛÛ˜XİÈœ™\]Z\™YÙ^Xİ]X›WØÚXÚÜÈ—BˆYˆ[ŠÚXÚÜÊHOH[Š^XİYØÚXÚÜÊN‚ˆ˜Z[
œÙ\™\ˆ™XÙZ\^Xİ]X›KXÚXÚÈÛİ[šYYŠBˆ›Ü›X[^™YØÚXÚÜÎˆ\İÙXİÜİ‹[WWHH×Bˆ›Üˆ[™^^XİYÛ˜[YH[ˆ[[Y\˜]J^XİYØÚXÚÜÊN‚ˆÚXÚÈH™\]Z\™WÛØš™Xİ
ÚXÚÜÖÚ[™^KˆœÙ\™\—Ü™XÙZ\™^Xİ]X›WØÚXÚÜÖŞÚ[™^WHŠBˆ™\]Z\™WÙ^XİÚÙ^\ÊˆÚXÚËˆÈ›˜[YH‹œİ]\È‹œİİ]ÜÚLMˆ‹œİ\œ—ÜÚLMˆŸKˆˆœÙ\™\—Ü™XÙZ\™^Xİ]X›WØÚXÚÜÖŞÚ[™^WH‹ˆ
BˆYˆÚXÚË™Ù]
›˜[YHŠHOH^XİYÛ˜[YHÜˆÚXÚË™Ù]
œİ]\ÈŠHOHœ\ÜÙY‚ˆ˜Z[
ˆœÙ\™\ˆ^Xİ]X›HÚXÚÈÙ^XİYÛ˜[Y_HY›İ\ÜÈ[ˆØ[›ÛšXØ[Ü™\ˆŠBˆ›Ü›X[^™YØÚXÚÜË˜\[™
ˆÂˆ›˜[YHˆ^XİYÛ˜[YKˆœİ]\Èˆœ\ÜÙY‹ˆœİİ]ÜÚLMˆˆ™\]Z\™WÚ\Ú
ˆÚXÚË™Ù]
œİİ]ÜÚLMˆŠKˆˆœÙ\™\—Ü™XÙZ\™^Xİ]X›WØÚXÚÜÖŞÚ[™^WKœİİ]ÜÚLMˆ‹ˆ
Kˆœİ\œ—ÜÚLMˆˆ™\]Z\™WÚ\Ú
ˆÚXÚË™Ù]
œİ\œ—ÜÚLMˆŠKˆˆœÙ\™\—Ü™XÙZ\™^Xİ]X›WØÚXÚÜÖŞÚ[™^WKœİ\œ—ÜÚLMˆ‹ˆ
KˆBˆ
B‚ˆÛİ\˜ÙWÙYÙ\İÈH™\]Z\™WÛØš™Xİ
™XÙZ\™Ù]
œÛİ\˜ÙWÙYÙ\İÈŠKœÙ\™\—Ü™XÙZ\œÛİ\˜ÙWÙYÙ\İÈŠBˆ™\]Z\™YÙYÙ\İÈHÛÛ˜XİÈœÛİ\˜ÙWØš[™[™È—VÈœ™\]Z\™YÜÛİ\˜ÙWÙYÙ\İÈ—BˆYˆÙ]
Ûİ\˜ÙWÙYÙ\İÊHOHÙ]
™\]Z\™YÙYÙ\İÊN‚ˆ˜Z[
œÙ\™\ˆ™XÙZ\Ûİ\˜ÙKYYÙ\İÙ^HÙ]šYYŠBˆ›Ü›X[^™YÙYÙ\İÎˆXİÜİ‹İ—HHßBˆ›Üˆ˜[YK™[]]™H[ˆ™\]Z\™YÙYÙ\İËš][\Ê
N‚ˆXÛ\™YH™\]Z\™WÚ\Ú
ˆÛİ\˜ÙWÙYÙ\İË™Ù]
˜[YJKˆœÙ\™\—Ü™XÙZ\œÛİ\˜ÙWÙYÙ\İËÛ˜[Y_H‚ˆ
Bˆ]HÛİ\˜ÙWÜ›ÛİÈ™[]]™BˆYˆ›İ]š\×Ùš[J
N‚ˆ˜Z[
ˆœÙ\™\ˆ™XÙZ\Ûİ\˜ÙHš[™[™È\ÈZ\ÜÚ[™Èš[HÜ™[]]™_HŠBˆXİX[HÚLM—Ùš[J]
BˆYˆXÛ\™YOHXİX[‚ˆ˜Z[
ˆœÙ\™\ˆ™XÙZ\Ûİ\˜ÙHYÙ\İY™™\œÈ›ÜˆÜ™[]]™_HŠBˆ›Ü›X[^™YÙYÙ\İÖÛ˜[YWHHXÛ\™Y‚ˆYˆ™XÙZ\™Ù]
›]]][Û—ØØ\Xš[]Y\ÈŠHOH×N‚ˆ˜Z[
œÙ\™\ˆ™XÙZ\]\İØ\œH›È]]][ÛˆØ\Xš[]Y\ÈŠBˆYˆ™XÙZ\™Ù]
˜ÛZ[\×Û›İÙ\İX›\ÚYŠHOHÛÛ˜Xİ™Ù]
˜ÛZ[\×Û›İÙ\İX›\ÚYŠN‚ˆ˜Z[
œÙ\™\ˆ™XÙZ\ÛZ[\Ë[›İY\İX›\ÚYÙ]šYYŠB‚ˆ™]\›ˆÂˆœ™XÙZ\ÜÚLMˆˆ™XÙZ\Ùš[WÜÚLM‹ˆœ™XÙZ\ÙYÙ\İˆXÛ\™YÜ™XÙZ\ÙYÙ\İˆœÛİ\˜ÙHˆÂˆ™›XÜØÛÛ[Z]ˆÛÛ[Z]ˆ™›XÜÙ\Hˆ˜[ÙKˆ›ØØ[Ü]X[YšXØ][Û—Ü™XÙZ\ÜÚLMˆˆØØ[Ü™XÙZ\ÜÚKˆKˆœ]›Ü›HˆÈœŞ\İ[HˆŞ\İ[K›XXÚ[™HˆXXÚ[™_Kˆ˜š[˜\HˆÂˆ›˜[YHˆš[˜\VÈ›˜[YH—Kˆœ›Ùš[Hˆœ™[X\ÙH‹ˆœ™[]]™WÜ]ˆ™[]]™WÜ]ˆ˜]\Èˆš[˜\WØ]\ËˆœÚLMˆˆš[˜\WÜÚLM‹ˆKˆ™^Xİ]X›WØÚXÚÜÈˆ›Ü›X[^™YØÚXÚÜËˆœÛİ\˜ÙWÙYÙ\İÈˆ›Ü›X[^™YÙYÙ\İËˆ›]]][Û—ØØ\Xš[]Y\Èˆ×KˆB‚‚™Yˆ˜[Y]WÛÜ[—ÛY]Y]JY]Y]NˆÜËœİ]Ü™\İ[
HOˆ›Û™N‚ˆYˆ›İİ]”×ÒTÔ‘QÊY]Y]KœİÛ[ÙJN‚ˆ˜Z[
›Ü[™YÙ\™\ˆ\Y˜Xİ\È›İH™Yİ[\ˆš[HŠBˆYˆY]Y]KœİÛ[ÙH	ˆ
İ]”×ÒUÑÔ”İ]”×ÒUÓÕ
N‚ˆ˜Z[
œÙ\™\ˆ\Y˜Xİ\ÈÜ›İ\HÜˆÛÜ›]Üš]X›HŠBˆYˆY]Y]KœİÛ[ÙH	ˆ
İ]”×ÒVTÔˆİ]”×ÒVÔ”İ]”×ÒVÕ
HOH‚ˆ˜Z[
›Ü[™YÙ\™\ˆ\Y˜Xİ\È›È^Xİ]X›H\›Z\ÜÚ[Ûˆš]ŠBˆYˆ\Ø]ŠÜË™Ù]]ZYŠN‚ˆ\›Z]YÛİÛ™\œÈHÌÜË™Ù]]ZY

_BˆYˆY]Y]KœİİZY›İ[ˆ\›Z]YÛİÛ™\œÎ‚ˆ˜Z[
œÙ\™\ˆ\Y˜Xİ\È›İİÛ™YH›ÛİÜˆH][˜Ú[™ÈY™™Xİ]™H\Ù\ˆŠB‚‚™YˆÜ[—İ™\šYšYYØš[˜\Jš[˜\WÜ]ˆ]^XİYˆXİÜİ‹[WJHOˆÜ[š[˜\N‚ˆ˜]ÈHÜË™œÜ]
š[˜\WÜ]
BˆYˆ›İ˜]ÈÜˆ[ŠÜË™œÙ[˜ÛÙJ˜]ÊJHˆM‚ˆ˜Z[
œÙ\™\ˆ\Y˜Xİ]\È[\HÜˆ^ÙYYÈ]È]H›İ[™ŠBˆYˆ[JÜ™
Ú\˜Xİ\ŠHŒ›ÜˆÚ\˜Xİ\ˆ[ˆ˜]ÊN‚ˆ˜Z[
œÙ\™\ˆ\Y˜Xİ]ÛÛZ[œÈHÛÛ›ÛÚ\˜Xİ\ˆŠBˆXœÛÛ]HHš[˜\WÜ]Yˆš[˜\WÜ]š\×ØXœÛÛ]J
H[ÙH]˜İÙ

HÈš[˜\WÜ]ˆ\™[HXœÛÛ]Kœ\™[œ™\ÛÛ™JİšXİUYJBˆØ[™Y]HH\™[ÈXœÛÛ]K›˜[YBˆYˆ›İ\Ø]ŠÜË“×Ó“Ñ“ÓÕÈŠN‚ˆ˜Z[
\È]›Ü›HØ[››İ[™›Ü˜ÙH›ËY›ÛİÈ^Xİ]X›HÜ[š[™ÈŠBˆ›YÜÈHÜË“×Ô‘Ó“HÜË“×Ó“Ñ“ÓÕÂˆYˆ\Ø]ŠÜË“×ĞÓÑVPÈŠN‚ˆ›YÜÈHÜË“×ĞÓÑVPÂˆN‚ˆ\ØÜš\ÜˆHÜË›Ü[ŠØ[™Y]K›YÜÊBˆ^Ù\ÔÑ\œ›Üˆ\È^Î‚ˆ˜Z[
ˆ˜Ø[››İÜ[ˆÙ\™\ˆ\Y˜XİÚ]İ]›ÛİÚ[™ÈŞ[X›ÛXÈ[šÜÎˆÙ^ßHŠBˆN‚ˆY]Y]HHÜË™œİ]
\ØÜš\ÜŠBˆ˜[Y]WÛÜ[—ÛY]Y]JY]Y]JBˆÚ^™HHY]Y]KœİÜÚ^™BˆYˆÚ^™HOH^XİYÈ˜]\È—N‚ˆ˜Z[
ˆœÙ\™\ˆ\Y˜XİÚ^™HÜÚ^™_HY™™\œÈœ›ÛH™XÙZ\Ú^™HÙ^XİYÉØ]\É×_HŠBˆYÙ\İHÚLM—Ù\ØÜš\ÜŠ\ØÜš\ÜŠBˆYˆYÙ\İOH^XİYÈœÚLMˆ—N‚ˆ˜Z[
œÙ\™\ˆ\Y˜XİÒKLMˆY™™\œÈœ›ÛHH]X[YšYY™XÙZ\ŠBˆ™]\›ˆÜ[š[˜\Jˆ\ØÜš\ÜY\ØÜš\Ü‹ˆ]XØ[™Y]KˆÚLMYYÙ\İˆÚ^™O\Ú^™Kˆ]šXÙO[Y]Y]KœİÙ]‹ˆ[›ÙO[Y]Y]KœİÚ[›Ëˆ[ÙO\İ]”×ÒSSÑJY]Y]KœİÛ[ÙJKˆİÛ™\—İZY[Y]Y]KœİİZYˆ
Bˆ^Ù\˜\ÙQ^Ù\[Û‚ˆÜË˜ÛÜÙJ\ØÜš\ÜŠBˆ˜Z\ÙB‚‚™Yˆ™\šYJˆ™XÙZ\Ü]ˆ]ˆš[˜\WÜ]ˆ]ˆÛÛ˜XİÜ]ˆ]ˆÛİ\˜ÙWÜ›Ûİˆ]ˆØØ[Ü]X[YšXØ][Û—Ü™XÙZ\ˆ]ˆ^XİYØÛÛ[Z]ˆİˆ›Û™HH›Û™KŠHOˆ\VÙXİÜİ‹[WKÜ[š[˜\WN‚ˆ›Ü›X[^™YH˜[Y]WÜ™XÙZ\
ˆ™XÙZ\Ü]ˆÛÛ˜XİÜ]ˆÛİ\˜ÙWÜ›ÛİˆØØ[Ü]X[YšXØ][Û—Ü™XÙZ\ˆ^XİYØÛÛ[Z]ˆ
BˆÜ[™YHÜ[—İ™\šYšYYØš[˜\Jš[˜\WÜ]›Ü›X[^™YÈ˜š[˜\H—JBˆ™]\›ˆ›Ü›X[^™YÜ[™Y‚‚™Yˆ\œÙWØ\™ÜÊ\™İˆ\İÜİ—JHOˆ\™Ü\œÙK“˜[Y\ÜXÙN‚ˆ\œÙ\ˆH\™Ü\œÙK\™İ[Y[\œÙ\Š\ØÜš\[ÛW×ÙØ××ÊBˆ\œÙ\‹˜YØ\™İ[Y[
œ™XÙZ\‹\OT]
Bˆ\œÙ\‹˜YØ\™İ[Y[
˜š[˜\H‹\OT]
Bˆ\œÙ\‹˜YØ\™İ[Y[
‹KXÛÛ˜Xİ‹\OT]Y˜][QQUSĞÓÓ•PÕ
Bˆ\œÙ\‹˜YØ\™İ[Y[
‹K\Ûİ\˜ÙK\›Ûİ‹\OT]Y˜][T“ÓÕ
Bˆ\œÙ\‹˜YØ\™İ[Y[
‹K[ØØ[\]X[YšXØ][Û‹\™XÙZ\‹\OT]™\]Z\™YUYJBˆ\œÙ\‹˜YØ\™İ[Y[
‹KY^XİYY›XÜXÛÛ[Z]ŠBˆ™]\›ˆ\œÙ\‹œ\œÙWØ\™ÜÊ\™İŠB‚‚™YˆXZ[Š\™İˆ\İÜİ—H›Û™HH›Û™JHOˆ[‚ˆ\™ÜÈH\œÙWØ\™ÜÊŞ\Ë˜\™İ–ÌN—HYˆ\™İˆ\È›Û™H[ÙH\™İŠBˆÜ[™YˆÜ[š[˜\H›Û™HH›Û™BˆN‚ˆ›Ü›X[^™YÜ[™YH™\šYJˆ\™ÜËœ™XÙZ\ˆ\™ÜË˜š[˜\Kˆ\™ÜË˜ÛÛ˜Xİˆ\™ÜËœÛİ\˜ÙWÜ›Ûİˆ\™ÜË›ØØ[Ü]X[YšXØ][Û—Ü™XÙZ\ˆ\™ÜË™^XİYÙ›XÜØÛÛ[Z]ˆ
Bˆ^Ù\
ÔÑ\œ›Ü‹™\šYšXØ][Û‘\œ›ÜŠH\È^Î‚ˆš[
ˆ›]™HÙ\™\ˆš[˜\H™XÙZ\ˆRSˆÙ^ßH‹š[O\Ş\Ëœİ\œŠBˆ™]\›ˆBˆš[˜[N‚ˆYˆÜ[™Y\È›İ›Û™N‚ˆÜË˜ÛÜÙJÜ[™Y™\ØÜš\ÜŠBˆš[
ˆ›]™HÙ\™\ˆš[˜\H™XÙZ\ˆTÔÈ‚ˆˆŠÛ›Ü›X[^™YÉÜÛİ\˜ÙI×VÉÙ›XÜØÛÛ[Z]	×_KÛ›Ü›X[^™YÉØš[˜\I×VÉÜÚLM‰×_JH‚ˆ
Bˆ™]\›ˆ‚‚šYˆ×Û˜[YW×ÈOH—×ÛXZ[—×È‚ˆ˜Z\ÙHŞ\İ[Q^]
XZ[Š
JB