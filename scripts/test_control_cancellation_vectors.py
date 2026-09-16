#!/usr/bin/env python3
"""Independent binary vectors; does not execute Rust or establish crash durability."""
from __future__ import annotations

import hashlib
import json
import struct

ZERO = bytes(32)
HEADER_DOMAIN = b"dfmcp-control-effect-journal-header/1\0"
FRAME_DOMAIN = b"dfmcp-control-effect-journal-frame-header/1\0"
RECORD_DOMAIN = b"dfmcp-control-effect-journal-record/1\0"


def sha(data: bytes) -> bytes:
    return hashlib.sha256(data).digest()


def frame(journal: bytes, previous: bytes, sequence: int, state: int,
          *, outcome: bytes = bytes(43), plan: bytes | None = None) -> bytes:
    key = b"key"
    body = (struct.pack(">Q32sQH", sequence, previous, sequence, len(key)) + key
            + (sha(b"cancel-this-pause-plan") if plan is None else plan)
            + struct.pack(">BQQB16s", 1, 10, 7, state, bytes([1]) * 16) + outcome)
    prefix = b"DFMCREC1" + struct.pack(">I", len(body))
    encoded = prefix + sha(FRAME_DOMAIN + journal + prefix) + body
    return encoded + sha(RECORD_DOMAIN + journal + encoded) + b"DFMCEND1"


def verify(data: bytes, *, maximum_tag: int = 6) -> list[tuple[int, bytes]]:
    """Verify the narrowly scoped Prepared -> CancelledBeforeDispatch fixture."""
    if len(data) < 72 or data[:8] != b"DFMCEJ01":
        raise ValueError("header")
    journal, previous = data[8:40], data[40:72]
    if journal == ZERO or sha(HEADER_DOMAIN + data[:40]) != previous:
        raise ValueError("header digest")
    offset, identity, records = 72, None, []
    while offset < len(data):
        prefix = data[offset:offset + 44]
        if len(prefix) != 44 or prefix[:8] != b"DFMCREC1":
            raise ValueError("frame header")
        length = struct.unpack(">I", prefix[8:12])[0]
        if not 120 <= length <= 768 or prefix[12:] != sha(FRAME_DOMAIN + journal + prefix[:12]):
            raise ValueError("frame header digest")
        end = offset + 44 + length
        body, digest, footer = data[offset + 44:end], data[end:end + 32], data[end + 32:end + 40]
        if len(body) != length or footer != b"DFMCEND1" or digest != sha(RECORD_DOMAIN + journal + data[offset:end]):
            raise ValueError("frame digest or incomplete record")
        sequence, predecessor, revision, key_length = struct.unpack(">Q32sQH", body[:50])
        key_end = 50 + key_length
        if not 1 <= key_length <= 512 or len(body) != key_end + 109:
            raise ValueError("body bounds")
        state = body[key_end + 49]
        current_identity = body[50:key_end + 49] + body[key_end + 50:key_end + 66]
        if sequence != len(records) + 1 or revision != sequence or predecessor != previous:
            raise ValueError("chain")
        if state > maximum_tag or (not records and state != 1) or (records and state != 6):
            raise ValueError("unsupported fixture transition")
        if len(records) > 1 or (identity is not None and current_identity != identity):
            raise ValueError("immutable or terminal identity")
        if body[key_end + 66:] != bytes(43):
            raise ValueError("undispatched state carries native outcome")
        identity, previous, offset = current_identity, digest, end + 40
        records.append((state, digest))
    return records


def must_reject(data: bytes, **kwargs: int) -> None:
    try:
        verify(data, **kwargs)
    except ValueError:
        return
    raise AssertionError("reference accepted invalid fixture")


def main() -> None:
    incarnation = (b"dfmcp-control-effect-journal-incarnation/1\0"
                   + (81).to_bytes(16, "big") + (1).to_bytes(16, "big") + (7).to_bytes(8, "big"))
    journal = sha(incarnation)
    prefix = b"DFMCEJ01" + journal
    header = prefix + sha(HEADER_DOMAIN + prefix)
    prepared = frame(journal, header[-32:], 1, 1)
    cancelled = frame(journal, prepared[-40:-8], 2, 6)
    encoded = header + prepared + cancelled
    assert [state for state, _ in verify(encoded)] == [1, 6]
    assert verify(header) == []
    assert [state for state, _ in verify(header + prepared)] == [1]
    for index in range(len(encoded)):
        damaged = bytearray(encoded)
        damaged[index] ^= 1
        must_reject(bytes(damaged))
    incomplete = 0
    for length in range(len(encoded)):
        if length not in (len(header), len(header + prepared)):
            must_reject(encoded[:length])
            incomplete += 1
    for index in range(43):
        outcome = bytearray(43)
        outcome[index] = 1
        must_reject(header + prepared + frame(journal, prepared[-40:-8], 2, 6, outcome=bytes(outcome)))
    must_reject(encoded, maximum_tag=5)
    must_reject(header + prepared + frame(journal, ZERO, 2, 6))
    must_reject(header + prepared + frame(journal, prepared[-40:-8], 2, 6, plan=sha(b"different")))
    must_reject(encoded + frame(journal, cancelled[-40:-8], 3, 6))
    print(json.dumps({
        "scope": "independent_prepared_cancellation_binary_fixture_only",
        "rust_executed": False, "filesystem_durability_tested": False,
        "single_byte_corruptions_rejected": len(encoded),
        "incomplete_prefixes_rejected": incomplete,
        "validly_hashed_outcome_mutations_rejected": 43,
        "other_invalid_or_legacy_cases_rejected": 4,
        "vector": {"session_id": 81, "request_id": 1, "bridge_generation": 7,
                   "journal_id": journal.hex(), "header_digest": header[-32:].hex(),
                   "prepared_digest": prepared[-40:-8].hex(), "cancelled_digest": cancelled[-40:-8].hex(),
                   "journal_length": len(encoded), "journal_sha256": sha(encoded).hex()}
    }, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
