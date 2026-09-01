#!/usr/bin/env python3
"""Promote one qualified R1-R5 receipt pair into the exact live compatibility registry."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_REGISTRY = ROOT / "architecture/live_compatibility_registry_v1.json"
REGISTRY_SCHEMA = "dfmcp.live-compatibility-registry/1"
LIVE_RECEIPT_SCHEMA = "dfmcp.live-read-acceptance-receipt/1"
NATIVE_RECEIPT_SCHEMA = "dfmcp.dfhack-plugin-qualification/1"
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
LOCATOR = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:/@+-]{0,1023}$")
EXPECTED_LIVE_GATES = ["R2", "R3", "R4", "R5"]
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


class PromotionError(ValueError):
    pass


def fail(message: str) -> None:
    raise PromotionError(message)


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


def read_object(path: Path, maximum_bytes: int, label: str) -> dict[str, Any]:
    try:
        size = path.stat().st_size
    except OSError as exc:
        fail(f"cannot stat {label}: {exc}")
    if size <= 0 or size > maximum_bytes:
        fail(f"{label} must contain 1..={maximum_bytes} bytes, got {size}")
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        fail(f"cannot parse {label}: {exc}")
    if not isinstance(value, dict):
        fail(f"{label} must be a JSON object")
    return value


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


def validate_live_receipt(path: Path) -> dict[str, Any]:
    receipt = read_object(path, 8 * 1024 * 1024, "live acceptance receipt")
    if receipt.get("schema") != LIVE_RECEIPT_SCHEMA:
        fail("live acceptance receipt schema is unsupported")
    if receipt.get("status") != "qualified":
        fail("only a clean, non-synthetic qualified live receipt may be promoted")
    declared_digest = require_hash(receipt.get("receipt_digest"), "live_receipt.receipt_digest")
    unsigned = dict(receipt)
    del unsigned["receipt_digest"]
    actual_digest = sha256_bytes(canonical_json(unsigned))
    if declared_digest != actual_digest:
        fail("live acceptance receipt digest does not reproduce its canonical fields")

    source = require_object(receipt.get("source"), "live_receipt.source")
    normalized_source = {
        "dfmcp_commit": require_commit(source.get("dfmcp_commit"), "live_receipt.source.dfmcp_commit"),
        "dfmcp_dirty": source.get("dfmcp_dirty"),
        "dfhack_commit": require_commit(source.get("dfhack_commit"), "live_receipt.source.dfhack_commit"),
        "plugin_sha256": require_hash(source.get("plugin_sha256"), "live_receipt.source.plugin_sha256"),
        "native_build_receipt_sha256": require_hash(
            source.get("native_build_receipt_sha256"),
            "live_receipt.source.native_build_receipt_sha256",
        ),
        "live_acceptance_receipt_sha256": sha256_file(path),
        "live_acceptance_receipt_digest": declared_digest,
    }
    if normalized_source["dfmcp_dirty"] is not False:
        fail("live acceptance receipt is not bound to a clean dfmcp tree")

    version = require_object(receipt.get("version_tuple"), "live_receipt.version_tuple")
    normalized_version = {
        "dwarf_fortress": require_string(version.get("dwarf_fortress"), "live_receipt.version_tuple.dwarf_fortress", 128),
        "dfhack": require_string(version.get("dfhack"), "live_receipt.version_tuple.dfhack", 128),
        "bridge": require_string(version.get("bridge"), "live_receipt.version_tuple.bridge", 128),
        "protocol": require_string(version.get("protocol"), "live_receipt.version_tuple.protocol", 16),
    }
    if normalized_version["protocol"] != "1.0":
        fail("the V1 compatibility registry admits only bridge protocol 1.0")

    host = require_object(receipt.get("host"), "live_receipt.host")
    normalized_platform = {
        "system": require_string(host.get("system"), "live_receipt.host.system", 128),
        "machine": require_string(host.get("machine"), "live_receipt.host.machine", 128),
    }

    gates = require_list(receipt.get("gates"), "live_receipt.gates")
    if [gate.get("gate") for gate in gates if isinstance(gate, dict)] != EXPECTED_LIVE_GATES:
        fail("live acceptance receipt gate order or set drifted")
    normalized_gates: list[dict[str, Any]] = []
    for index, raw in enumerate(gates):
        gate = require_object(raw, f"live_receipt.gates[{index}]")
        name = require_string(gate.get("gate"), f"live_receipt.gates[{index}].gate", 8)
        if gate.get("status") != "passed":
            fail(f"live acceptance gate {name} did not pass")
        normalized_gates.append(
            {
                "gate": name,
                "case_count": require_positive_int(
                    gate.get("case_count"), f"live_receipt.gates[{index}].case_count"
                ),
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


def validate_native_receipt(path: Path, live: dict[str, Any]) -> dict[str, Any]:
    expected_receipt_digest = live["source"]["native_build_receipt_sha256"]
    actual_receipt_digest = sha256_file(path)
    if actual_receipt_digest != expected_receipt_digest:
        fail("native build receipt bytes do not match the live receipt binding")
    receipt = read_object(path, 8 * 1024 * 1024, "native build receipt")
    if receipt.get("schema") != NATIVE_RECEIPT_SCHEMA or receipt.get("status") != "native-build-passed":
        fail("native receipt is not a passing dfmcp R1 build receipt")
    source = require_object(receipt.get("source"), "native_receipt.source")
    if source.get("dfmcp_commit") != live["source"]["dfmcp_commit"]:
        fail("native and live receipts name different dfmcp commits")
    if source.get("dfhack_commit") != live["source"]["dfhack_commit"]:
        fail("native and live receipts name different DFHack commits")
    if source.get("dfmcp_dirty") is not False:
        fail("native build receipt is not bound to a clean dfmcp tree")
    plugin = require_object(receipt.get("plugin"), "native_receipt.plugin")
    if plugin.get("sha256") != live["source"]["plugin_sha256"]:
        fail("native and live receipts name different plugin binaries")
    if plugin.get("rpc_methods") != EXPECTED_RPC_METHODS:
        fail("native receipt RPC method set drifted")
    if plugin.get("mutation_rpc_methods") != []:
        fail("native receipt contains mutation RPC methods")
    if plugin.get("strings_inventory") != "passed":
        fail("compatibility promotion requires a passing plugin string inventory")
    if plugin.get("symbols_inventory") != "passed":
        fail("compatibility promotion requires a passing plugin symbol inventory")
    return {
        "native_build_receipt_sha256": actual_receipt_digest,
        "plugin_sha256": plugin["sha256"],
    }


def validate_registry(value: dict[str, Any]) -> list[dict[str, Any]]:
    if value.get("schema_version") != REGISTRY_SCHEMA:
        fail("compatibility registry schema is unsupported")
    entries = require_list(value.get("entries"), "registry.entries")
    ids: set[str] = set()
    previous = ""
    normalized: list[dict[str, Any]] = []
    for index, raw in enumerate(entries):
        entry = require_object(raw, f"registry.entries[{index}]")
        entry_id = require_hash(entry.get("entry_id"), f"registry.entries[{index}].entry_id")
        if entry_id in ids:
            fail(f"registry repeats entry_id {entry_id}")
        if previous and entry_id <= previous:
            fail("registry entries are not in strict entry_id order")
        unsigned = dict(entry)
        del unsigned["entry_id"]
        if sha256_bytes(canonical_json(unsigned)) != entry_id:
            fail(f"registry entry {entry_id} does not reproduce its identifier")
        ids.add(entry_id)
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
        "source": {
            **live["source"],
            **native,
        },
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
        "limitations": [
            "admission applies only to this exact source, binary, version, and platform tuple",
            "host compromise is outside the loopback bearer threat model",
            "no live mutation method is admitted",
            "durable production custody and release support are not established by R1-R5",
        ],
    }
    return {"entry_id": sha256_bytes(canonical_json(unsigned)), **unsigned}


def promote(
    registry_path: Path,
    live_receipt_path: Path,
    native_receipt_path: Path,
    evidence_locator: str,
) -> dict[str, Any]:
    registry = read_object(registry_path, 8 * 1024 * 1024, "compatibility registry")
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
    return {
        "schema_version": REGISTRY_SCHEMA,
        "status": "admitted_live_tuples",
        "entries": output_entries,
    }


def write_atomic(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    content = json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    except BaseException:
        try:
            os.unlink(temporary)
        except OSError:
            pass
        raise


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)
    parser.add_argument("--live-receipt", type=Path, required=True)
    parser.add_argument("--native-receipt", type=Path, required=True)
    parser.add_argument("--evidence-locator", required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--in-place", action="store_true")
    args = parser.parse_args(argv)
    if args.in_place == (args.output is not None):
        parser.error("choose exactly one of --in-place or --output")
    return args


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    destination = args.registry if args.in_place else args.output
    try:
        promoted = promote(
            args.registry,
            args.live_receipt,
            args.native_receipt,
            args.evidence_locator,
        )
        write_atomic(destination, promoted)
    except (PromotionError, OSError) as exc:
        print(f"live compatibility promotion: FAIL: {exc}", file=sys.stderr)
        return 1
    entry = promoted["entries"][-1]
    print(f"live compatibility promotion: PASS ({entry['entry_id']})")
    print(destination)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
