#!/usr/bin/env python3
"""Independent dig journal framing/transition reference; does NOT execute Rust.

Run alongside, not instead of, cargo test -p dfmcp-adapter dig_designation.
Uses the already checked-in native fixtures and standard-library hashing only.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

ROOT = Path(__file__).resolve().parents[1]
MAGIC, FRAME, END = b'DFMDJ001', b'DFMDJR01', b'DFMDJEND'
MAX_BODY, MAX_FRAME = 16870, 16962
VALID = [(0, None), (1, 0), (2, 0), (3, 0), (3, 1), (4, None), (4, 0), (4, 1), (5, 2), (5, 4)]


def sha(domain: bytes, data: bytes) -> bytes:
    return hashlib.sha256(domain + b'\0' + data).digest()


def text(value: bytes) -> bytes:
    return struct.pack('>H', len(value)) + value


def transition(old: tuple[int, int | None] | None, new: tuple[int, int | None]) -> bool:
    if new not in VALID:
        return False
    if old is None:
        return new == (0, None)
    a, phase = old; b, after = new
    if old == new or a == 5 or phase == 1 or phase is not None and after is None:
        return False
    if a == 0 and b in (1, 3, 5):
        return True
    if a in (0, 1, 2, 3) and b == 4 or a == 1 and b == 2:
        return phase == after
    return (a in (1, 2, 3) and b in (3, 5)) or (a == 4 and b in (4, 5))


def frame(body: bytes, sequence: int, previous: bytes) -> bytes:
    prefix = FRAME + struct.pack('>IQ', len(body), sequence) + previous + body
    return prefix + sha(b'dfmcp-dig-journal-frame/1', prefix) + END


def decode(raw: bytes, binding: bytes, plan: bytes, native: dict[str, bytes]) -> list[tuple[int, int | None]]:
    header = MAGIC + struct.pack('>H', len(binding)) + binding + bytes([7]) * 32
    digest = sha(b'dfmcp-dig-journal/1', header)
    assert raw.startswith(header + digest)
    at = len(header) + 32
    records = []
    while at < len(raw):
        start = at
        assert raw[at:at + 8] == FRAME
        size, sequence = struct.unpack_from('>IQ', raw, at + 8)
        assert size <= MAX_BODY and sequence == len(records) + 1 and sequence <= 896
        assert raw[at + 20:at + 52] == digest
        at += 52
        body = raw[at:at + size]
        assert len(body) == size
        at += size
        digest = sha(b'dfmcp-dig-journal-frame/1', raw[start:at])
        assert raw[at:at + 32] == digest and raw[at + 32:at + 40] == END
        at += 40
        state = body[0]
        n = struct.unpack_from('>I', body, 1)[0]
        assert n <= 16527 and body[5:5 + n] == plan
        m = struct.unpack_from('>I', body, 5 + n)[0]
        effect = body[9 + n:]
        assert m <= 334 and len(effect) == m
        assert not effect or effect in native.values()
        phase = effect[133] if effect else None
        new = (state, phase)
        assert transition(records[-1] if records else None, new)
        records.append(new)
    assert at == len(raw)
    return records


def main() -> None:
    native = {name: bytes.fromhex((ROOT / f'tests/native/dig_designation/vectors/{name}.hex').read_text())
              for name in ('observation', 'prepared', 'designated', 'cancelled')}
    # These bytes independently mirror the Rust fixture helper, not a Python
    # substitute used by the coordinator. The existing native wire is unchanged.
    binding = (text(b'127.0.0.1:5000') + struct.pack('>Q', 7) + text(b'test-df')
               + text(b'test-dfhack') + text(b'region1') + struct.pack('>Iiiiiii', 1, 0, 0, 0, 63, 63, 7))
    plan = b'DFMDGP16' + text(b'dig-001') + b'\0' + struct.pack('>I', len(native['observation'])) + native['observation']
    prefix = MAGIC + struct.pack('>H', len(binding)) + binding + bytes([7]) * 32
    header_id = sha(b'dfmcp-dig-journal/1', prefix)
    raw = prefix + header_id; boundaries = [len(raw)]; heads = []
    def body(state: int, effect: bytes = b'') -> bytes:
        return bytes([state]) + struct.pack('>I', len(plan)) + plan + struct.pack('>I', len(effect)) + effect
    previous = header_id
    for index, (state, effect) in enumerate([(0, b''), (1, native['prepared']), (2, native['prepared']), (5, native['designated'])], 1):
        encoded = frame(body(state, effect), index, previous)
        previous = encoded[-40:-8]
        raw += encoded; boundaries.append(len(raw)); heads.append(previous.hex())
    assert decode(raw, binding, plan, native)[-1] == (5, 2)
    corruptions = 0
    for at in range(len(raw)):
        bad = bytearray(raw); bad[at] ^= 1
        try:
            decode(bytes(bad), binding, plan, native)
        except (AssertionError, IndexError, struct.error):
            corruptions += 1
        else:
            raise AssertionError(f'undetected byte corruption at {at}')
    prefixes = 0
    for size in range(len(raw)):
        try:
            result = decode(raw[:size], binding, plan, native)
        except (AssertionError, IndexError, struct.error):
            assert size not in boundaries
            prefixes += 1
        else:
            assert size in boundaries and (not result or result[-1][0] != 5)
    # Rehashing a forbidden transition cannot turn native Unknown into success.
    unknown = bytearray(native['prepared']); unknown[133] = 1
    unknown = bytes(unknown)
    hostile = prefix + header_id
    for index, b in enumerate([body(0), body(3, unknown), body(5, native['designated'])], 1):
        previous = header_id if index == 1 else hostile[-40:-8]
        hostile += frame(b, index, previous)
    try:
        decode(hostile, binding, plan, {**native, 'unknown': unknown})
    except AssertionError:
        pass
    else:
        raise AssertionError('native Unknown incorrectly became terminal')
    edges = sum(transition(a, b) for a in VALID for b in VALID)
    def longest(node: tuple[int, int | None], seen: frozenset) -> int:
        assert node not in seen, 'cyclic non-idempotent history'
        return 1 + max((longest(n, seen | {node}) for n in VALID if transition(node, n)), default=0)
    depth = longest((0, None), frozenset())
    assert depth <= 7 and 128 * 7 <= 896
    assert 1014 + 896 * MAX_FRAME < 16 * 1024 * 1024
    for a in VALID:
        if a[0] == 5 or a[1] == 1:
            assert not any(transition(a, b) for b in VALID)
    source = (ROOT / 'crates/dfmcp-adapter/src/dig_designation/journal.rs').read_text()
    assert 'fresh_key: None' in source and 'self.fresh_key = None;' in source
    expected = json.loads((ROOT / 'architecture/dig_journal_v1.json').read_text())
    assert expected['reference_vectors'] == {'header_id': header_id.hex(), 'heads': heads}
    print(json.dumps({'status': 'passed_independent_reference_only', 'rust_executed': False,
        'rust_compiled': False, 'native_or_filesystem_execution': False, 'journal_bytes': len(raw),
        'single_byte_corruptions_rejected': corruptions, 'incomplete_prefixes_rejected': prefixes,
        'complete_crash_prefixes_accepted': len(boundaries) - 1,
        'transition_pairs_checked': len(VALID) ** 2, 'allowed_transitions': edges, 'maximum_history_frames': depth,
        'header_id': header_id.hex(), 'heads': heads}, indent=2))


if __name__ == '__main__':
    main()
