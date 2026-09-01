#!/usr/bin/env python3
"""Wrap one passing native receipt with exact protocol-1.1 generation identity."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import re
import stat
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
PROMOTION_PATH = ROOT / "scripts/promote_live_compatibility.py"
DEFAULT_CONTRACT = ROOT / "architecture/dfhack_plugin_native_receipt_v1_1.json"
RECEIPT_SCHEMA = "dfmcp.dfhack-plugin-native-qualification/1.1"
MAX_JSON_BYTES = 16 * 1024 * 1024
MAX_SOURCE_FILE_BYTES = 64 * 1024 * 1024
HEX64 = re.compile(r"^[0-9a-f]{64}$")

SPEC = importlib.util.spec_from_file_location(
    "promote_live_compatibility", PROMOTION_PATH
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load native receipt primitives")
promotion = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = promotion
SPEC.loader.exec_module(promotion)


class ReceiptError(ValueError):
    pass


def fail(message: str) -> None:
    raise ReceiptError(message)


def require(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


def canonical_json(value: Any) -> bytes:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def require_hash(value: Any, path: str) -> str:
    if not isinstance(value, str) or HEX64.fullmatch(value) is None or value == "0" * 64:
        fail(f"{path} must be a nonzero lowercase SHA-256 digest")
    return value


def require_object(value: Any, path: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail(f"{path} must be an object")
    return value


def require_exact_keys(value: dict[str, Any], expected: set[str], path: str) -> None:
    if set(value) != expected:
        fail(
            f"{path} fields differ: expected {sorted(expected)}, got {sorted(value)}"
        )


def stable_file_sha256(path: Path, maximum: int, label: str) -> str:
    raw = os.fspath(path)
    if not raw or len(os.fsencode(raw)) > 4096:
        fail(f"{label} path is empty or exceeds its byte bound")
    if not hasattr(os, "O_NOFOLLOW"):
        fail("this platform cannot enforce no-follow source hashing")
    flags = os.O_RDONLY | os.O_NOFOLLOW
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    try:
        descriptor = os.open(path, flags)
    except OSError as exc:
        fail(f"cannot open {label}: {exc}")
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode):
            fail(f"{label} must be a regular file")
        if before.st_size <= 0 or before.st_size > maximum:
            fail(f"{label} must contain 1..={maximum} bytes, got {before.st_size}")
        digest = hashlib.sha256()
        total = 0
        while True:
            chunk = os.read(descriptor, min(1024 * 1024, maximum + 1 - total))
            if not chunk:
                break
            total += len(chunk)
            if total > maximum:
                fail(f"{label} grew beyond its byte bound")
            digest.update(chunk)
        after = os.fstat(descriptor)
        if (
            before.st_dev != after.st_dev
            or before.st_ino != after.st_ino
            or before.st_size != after.st_size
            or before.st_mtime_ns != after.st_mtime_ns
            or total != before.st_size
        ):
            fail(f"{label} changed while being hashed")
        return digest.hexdigest()
    finally:
        os.close(descriptor)


def load_contract(path: Path) -> dict[str, Any]:
    contract = promotion.read_object(path, 1024 * 1024, "protocol-1.1 native receipt contract")
    require_exact_keys(
        contract,
        {
            "schema_version",
            "receipt_schema",
            "status",
            "base_receipt_schema",
            "bridge",
            "required_source_digests",
            "required_inventories",
            "authority",
            "claims_established",
            "claims_not_established",
        },
        "contract",
    )
    if contract.get("schema_version") != "dfmcp.dfhack-plugin-native-qualification-contract/1.1":
        fail("protocol-1.1 native receipt contract schema is unsupported")
    if contract.get("receipt_schema") != RECEIPT_SCHEMA:
        fail("protocol-1.1 native receipt schema drifted")
    if contract.get("status") != "normative_protocol_generation_wrapper":
        fail("protocol-1.1 native receipt contract status drifted")
    if contract.get("base_receipt_schema") != promotion.NATIVE_RECEIPT_SCHEMA:
        fail("base native receipt schema drifted")
    bridge = require_object(contract.get("bridge"), "contract.bridge")
    expected_bridge = {
        "plugin": "dfmcp_bridge_v1_1",
        "protobuf_package": "dfmcp.bridge.v1_1",
        "bridge_version": "0.2.0",
        "protocol": "1.1",
        "rpc_methods": ["Handshake", "ReadObservation"],
        "mutation_rpc_methods": [],
    }
    if bridge != expected_bridge:
        fail("protocol-1.1 bridge identity contract drifted")
    authority = require_object(contract.get("authority"), "contract.authority")
    if authority.get("capabilities_granted") != [] or authority.get("mutation_capabilities") != []:
        fail("protocol-1.1 native receipt contract grants authority")
    return contract


def source_digests(contract: dict[str, Any], source_root: Path) -> dict[str, str]:
    mapping = require_object(
        contract.get("required_source_digests"), "contract.required_source_digests"
    )
    normalized: dict[str, str] = {}
    for name, relative in mapping.items():
        if not isinstance(name, str) or not isinstance(relative, str):
            fail("source-digest mapping contains a non-string entry")
        candidate = Path(relative)
        if candidate.is_absolute() or ".." in candidate.parts or relative.startswith("./"):
            fail(f"source-digest path {name!r} is not canonical and traversal-free")
        normalized[name] = stable_file_sha256(
            source_root / candidate,
            MAX_SOURCE_FILE_BYTES,
            f"source-bound file {relative}",
        )
    return normalized


def normalize_base_source_digests(value: Any) -> dict[str, str]:
    raw = require_object(value, "base_receipt.source_digests")
    if not raw:
        fail("base native receipt contains no source digests")
    normalized: dict[str, str] = {}
    for name, digest in raw.items():
        if not isinstance(name, str) or not name or len(name.encode("utf-8")) > 256:
            fail("base native receipt contains an invalid source-digest name")
        normalized[name] = require_hash(digest, f"base_receipt.source_digests.{name}")
    return normalized


def validate_base_source_binding(
    base_source_digests: dict[str, str], generation_source_digests: dict[str, str]
) -> None:
    values = set(base_source_digests.values())
    for name in [
        "protocol_1_1_proto",
        "protocol_1_1_native",
        "protocol_1_1_qualifier",
    ]:
        expected = generation_source_digests[name]
        if expected not in values:
            fail(
                f"base native receipt does not bind the exact protocol-1.1 source digest {name}"
            )


def issue(
    base_receipt_path: Path,
    source_root: Path,
    contract_path: Path,
) -> dict[str, Any]:
    contract = load_contract(contract_path)
    base_receipt, base_file_sha256 = promotion.read_object_with_digest(
        base_receipt_path,
        MAX_JSON_BYTES,
        "base native build receipt",
    )
    normalized_base = promotion.validate_native_receipt(base_receipt)
    generation_digests = source_digests(contract, source_root)
    base_source_digests = normalize_base_source_digests(
        base_receipt.get("source_digests")
    )
    validate_base_source_binding(base_source_digests, generation_digests)
    unsigned: dict[str, Any] = {
        "schema": RECEIPT_SCHEMA,
        "status": "qualified",
        "base_receipt": {
            "file_sha256": base_file_sha256,
            "content_digest": sha256_bytes(canonical_json(base_receipt)),
            "receipt": base_receipt,
            "source_digests": base_source_digests,
        },
        "source": {
            "dfmcp_commit": normalized_base["source"]["dfmcp_commit"],
            "dfmcp_dirty": False,
            "dfhack_commit": normalized_base["source"]["dfhack_commit"],
            "dfhack_dirty": False,
        },
        "bridge": contract["bridge"],
        "plugin": {
            "sha256": normalized_base["plugin"]["sha256"],
            "rpc_methods": normalized_base["plugin"]["rpc_methods"],
            "mutation_rpc_methods": [],
            "strings_inventory": normalized_base["plugin"]["strings_inventory"],
            "symbols_inventory": normalized_base["plugin"]["symbols_inventory"],
        },
        "source_digests": generation_digests,
        "capabilities_granted": [],
        "mutation_capabilities": [],
        "claims_established": list(contract["claims_established"]),
        "claims_not_established": list(contract["claims_not_established"]),
    }
    receipt = {
        **unsigned,
        "receipt_digest": sha256_bytes(canonical_json(unsigned)),
    }
    return validate_receipt(receipt, contract)


def validate_receipt(value: dict[str, Any], contract: dict[str, Any]) -> dict[str, Any]:
    require_exact_keys(
        value,
        {
            "schema",
            "status",
            "base_receipt",
            "source",
            "bridge",
            "plugin",
            "source_digests",
            "capabilities_granted",
            "mutation_capabilities",
            "claims_established",
            "claims_not_established",
            "receipt_digest",
        },
        "receipt",
    )
    if value.get("schema") != RECEIPT_SCHEMA or value.get("status") != "qualified":
        fail("protocol-1.1 native receipt is not qualified")
    base = require_object(value.get("base_receipt"), "receipt.base_receipt")
    require_exact_keys(
        base,
        {"file_sha256", "content_digest", "receipt", "source_digests"},
        "receipt.base_receipt",
    )
    require_hash(base.get("file_sha256"), "receipt.base_receipt.file_sha256")
    embedded = require_object(base.get("receipt"), "receipt.base_receipt.receipt")
    if require_hash(
        base.get("content_digest"), "receipt.base_receipt.content_digest"
    ) != sha256_bytes(canonical_json(embedded)):
        fail("embedded base receipt content digest is invalid")
    normalized_base = promotion.validate_native_receipt(embedded)
    embedded_source_digests = normalize_base_source_digests(base.get("source_digests"))
    if embedded_source_digests != normalize_base_source_digests(
        embedded.get("source_digests")
    ):
        fail("embedded base source digests differ from their normalized copy")

    source = require_object(value.get("source"), "receipt.source")
    require_exact_keys(
        source,
        {"dfmcp_commit", "dfmcp_dirty", "dfhack_commit", "dfhack_dirty"},
        "receipt.source",
    )
    expected_source = {
        "dfmcp_commit": normalized_base["source"]["dfmcp_commit"],
        "dfmcp_dirty": False,
        "dfhack_commit": normalized_base["source"]["dfhack_commit"],
        "dfhack_dirty": False,
    }
    if source != expected_source:
        fail("protocol-1.1 native receipt source differs from its base receipt")
    if value.get("bridge") != contract["bridge"]:
        fail("protocol-1.1 native receipt bridge identity drifted")

    plugin = require_object(value.get("plugin"), "receipt.plugin")
    require_exact_keys(
        plugin,
        {
            "sha256",
            "rpc_methods",
            "mutation_rpc_methods",
            "strings_inventory",
            "symbols_inventory",
        },
        "receipt.plugin",
    )
    expected_plugin = {
        "sha256": normalized_base["plugin"]["sha256"],
        "rpc_methods": ["Handshake", "ReadObservation"],
        "mutation_rpc_methods": [],
        "strings_inventory": "passed",
        "symbols_inventory": "passed",
    }
    if plugin != expected_plugin:
        fail("protocol-1.1 native receipt plugin inventory drifted")

    digests = require_object(value.get("source_digests"), "receipt.source_digests")
    mapping = require_object(
        contract.get("required_source_digests"), "contract.required_source_digests"
    )
    if set(digests) != set(mapping):
        fail("protocol-1.1 native receipt source-digest key set drifted")
    normalized_digests = {
        name: require_hash(digests.get(name), f"receipt.source_digests.{name}")
        for name in mapping
    }
    validate_base_source_binding(embedded_source_digests, normalized_digests)
    if value.get("capabilities_granted") != [] or value.get("mutation_capabilities") != []:
        fail("protocol-1.1 native receipt grants authority")
    if value.get("claims_established") != contract["claims_established"]:
        fail("protocol-1.1 native receipt established-claim set drifted")
    if value.get("claims_not_established") != contract["claims_not_established"]:
        fail("protocol-1.1 native receipt limitation set drifted")
    declared = require_hash(value.get("receipt_digest"), "receipt.receipt_digest")
    unsigned = dict(value)
    del unsigned["receipt_digest"]
    if declared != sha256_bytes(canonical_json(unsigned)):
        fail("protocol-1.1 native receipt digest is invalid")
    return {
        **value,
        "source_digests": normalized_digests,
    }


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
        directory = os.open(path.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    except BaseException:
        try:
            os.unlink(temporary)
        except OSError:
            pass
        raise


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base_receipt", type=Path)
    parser.add_argument("--source-root", type=Path, default=ROOT)
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--validate", action="store_true")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        contract = load_contract(args.contract)
        if args.validate:
            value = promotion.read_object(
                args.base_receipt, MAX_JSON_BYTES, "protocol-1.1 native receipt"
            )
            receipt = validate_receipt(value, contract)
        else:
            receipt = issue(
                args.base_receipt,
                args.source_root,
                args.contract,
            )
        if args.output is None:
            print(json.dumps(receipt, sort_keys=True, separators=(",", ":")))
        else:
            write_atomic(args.output, receipt)
    except (OSError, promotion.PromotionError, ReceiptError) as exc:
        print(f"protocol-1.1 native receipt: FAIL: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
