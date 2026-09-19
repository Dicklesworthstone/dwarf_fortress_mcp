#!/usr/bin/env python3
"""Independent fixed-vector receipt checks, NOT execution of the Rust decoder."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from test_work_orders_native import reference_vectors

ROOT = Path(__file__).resolve().parents[1]
PREFIX = b"dfmcp-work-order-receipt/1\0"


def receipt(raw: bytes) -> bytes:
    return hashlib.sha256(PREFIX + raw[8:16] + raw[227:] + raw[73:195]).digest()


def validate(raw: bytes, expected: bytes) -> None:
    if not 230 <= len(raw) <= 357 or raw[:121] != expected[:121]:
        raise ValueError("incomplete or wrong sealed preparation")
    key_length = struct.unpack_from(">H", raw, 227)[0]
    if key_length != len(raw) - 229 or not 1 <= key_length <= 128 or raw[227:] != expected[227:]:
        raise ValueError("wrong key or noncanonical extent")
    state, known, tick = struct.unpack_from(">BBQ", raw, 121)
    if state not in (0, 1, 2, 4) or known != int(state == 2):
        raise ValueError("unknown state or inconsistent readback")
    if state == 2:
        if tick != 12345 or raw[131:195] != expected[131:195]:
            raise ValueError("not the exact controlled queue and template")
    elif tick != 0 or raw[131:195] != bytes(64):
        raise ValueError("nonzero absent backing")
    if raw[195:227] != (receipt(raw) if state in (2, 4) else bytes(32)):
        raise ValueError("wrong terminal receipt or nonzero nonterminal checksum")


def rejected(raw: bytes, expected: bytes) -> None:
    try:
        validate(raw, expected)
    except (ValueError, struct.error):
        return
    raise AssertionError("invalid evidence was accepted")


def main() -> None:
    vectors = reference_vectors()
    for name, expected in vectors.items():
        path = ROOT / "crates/dfmcp-adapter/tests/fixtures" / f"work_order_{name}_v1_10.hex"
        if bytes.fromhex(path.read_text(encoding="ascii")) != expected:
            raise AssertionError(f"native fixture mismatch: {name}")
    expected = vectors["created"]
    for name in ("prepared", "created", "unknown", "refused"):
        validate(vectors[name], expected)
    corruptions = 0
    for position in range(len(expected)):
        for bit in range(8):
            bad = bytearray(expected)
            bad[position] ^= 1 << bit
            rejected(bytes(bad), expected)
            corruptions += 1
    prefixes = 0
    for name in ("prepared", "created", "unknown", "refused"):
        for size in range(len(vectors[name])):
            rejected(vectors[name][:size], expected)
            prefixes += 1
        rejected(vectors[name] + b"\0", expected)
    forgeries = 0
    # Change identity/readback and recompute an internally consistent receipt.
    for position in (15, 23, 31, 35, 36, 40, 73, 105, 123, 130, 131, 162, 163, 194, 229):
        bad = bytearray(expected)
        bad[position] ^= 1
        bad[195:227] = receipt(bad)
        rejected(bytes(bad), expected)
        forgeries += 1
    for state in (3, 5, 255):
        bad = bytearray(vectors["refused"])
        bad[121] = state
        bad[195:227] = receipt(bad)
        rejected(bytes(bad), expected)
        forgeries += 1
    for name in ("prepared", "unknown", "refused"):
        for position in (122, 123, 131, 163):
            bad = bytearray(vectors[name])
            bad[position] = 1
            if name == "refused":
                bad[195:227] = receipt(bad)
            rejected(bytes(bad), expected)
            forgeries += 1
    files = ["crates/dfmcp-adapter/src/work_orders.rs", "crates/dfmcp-adapter/src/work_orders/rpc.rs",
             "crates/dfmcp-adapter/src/work_orders/tests.rs", "crates/dfmcp-adapter/src/work_orders/rpc_tests.rs",
             "scripts/check_work_order_vectors.py", "scripts/test_work_orders_native.py"]
    report = {
        "schema": "dfmcp.work-order-reference-evidence/1", "status": "passed_reference_only",
        "native_vectors": len(vectors), "effect_bit_corruptions_rejected": corruptions,
        "incomplete_effect_prefixes_rejected": prefixes, "trailing_records_rejected": 4,
        "rehashed_or_noncanonical_outcomes_rejected": forgeries,
        "rust_tests_registered": 16, "rust_compiled": False, "rust_tests_executed": False,
        "native_sdk_or_live_game_executed": False,
        "scope": "Python fixed-plan codec/checksum reference, not Rust, RPC or filesystem execution",
        "source_sha256": {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in files},
    }
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
