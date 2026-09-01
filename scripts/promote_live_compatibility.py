#!/usr/bin/env python3
"""Promote one qualified R1-R5 receipt pair into the exact live compatibility registry."""

from __future__ import annotations

import argparse
import contextlib
import hashlib
import json
import os
import re
import stat
import sys
import tempfile
from pathlib import Path
from typing import Any, Iterator

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_REGISTRY = ROOT / "architecture/live_compatibility_registry_v1.json"
REGISTRY_SCHEMA = "dfmcp.live-compatibility-registry/1"
LIVE_RECEIPT_SCHEMA = "dfmcp.live-read-acceptance-receipt/1"
NATIVE_RECEIPT_SCHEMA = "dfmcp.dfhack-plugin-qualification/1"
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
LOCATOR = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:/@+-]{0,1023}$")
EXPECTED_LIVE_GATES = ["R2", "R3", "R4", "R5"]
EXPECTED_LIVE_CASE_COUNTS = {"R2": 12, "R3": 14, "R4": 7, "R5": 1}
EXPECTED_RPC_METHODS = ["Handshake", "ReadObservation"]
READ_ONLY_CAPABILITIES = ["doctor", "observe", "query", "wait"]
OBSERVED_DOMAINS = [
    "fortress.citizens.roster",
    "fortress.clock",
    "fortress.identity",
    "fortress.pause_state",
]
CONDITIONAL_DOMAINS = ["fortress.citizens.names"]
OMITTED_DOMAINS = [
    "fortress.economy",
    "fortress.history",
    "fortress.items",
    "fortress.jobs",
    "fortress.map",
    "fortress.military",
    "fortress.welfare",
]
LIMITATIONS = [
    "admission applies only to this exact source, binary, version, and platform tuple",
    "host compromise is outside the loopback bearer threat model",
    "no live mutation method is admitted",
    "durable production custody and release support are not established by R1-R5",
]
MAX_JSON_BYTES = 8 * 1024 * 1024
MAX_STRING_BYTES = 16 * 1024
MAX_COLLECTION_ITEMS = 4096
MAX_DEPTH = 64


class PromotionError(ValueError):
    pass


def fail(message: str) -> None:
    raise PromotionError(message)


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
                fail(f"{path} contains a non-string object key")
            bounded_tree(key, f"{path}.<key>", depth + 1)
            bounded_tree(item, f"{path}.{key}", depth + 1)
        return
    fail(f"{path} contains unsupported JSON type {type(value).__name__}")


def _open_stable_regular(path: Path, maximum_bytes: int, label: str) -> tuple[int, os.stat_result]:
    raw = os.fspath(path)
    if not raw or len(os.fsencode(raw)) > 4096:
        fail(f"{label} path is empty or exceeds its byte bound")
    flags = os.O_RDONLY
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
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
    path: Path, maximum_bytes: int, label: str
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
    _, digest = read_bytes_with_digest(path, MAX_JSON_BYTES, "evidence file")
    return digest


def read_object_with_digest(
    path: Path, maximum_bytes: int, label: str
) -> tuple[dict[str, Any], str]:
    raw, digest = read_bytes_with_digest(path, maximum_bytes, label)
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


def read_object(path: Path, maximum_bytes: int, label: str) -> dict[str, Any]:
    value, _ = read_object_with_digest(path, maximum_bytes, label)
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


def require_string(value: Any, path: str, maximum: int = 4096) -> str:
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


