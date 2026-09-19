#!/usr/bin/env python3
"""Independent storage-framing reference; native payloads/anchors are opaque fixtures.

This does not execute Rust, filesystem custody, canonical projection or MCP.
"""
from __future__ import annotations
import hashlib
import json
import struct
from dataclasses import dataclass
from test_journal_delta_reference import encode as delta_encode, decode as delta_decode, MAX_BYTES

RAW = b"DFMOREC1"
DELTA = b"DFMODLT1"
END = b"DFMOEND1"
ID = hashlib.sha256(b"delta-frame-reference-incarnation").digest()
HEADER_DOMAIN = b"dfmcp-operations-journal-header/1\0"
LENGTH_DOMAIN = b"dfmcp-operations-journal-frame-header/1\0"
FRAME_DOMAIN = b"dfmcp-operations-journal-record/1\0"


def sha(data: bytes) -> bytes:
    return hashlib.sha256(data).digest()


def header() -> bytes:
    prefix = b"DFMUJ001" + struct.pack(">Q", 1) + ID
    return prefix + sha(HEADER_DOMAIN + prefix)


@dataclass
class Record:
    number: int
    epoch: int
    sequence: int
    tick: int
    payload: bytes
    digest: bytes
    is_delta: bool
    stored: int


def make_frame(payload: bytes, number: int, previous: bytes, base: Record | None,
               *, epoch: int = 0, sequence: int | None = None, raw: bool = False) -> bytes:
    sequence = number - 1 if sequence is None else sequence
    packed = None
    if base is not None and not raw and number % 64 != 1 and base.epoch == epoch and base.sequence < sequence:
        assert previous == base.digest
        packed = delta_encode(base.payload, payload)
    marker = RAW if packed is None else DELTA
    stored = payload if packed is None else packed
    # Source/state digests below are deliberately opaque-fixture identities,
    # NOT the native profile's source or canonical-world hash derivation.
    tick = number + 100
    body = struct.pack(">Q", number) + previous + struct.pack(">QQQQ", 1, epoch, sequence, tick)
    body += sha(b"opaque-state\0" + payload) + sha(payload) + struct.pack(">Q", 7)
    for text in (b"df", b"dfhack"):
        body += struct.pack(">H", len(text)) + text
    body += struct.pack(">I", len(stored)) + stored
    prefix = marker + struct.pack(">I", len(body))
    out = prefix + sha(LENGTH_DOMAIN + ID + prefix) + body
    return out + sha(FRAME_DOMAIN + ID + out) + END


def frame_length(prefix: bytes, allow_delta: bool = True) -> int:
    if len(prefix) != 44 or prefix[:8] not in ((RAW, DELTA) if allow_delta else (RAW,)):
        raise ValueError("unsupported marker")
    length = struct.unpack_from(">I", prefix, 8)[0]
    if not 148 <= length <= MAX_BYTES + 1024 or prefix[12:] != sha(LENGTH_DOMAIN + ID + prefix[:12]):
        raise ValueError("length/checksum")
    return length + 84


def read_frame(frame: bytes, number: int, previous: bytes, base: Record | None,
               *, maximum: int = MAX_BYTES, allow_delta: bool = True) -> Record:
    if len(frame) != frame_length(frame[:44], allow_delta):
        raise ValueError("frame length")
    if frame[-8:] != END or frame[-40:-8] != sha(FRAME_DOMAIN + ID + frame[:-40]):
        raise ValueError("frame checksum/footer")
    body = frame[44:-40]
    n = struct.unpack_from(">Q", body)[0]
    if n != number or body[8:40] != previous:
        raise ValueError("chain")
    fortress, epoch, sequence, tick = struct.unpack_from(">QQQQ", body, 40)
    if fortress != 1:
        raise ValueError("fortress")
    pos = 144
    for _ in range(2):
        size = struct.unpack_from(">H", body, pos)[0]
        pos += 2
        if not 1 <= size <= 128 or pos + size > len(body):
            raise ValueError("manifest")
        text = body[pos:pos + size].decode("utf8")
        if "\0" in text:
            raise ValueError("manifest NUL")
        pos += size
    size = struct.unpack_from(">I", body, pos)[0]
    pos += 4
    if len(body) != pos + size:
        raise ValueError("payload framing")
    stored = body[pos:]
    compressed = frame[:8] == DELTA
    if compressed:
        if base is None or base.digest != previous or number % 64 == 1 or base.epoch != epoch or base.sequence >= sequence:
            raise ValueError("delta base/epoch/keyframe")
        payload = delta_decode(base.payload, stored, maximum)
    else:
        if len(stored) > maximum:
            raise ValueError("expanded budget")
        payload = stored
    if body[104:136] != sha(payload) or body[72:104] != sha(b"opaque-state\0" + payload):
        raise ValueError("opaque fixture identity")
    return Record(n, epoch, sequence, tick, payload, frame[-40:-8], compressed, len(stored))


