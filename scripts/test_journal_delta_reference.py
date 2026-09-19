#!/usr/bin/env python3
"""Independent byte-delta reference vectors, NOT execution of the Rust codec."""
from __future__ import annotations

import hashlib
import json
import random
import struct
from pathlib import Path

MAGIC = b"DFMDLT01"
MAX_BYTES = 16 * 1024 * 1024
MAX_COMMANDS = 32768
SAVINGS = 128
COPY_MINIMUM = 32


def encode(base: bytes, target: bytes) -> bytes | None:
    if len(base) > MAX_BYTES or len(target) > MAX_BYTES:
        raise ValueError("payload limit")
    limit = len(target) - SAVINGS
    if limit < 29:
        return None
    suffix = 0
    for a, b in zip(reversed(base), reversed(target)):
        if a != b:
            break
        suffix += 1
    if suffix < COPY_MINIMUM:
        suffix = 0
    end = len(target) - suffix
    commands: list[bytes] = []
    stored = 20

    def emit(command: bytes) -> bool:
        nonlocal stored
        if len(commands) >= MAX_COMMANDS or stored + len(command) > limit:
            return False
        commands.append(command)
        stored += len(command)
        return True

    pending = position = 0
    while position < end:
        start = position
        while position < end and position < len(base) and target[position] == base[position]:
            position += 1
        if position - start >= COPY_MINIMUM:
            if start != pending and not emit(b"\0" + struct.pack(">I", start - pending) + target[pending:start]):
                return None
            if not emit(b"\1" + struct.pack(">II", start, position - start)):
                return None
            pending = position
        if position < end:
            position += 1
    if pending < end and not emit(b"\0" + struct.pack(">I", end - pending) + target[pending:end]):
        return None
    if suffix and not emit(b"\1" + struct.pack(">II", len(base) - suffix, suffix)):
        return None
    return struct.pack(">8sIII", MAGIC, len(base), len(target), len(commands)) + b"".join(commands)


def decode(base: bytes, delta: bytes, maximum: int = MAX_BYTES) -> bytes:
    if maximum < 0 or maximum > MAX_BYTES or len(base) > MAX_BYTES or len(delta) > MAX_BYTES:
        raise ValueError("limit")
    if len(delta) < 20:
        raise ValueError("header")
    magic, base_size, target_size, count = struct.unpack_from(">8sIII", delta)
    if magic != MAGIC or base_size != len(base):
        raise ValueError("base/header")
    if target_size > maximum or not 1 <= count <= MAX_COMMANDS:
        raise ValueError("expanded/command limit")
    position = 20
    parts: list[bytes] = []
    produced = 0
    for _ in range(count):
        if position + 5 > len(delta):
            raise ValueError("truncated command")
        tag = delta[position]
        argument = struct.unpack_from(">I", delta, position + 1)[0]
        position += 5
        if tag == 0:
            length = argument
            if position + length > len(delta):
                raise ValueError("truncated literal")
            part = delta[position:position + length]
            position += length
        elif tag == 1:
            if position + 4 > len(delta):
                raise ValueError("truncated copy")
            length = struct.unpack_from(">I", delta, position)[0]
            position += 4
            if argument > len(base) or length > len(base) - argument:
                raise ValueError("copy range")
            part = base[argument:argument + length]
        else:
            raise ValueError("tag")
        if length == 0 or produced + length > target_size:
            raise ValueError("output range")
        produced += length
        parts.append(part)
    if position != len(delta) or produced != target_size:
        raise ValueError("trailing/incomplete output")
    return b"".join(parts)


def golden() -> tuple[bytes, bytes, bytes]:
    base = bytes(range(256)) * 2
    target = bytearray(base)
    target[10] = 99
    target[290:294] = b"ABCD"
    delta = encode(base, bytes(target))
    assert delta is not None
    return base, bytes(target), delta


def main() -> None:
    checks = 0
    encoded_cases = 0
    raw_cases = 0

    def roundtrip(base: bytes, target: bytes) -> None:
        nonlocal checks, encoded_cases, raw_cases
        delta = encode(base, target)
        checks += 1
        if delta is None:
            raw_cases += 1
            return
        encoded_cases += 1
        assert len(target) - len(delta) >= SAVINGS
        assert decode(base, delta, len(target)) == target
        assert encode(base, target) == delta
        checks += 3

    rng = random.Random(0xDFA118)
    for _ in range(4096):
        size = rng.randrange(0, 8193)
        base = rng.randbytes(size)
        target = bytearray(base)
        for _ in range(rng.randrange(1, 9)):
            at = rng.randrange(len(target) + 1)
            change = rng.randrange(4)
            amount = rng.randrange(1, 65)
            if change == 0:
                target[at:at + amount] = rng.randbytes(amount)
            elif change == 1:
                target[at:at] = rng.randbytes(amount)
            elif change == 2:
                del target[at:at + amount]
            else:
                target[at:at + amount] = b"\0" * amount
        roundtrip(base, bytes(target))
    for size in (0, 1, 31, 32, 128, 156, 157, 158, 512, 4096, MAX_BYTES):
        base = b"x" * size
        roundtrip(base, base)
        if size:
            roundtrip(base, b"z" + base[1:])
            roundtrip(base, base[:-1])
        if size < MAX_BYTES:
            roundtrip(base, b"z" + base)
        roundtrip(base, b"y" * size)
    base, target, delta = golden()
    for prefix in range(len(delta)):
        try:
            decode(base, delta[:prefix])
        except ValueError:
            checks += 1
        else:
            raise AssertionError(f"accepted incomplete delta prefix {prefix}")
    malformed = [delta + b"x", b"x" + delta[1:], delta[:16] + struct.pack(">I", 0) + delta[20:],
                 delta[:16] + struct.pack(">I", MAX_COMMANDS + 1) + delta[20:],
                 delta[:12] + struct.pack(">I", MAX_BYTES + 1) + delta[16:],
                 delta[:8] + struct.pack(">I", len(base) + 1) + delta[12:]]
    for commands in (b"\2" + b"\0" * 4, b"\0" + b"\0" * 4,
                     b"\1" + struct.pack(">II", 0, 0),
                     b"\1" + struct.pack(">II", 0xFFFFFFFF, 512),
                     b"\1" + struct.pack(">II", 512, 1),
                     b"\1" + struct.pack(">II", 0, 513)):
        malformed.append(struct.pack(">8sIII", MAGIC, 512, 512, 1) + commands)
    for value in malformed:
        try:
            decode(base, value)
        except ValueError:
            checks += 1
        else:
            raise AssertionError("accepted malformed delta")
    try:
        decode(base, delta, len(target) - 1)
    except ValueError:
        checks += 1
    else:
        raise AssertionError("expanded budget bypass")
    # High fragmentation MUST select bounded raw storage, not exceed command limits.
    base = b"a" * (MAX_COMMANDS * 33)
    target = (b"a" * 32 + b"b") * MAX_COMMANDS
    assert encode(base, target) is None
    checks += 1
    _, target, delta = golden()
    report = {"reference_checks": checks, "compressed_roundtrips": encoded_cases,
              "raw_fallbacks": raw_cases, "golden_delta_hex": delta.hex(),
              "golden_delta_sha256": hashlib.sha256(delta).hexdigest(),
              "golden_target_sha256": hashlib.sha256(target).hexdigest(),
              "evidence": "Independent Python reference only; no Rust, journal, MCP or game execution"}
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