def require_positive_int(value: Any, path: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        fail(f"{path} must be a positive integer")
    return value


def _validate_hash_map(value: Any, path: str) -> dict[str, str]:
    source = require_object(value, path)
    if not source:
        fail(f"{path} must not be empty")
    normalized: dict[str, str] = {}
    for key, raw in source.items():
        name = require_string(key, f"{path}.<key>", 256)
        normalized[name] = require_hash(raw, f"{path}.{name}")
    return normalized


def validate_live_receipt(path: Path) -> dict[str, Any]:
    receipt, receipt_file_sha256 = read_object_with_digest(
        path, MAX_JSON_BYTES, "live acceptance receipt"
    )
    require_exact_keys(
        receipt,
        {
            "schema",
            "status",
            "run_id",
            "source",
            "version_tuple",
            "host",
            "evidence",
            "gates",
            "claims_established",
            "claims_not_established",
            "receipt_digest",
        },
        "live_receipt",
    )
    if receipt.get("schema") != LIVE_RECEIPT_SCHEMA:
        fail("live acceptance receipt schema is unsupported")
    if receipt.get("status") != "qualified":
        fail("only a clean, non-synthetic qualified live receipt may be promoted")
    require_string(receipt.get("run_id"), "live_receipt.run_id", 128)
    declared_digest = require_hash(receipt.get("receipt_digest"), "live_receipt.receipt_digest")
    unsigned = dict(receipt)
    del unsigned["receipt_digest"]
    if declared_digest != sha256_bytes(canonical_json(unsigned)):
        fail("live acceptance receipt digest does not reproduce its canonical fields")

    source = require_object(receipt.get("source"), "live_receipt.source")
    require_exact_keys(
        source,
        {
            "dfmcp_commit",
            "dfmcp_dirty",
            "dfhack_commit",
            "plugin_sha256",
            "native_build_receipt_sha256",
            "source_digests",
        },
        "live_receipt.source",
    )
    normalized_source = {
        "dfmcp_commit": require_commit(
            source.get("dfmcp_commit"), "live_receipt.source.dfmcp_commit"
        ),
        "dfmcp_dirty": source.get("dfmcp_dirty"),
        "dfhack_commit": require_commit(
            source.get("dfhack_commit"), "live_receipt.source.dfhack_commit"
        ),
        "plugin_sha256": require_hash(
            source.get("plugin_sha256"), "live_receipt.source.plugin_sha256"
        ),
        "native_build_receipt_sha256": require_hash(
            source.get("native_build_receipt_sha256"),
            "live_receipt.source.native_build_receipt_sha256",
        ),
        "live_acceptance_receipt_sha256": receipt_file_sha256,
        "live_acceptance_receipt_digest": declared_digest,
    }
    _validate_hash_map(source.get("source_digests"), "live_receipt.source.source_digests")
    if normalized_source["dfmcp_dirty"] is not False:
        fail("live acceptance receipt is not bound to a clean dfmcp tree")

    version = require_object(receipt.get("version_tuple"), "live_receipt.version_tuple")
    require_exact_keys(
        version,
        {"dwarf_fortress", "dfhack", "bridge", "protocol"},
        "live_receipt.version_tuple",
    )
    normalized_version = {
        "dwarf_fortress": require_string(
            version.get("dwarf_fortress"),
            "live_receipt.version_tuple.dwarf_fortress",
            128,
        ),
        "dfhack": require_string(
            version.get("dfhack"), "live_receipt.version_tuple.dfhack", 128
        ),
        "bridge": require_string(
            version.get("bridge"), "live_receipt.version_tuple.bridge", 128
        ),
        "protocol": require_string(
            version.get("protocol"), "live_receipt.version_tuple.protocol", 16
        ),
    }
    if normalized_version["protocol"] != "1.0":
        fail("the V1 compatibility registry admits only bridge protocol 1.0")

    host = require_object(receipt.get("host"), "live_receipt.host")
    normalized_platform = {
        "system": require_string(host.get("system"), "live_receipt.host.system", 128),
        "machine": require_string(host.get("machine"), "live_receipt.host.machine", 128),
    }
    require_object(receipt.get("evidence"), "live_receipt.evidence")
    require_list(receipt.get("claims_established"), "live_receipt.claims_established")
    require_list(receipt.get("claims_not_established"), "live_receipt.claims_not_established")

    gates = require_list(receipt.get("gates"), "live_receipt.gates")
    if [gate.get("gate") for gate in gates if isinstance(gate, dict)] != EXPECTED_LIVE_GATES:
        fail("live acceptance receipt gate order or set drifted")
    normalized_gates: list[dict[str, Any]] = []
    for index, raw in enumerate(gates):
        gate = require_object(raw, f"live_receipt.gates[{index}]")
        require_exact_keys(
            gate,
            {"gate", "status", "case_count", "evidence_digest"},
            f"live_receipt.gates[{index}]",
        )
        name = require_string(gate.get("gate"), f"live_receipt.gates[{index}].gate", 8)
        if gate.get("status") != "passed":
            fail(f"live acceptance gate {name} did not pass")
        case_count = require_positive_int(
            gate.get("case_count"), f"live_receipt.gates[{index}].case_count"
        )
        if case_count != EXPECTED_LIVE_CASE_COUNTS[name]:
            fail(
                f"live acceptance gate {name} case count {case_count} differs from "
                f"the V1 contract count {EXPECTED_LIVE_CASE_COUNTS[name]}"
            )
        normalized_gates.append(
            {
                "gate": name,
                "case_count": case_count,
                "evidence_digest": require_hash(
                    gate.get("evidence_digest"),
                    f"live_receipt.gates[{index}].evidence_digest",
                ),
            }
        )

    return {
        "source": normalized_source,
        "version_tuple": normalized_version,
        "platform": normalized_platform,
        "gates": normalized_gates,
    }


def normalize_native_receipt(receipt: dict[str, Any]) -> dict[str, Any]:
    if receipt.get("schema") != NATIVE_RECEIPT_SCHEMA or receipt.get("status") != "native-build-passed":
        fail("native receipt is not a passing dfmcp R1 build receipt")
    source = require_object(receipt.get("source"), "native_receipt.source")
    normalized_source = {
        "dfmcp_commit": require_commit(
            source.get("dfmcp_commit"), "native_receipt.source.dfmcp_commit"
        ),
        "dfmcp_dirty": source.get("dfmcp_dirty"),
        "dfhack_commit": require_commit(
            source.get("dfhack_commit"), "native_receipt.source.dfhack_commit"
        ),
    }
    if normalized_source["dfmcp_dirty"] is not False:
        fail("native build receipt is not bound to a clean dfmcp tree")
    if source.get("dfhack_dirty") not in {None, False}:
        fail("native build receipt is not bound to a clean DFHack tree")

    plugin = require_object(receipt.get("plugin"), "native_receipt.plugin")
    normalized_plugin = {
        "sha256": require_hash(plugin.get("sha256"), "native_receipt.plugin.sha256"),
        "rpc_methods": plugin.get("rpc_methods"),
        "mutation_rpc_methods": plugin.get("mutation_rpc_methods"),
        "strings_inventory": plugin.get("strings_inventory"),
        "symbols_inventory": plugin.get("symbols_inventory"),
    }
    if normalized_plugin["rpc_methods"] != EXPECTED_RPC_METHODS:
        fail("native receipt RPC method set drifted")
    if normalized_plugin["mutation_rpc_methods"] != []:
        fail("native receipt contains mutation RPC methods")
    if normalized_plugin["strings_inventory"] != "passed":
        fail("compatibility promotion requires a passing plugin string inventory")
    if normalized_plugin["symbols_inventory"] != "passed":
        fail("compatibility promotion requires a passing plugin symbol inventory")

    source_digests: dict[str, str] = {}
    if "source_digests" in receipt:
        source_digests = _validate_hash_map(
            receipt.get("source_digests"), "native_receipt.source_digests"
        )
    return {
        "source": normalized_source,
        "plugin": normalized_plugin,
        "source_digests": source_digests,
    }


def validate_native_receipt(
    value: Path | dict[str, Any],
    live: dict[str, Any] | None = None,
) -> dict[str, Any]:
    actual_receipt_digest: str | None = None
    if isinstance(value, Path):
        receipt, actual_receipt_digest = read_object_with_digest(
            value, MAX_JSON_BYTES, "native build receipt"
        )
    elif isinstance(value, dict):
        receipt = value
    else:
        fail("native receipt must be a path or parsed JSON object")

    normalized = normalize_native_receipt(receipt)
    if live is None:
        return normalized
    if actual_receipt_digest is None:
        fail("cross-receipt native validation requires exact receipt-file bytes")

    expected_receipt_digest = live["source"]["native_build_receipt_sha256"]
    if actual_receipt_digest != expected_receipt_digest:
        fail("native build receipt bytes do not match the live receipt binding")
    if normalized["source"]["dfmcp_commit"] != live["source"]["dfmcp_commit"]:
        fail("native and live receipts name different dfmcp commits")
    if normalized["source"]["dfhack_commit"] != live["source"]["dfhack_commit"]:
        fail("native and live receipts name different DFHack commits")
    if normalized["plugin"]["sha256"] != live["source"]["plugin_sha256"]:
        fail("native and live receipts name different plugin binaries")
    return {
        "native_build_receipt_sha256": actual_receipt_digest,
        "plugin_sha256": normalized["plugin"]["sha256"],
    }


def _validate_entry(entry: dict[str, Any], index: int) -> None:
    path = f"registry.entries[{index}]"
    require_exact_keys(
        entry,
        {
            "entry_id",
            "support_level",
            "version_tuple",
            "platform",
            "source",
            "gates",
            "capabilities",
            "mutation_capabilities",
            "observed_domains",
            "conditional_domains",
            "omitted_domains",
            "evidence_locator",
            "limitations",
        },
        path,
    )
    if entry.get("support_level") != "experimental":
        fail(f"{path}.support_level must be experimental")
    version = require_object(entry.get("version_tuple"), f"{path}.version_tuple")
    require_exact_keys(
        version,
        {"dwarf_fortress", "dfhack", "bridge", "protocol"},
        f"{path}.version_tuple",
    )
    for name in ["dwarf_fortress", "dfhack", "bridge"]:
        require_string(version.get(name), f"{path}.version_tuple.{name}", 128)
    if version.get("protocol") != "1.0":
        fail(f"{path}.version_tuple.protocol must be 1.0")
    platform = require_object(entry.get("platform"), f"{path}.platform")
    require_exact_keys(platform, {"system", "machine"}, f"{path}.platform")
    require_string(platform.get("system"), f"{path}.platform.system", 128)
    require_string(platform.get("machine"), f"{path}.platform.machine", 128)
    source = require_object(entry.get("source"), f"{path}.source")
    require_exact_keys(
        source,
        {
            "dfmcp_commit",
            "dfmcp_dirty",
            "dfhack_commit",
            "plugin_sha256",
            "native_build_receipt_sha256",
            "live_acceptance_receipt_sha256",
            "live_acceptance_receipt_digest",
        },
        f"{path}.source",
    )
    require_commit(source.get("dfmcp_commit"), f"{path}.source.dfmcp_commit")
    require_commit(source.get("dfhack_commit"), f"{path}.source.dfhack_commit")
    if source.get("dfmcp_dirty") is not False:
        fail(f"{path}.source.dfmcp_dirty must be false")
    for name in [
        "plugin_sha256",
        "native_build_receipt_sha256",
        "live_acceptance_receipt_sha256",
        "live_acceptance_receipt_digest",
    ]:
        require_hash(source.get(name), f"{path}.source.{name}")
    gates = require_list(entry.get("gates"), f"{path}.gates")
    if [gate.get("gate") for gate in gates if isinstance(gate, dict)] != [
        "R1",
        *EXPECTED_LIVE_GATES,
    ]:
        fail(f"{path}.gates must contain R1-R5 in canonical order")
    for gate_index, raw in enumerate(gates):
        gate = require_object(raw, f"{path}.gates[{gate_index}]")
        name = gate.get("gate")
        if name == "R1":
            require_exact_keys(
                gate,
                {"gate", "status", "receipt_sha256"},
                f"{path}.gates[{gate_index}]",
            )
            require_hash(
                gate.get("receipt_sha256"),
                f"{path}.gates[{gate_index}].receipt_sha256",
            )
        else:
            require_exact_keys(
                gate,
                {"gate", "status", "case_count", "evidence_digest"},
                f"{path}.gates[{gate_index}]",
            )
            case_count = require_positive_int(
                gate.get("case_count"), f"{path}.gates[{gate_index}].case_count"
            )
            if case_count != EXPECTED_LIVE_CASE_COUNTS.get(name):
                fail(f"{path}.gates[{gate_index}] has the wrong V1 case count")
            require_hash(
                gate.get("evidence_digest"),
                f"{path}.gates[{gate_index}].evidence_digest",
            )
        if gate.get("status") != "passed":
            fail(f"{path}.gates[{gate_index}] did not pass")
    if entry.get("capabilities") != READ_ONLY_CAPABILITIES:
        fail(f"{path}.capabilities drifted")
    if entry.get("mutation_capabilities") != []:
        fail(f"{path}.mutation_capabilities must remain empty")
    if entry.get("observed_domains") != OBSERVED_DOMAINS:
        fail(f"{path}.observed_domains drifted")
    if entry.get("conditional_domains") != CONDITIONAL_DOMAINS:
        fail(f"{path}.conditional_domains drifted")
    if entry.get("omitted_domains") != OMITTED_DOMAINS:
        fail(f"{path}.omitted_domains drifted")
    locator = require_string(entry.get("evidence_locator"), f"{path}.evidence_locator", 1024)
    if LOCATOR.fullmatch(locator) is None or ".." in Path(locator).parts:
        fail(f"{path}.evidence_locator is malformed or contains traversal")
    if entry.get("limitations") != LIMITATIONS:
        fail(f"{path}.limitations drifted")


def validate_registry(value: dict[str, Any]) -> list[dict[str, Any]]:
    require_exact_keys(value, {"schema_version", "status", "entries"}, "registry")
    if value.get("schema_version") != REGISTRY_SCHEMA:
        fail("compatibility registry schema is unsupported")
    entries = require_list(value.get("entries"), "registry.entries")
    ids: set[str] = set()
    keys: set[bytes] = set()
    previous = ""
    normalized: list[dict[str, Any]] = []
    for index, raw in enumerate(entries):
        entry = require_object(raw, f"registry.entries[{index}]")
        _validate_entry(entry, index)
        entry_id = require_hash(entry.get("entry_id"), f"registry.entries[{index}].entry_id")
        if entry_id in ids:
            fail(f"registry repeats entry_id {entry_id}")
        if previous and entry_id <= previous:
            fail("registry entries are not in strict entry_id order")
        unsigned = dict(entry)
        del unsigned["entry_id"]
        if sha256_bytes(canonical_json(unsigned)) != entry_id:
            fail(f"registry entry {entry_id} does not reproduce its identifier")
        key = compatibility_key(entry)
        if key in keys:
            fail("registry contains duplicate exact deployment tuples")
        ids.add(entry_id)
        keys.add(key)
        previous = entry_id
        normalized.append(entry)
    expected_status = "admitted_live_tuples" if entries else "no_admitted_live_tuples"
    if value.get("status") != expected_status:
        fail(f"registry status must be {expected_status}")
    return normalized


def compatibility_key(entry: dict[str, Any]) -> bytes:
    return canonical_json(
        {
            "version_tuple": entry["version_tuple"],
            "platform": entry["platform"],
            "dfmcp_commit": entry["source"]["dfmcp_commit"],
            "dfhack_commit": entry["source"]["dfhack_commit"],
            "plugin_sha256": entry["source"]["plugin_sha256"],
        }
    )


def build_entry(
    live: dict[str, Any],
    native: dict[str, Any],
    evidence_locator: str,
) -> dict[str, Any]:
    if LOCATOR.fullmatch(evidence_locator) is None:
        fail("evidence locator must be bounded, traversal-free machine text")
    if ".." in Path(evidence_locator).parts:
        fail("evidence locator must not contain path traversal")
    unsigned: dict[str, Any] = {
        "support_level": "experimental",
        "version_tuple": live["version_tuple"],
        "platform": live["platform"],
        "source": {**live["source"], **native},
        "gates": [
            {
                "gate": "R1",
                "status": "passed",
                "receipt_sha256": native["native_build_receipt_sha256"],
            },
            *[
                {
                    "gate": gate["gate"],
                    "status": "passed",
                    "case_count": gate["case_count"],
                    "evidence_digest": gate["evidence_digest"],
                }
                for gate in live["gates"]
            ],
        ],
        "capabilities": READ_ONLY_CAPABILITIES,
        "mutation_capabilities": [],
        "observed_domains": OBSERVED_DOMAINS,
        "conditional_domains": CONDITIONAL_DOMAINS,
        "omitted_domains": OMITTED_DOMAINS,
        "evidence_locator": evidence_locator,
        "limitations": LIMITATIONS,
    }
    return {"entry_id": sha256_bytes(canonical_json(unsigned)), **unsigned}


def _promote(
    registry_path: Path,
    live_receipt_path: Path,
    native_receipt_path: Path,
    evidence_locator: str,
    expected_registry_sha256: str | None = None,
) -> tuple[dict[str, Any], str]:
    registry, registry_sha256 = read_object_with_digest(
        registry_path, MAX_JSON_BYTES, "compatibility registry"
    )
    if expected_registry_sha256 is not None:
        require_hash(expected_registry_sha256, "expected_registry_sha256")
        if registry_sha256 != expected_registry_sha256:
            fail(
                "compatibility registry bytes changed since the caller selected its expected generation"
            )
    entries = validate_registry(registry)
    live = validate_live_receipt(live_receipt_path)
    native = validate_native_receipt(native_receipt_path, live)
    candidate = build_entry(live, native, evidence_locator)
    candidate_key = compatibility_key(candidate)
    for existing in entries:
        if existing["entry_id"] == candidate["entry_id"]:
            fail("the exact compatibility entry is already present")
        if compatibility_key(existing) == candidate_key:
            fail(
                "the exact source/binary/version/platform tuple already has a canonical compatibility entry"
            )
    output_entries = [*entries, candidate]
    output_entries.sort(key=lambda entry: entry["entry_id"])
    return (
        {
            "schema_version": REGISTRY_SCHEMA,
            "status": "admitted_live_tuples",
            "entries": output_entries,
        },
        candidate["entry_id"],
    )


def promote(
    registry_path: Path,
    live_receipt_path: Path,
    native_receipt_path: Path,
    evidence_locator: str,
    expected_registry_sha256: str | None = None,
) -> dict[str, Any]:
    output, _ = _promote(
        registry_path,
        live_receipt_path,
        native_receipt_path,
        evidence_locator,
        expected_registry_sha256,
    )
    return output


def _fsync_directory(path: Path) -> None:
    flags = os.O_RDONLY
    if hasattr(os, "O_DIRECTORY"):
        flags |= os.O_DIRECTORY
    try:
        descriptor = os.open(path, flags)
    except OSError:
        return
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def write_atomic(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
        _fsync_directory(path.parent)
    except BaseException:
        try:
            os.unlink(temporary)
        except OSError:
            pass
        raise


@contextlib.contextmanager
def registry_lock(path: Path) -> Iterator[None]:
    lock_path = path.with_name(f".{path.name}.promotion.lock")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    try:
        descriptor = os.open(lock_path, flags, 0o600)
    except FileExistsError:
        fail(
            f"compatibility registry promotion lock already exists: {lock_path}; "
            "inspect and remove it only after proving no promotion is active"
        )
    except OSError as exc:
        fail(f"cannot acquire compatibility registry promotion lock: {exc}")
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(f"pid={os.getpid()}\n")
            handle.flush()
            os.fsync(handle.fileno())
        _fsync_directory(lock_path.parent)
        yield
    finally:
        try:
            lock_path.unlink()
            _fsync_directory(lock_path.parent)
        except OSError:
            pass


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)
    parser.add_argument("--live-receipt", type=Path, required=True)
    parser.add_argument("--native-receipt", type=Path, required=True)
    parser.add_argument("--evidence-locator", required=True)
    parser.add_argument("--expected-registry-sha256")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--in-place", action="store_true")
    args = parser.parse_args(argv)
    if args.in_place == (args.output is not None):
        parser.error("choose exactly one of --in-place or --output")
    if args.in_place and args.expected_registry_sha256 is None:
        parser.error("--in-place requires --expected-registry-sha256")
    return args


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    destination = args.registry if args.in_place else args.output
    try:
        if args.in_place:
            with registry_lock(args.registry):
                promoted, candidate_id = _promote(
                    args.registry,
                    args.live_receipt,
                    args.native_receipt,
                    args.evidence_locator,
                    args.expected_registry_sha256,
                )
                write_atomic(destination, promoted)
        else:
            promoted, candidate_id = _promote(
                args.registry,
                args.live_receipt,
                args.native_receipt,
                args.evidence_locator,
                args.expected_registry_sha256,
            )
            write_atomic(destination, promoted)
    except (PromotionError, OSError) as exc:
        print(f"live compatibility promotion: FAIL: {exc}", file=sys.stderr)
        return 1
    print(f"live compatibility promotion: PASS ({candidate_id})")
    print(destination)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