def read_archive(data: bytes, *, repair: bool = False, allow_delta: bool = True) -> tuple[list[bytes], int]:
    if len(data) < 80 or data[:80] != header():
        raise ValueError("file header")
    previous = data[48:80]
    offset = 80
    base = None
    digests = []
    while offset < len(data):
        remaining = data[offset:]
        if len(remaining) < 44:
            markers = (RAW, DELTA) if allow_delta else (RAW,)
            n = min(len(remaining), 8)
            if not any(remaining[:n] == marker[:n] for marker in markers):
                raise ValueError("unknown partial marker")
            if repair:
                return digests, offset
            raise ValueError("incomplete header")
        length = frame_length(remaining[:44], allow_delta)
        if length > len(remaining):
            if repair:
                return digests, offset
            raise ValueError("incomplete body")
        base = read_frame(remaining[:length], len(digests) + 1, previous, base, allow_delta=allow_delta)
        previous = base.digest
        digests.append(sha(base.payload))
        offset += length
    return digests, offset


def main() -> None:
    checks = 0
    first = bytes(range(256)) * 2
    next_payload = bytearray(first)
    next_payload[10] = 99
    next_payload[290:294] = b"ABCD"
    h = header()
    raw = make_frame(first, 1, h[48:], None)
    a = read_frame(raw, 1, h[48:], None)
    compressed = make_frame(bytes(next_payload), 2, a.digest, a)
    b = read_frame(compressed, 2, a.digest, a)
    assert b.payload == next_payload and b.is_delta
    checks += 1
    complete = h + raw + compressed
    boundary = len(h + raw)
    for end in range(boundary + 1, len(complete)):
        try:
            read_archive(complete[:end])
        except ValueError:
            checks += 1
        else:
            raise AssertionError("incomplete delta accepted without repair")
        digests, retained = read_archive(complete[:end], repair=True)
        assert retained == boundary and digests == [sha(first)]
        checks += 1
    for position in range(boundary, len(complete)):
        changed = bytearray(complete)
        changed[position] ^= 1
        try:
            read_archive(bytes(changed), repair=True)
        except ValueError:
            checks += 1
        else:
            raise AssertionError("corruption repaired or accepted")
    for offset, value in ((8, b"\0" * 32), (48, struct.pack(">Q", 2)),
                          (104, b"\0" * 32), (0, struct.pack(">Q", 65))):
        changed = bytearray(compressed)
        changed[44 + offset:44 + offset + len(value)] = value
        changed[-40:-8] = sha(FRAME_DOMAIN + ID + changed[:-40])
        try:
            read_archive(h + raw + changed, repair=True)
        except ValueError:
            checks += 1
        else:
            raise AssertionError("checksummed semantic corruption accepted")
    for allow_delta, data, expected in ((False, h + raw, True), (False, complete, False), (True, complete, True)):
        try:
            read_archive(data, repair=True, allow_delta=allow_delta)
            valid = True
        except ValueError:
            valid = False
        assert valid == expected
        checks += 1
    try:
        read_frame(compressed, 2, a.digest, a, maximum=511)
    except ValueError:
        checks += 1
    else:
        raise AssertionError("expanded acquisition allowance bypassed")
    # Demonstrate retention economics only on a declared synthetic trace.
    previous = h[48:]
    base = None
    archive = bytearray(h)
    expected = []
    raw_bytes = 80
    raw_count = delta_count = 0
    template = bytes(range(256)) * 1024
    for number in range(1, 257):
        target = bytearray(template)
        target[16:24] = struct.pack(">Q", number)
        target[8192:8200] = struct.pack(">Q", number * 7)
        target = bytes(target)
        frame = make_frame(target, number, previous, base)
        raw_bytes += len(make_frame(target, number, previous, base, raw=True))
        base = read_frame(frame, number, previous, base)
        assert base.payload == target
        previous = base.digest
        archive.extend(frame)
        expected.append(sha(target))
        raw_count += not base.is_delta
        delta_count += base.is_delta
        checks += 1
    actual, retained = read_archive(bytes(archive))
    assert actual == expected and retained == len(archive)
    assert raw_bytes > 64 * 1024 * 1024 > len(archive)
    assert (raw_count, delta_count) == (4, 252)
    checks += 3
    print(json.dumps({"framing_reference_checks": checks,
        "small_delta_frame_sha256": sha(compressed).hex(),
        "synthetic_records": 256, "synthetic_payload_bytes_each": len(template),
        "raw_archive_bytes": raw_bytes, "delta_archive_bytes": len(archive),
        "raw_keyframes": raw_count, "delta_records": delta_count,
        "storage_reduction_percent": round(100 * (1 - len(archive) / raw_bytes), 4),
        "evidence": "Independent Python framing and opaque-payload model only; not Rust/native canonical replay, filesystem, MCP or game execution"}, indent=2))


if __name__ == "__main__":
    main()
