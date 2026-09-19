#!/usr/bin/env python3
"""Independent fixed-plan creation-journal reference; does NOT execute Rust or disk custody."""
from __future__ import annotations
import hashlib
import json
from pathlib import Path
import struct

ROOT = Path(__file__).resolve().parents[1]
H = lambda data: hashlib.sha256(data).digest()
U32 = lambda value: struct.pack('>I', value)
U64 = lambda value: struct.pack('>Q', value)
BLOB = lambda data: U32(len(data)) + data
DOMAIN = b'dfmcp-work-order-journal-frame/1\0'


def main() -> None:
    fixture = ROOT / 'crates/dfmcp-adapter/tests/fixtures'
    obs = bytes.fromhex((fixture / 'work_order_observation_v1_10.hex').read_text())
    created = bytes.fromhex((fixture / 'work_order_created_v1_10.hex').read_text())
    key = b'order-001'
    key16 = struct.pack('>H', len(key)) + key
    witness = H(obs)
    plan = H(b'dfmcp-work-order-plan/1\0' + b'\1' + U32(5) + witness)
    token = H(b'dfmcp-work-order-token/1\0' + U64(7) + key16 + plan)[:16]
    after = bytearray(obs)
    after[16:24] = U64(1)
    after[32:36] = U32(11)
    after[50:54] = U32(3)
    after.extend(U32(10))
    config = H(b'DFMWOC10' + U32(10) + b'\1' + U32(5))
    prefix = b'DFMWOE10' + U64(7) + U64(0) + U64(12345) + U32(10) + b'\1' + U32(5) + witness + plan + token
    outcomes = {}
    for state in [0, 1, 2, 4]:
        known = state == 2
        fields = bytes([state, int(known)]) + U64(12345 if known else 0)
        fields += H(after) + config if known else bytes(64)
        receipt = H(b'dfmcp-work-order-receipt/1\0' + U64(7) + key16 + plan + token + fields) if state in [2, 4] else bytes(32)
        outcomes[state] = prefix + fields + receipt + key16
    assert outcomes[2] == created
    fortress = int.from_bytes(H(b'dfmcp-live-fortress-id-v1\0region1\0' + U32(1))[:8], 'big') | 1
    journal_id = H(b'dfmcp-work-order-journal-incarnation/1\0' + (11).to_bytes(16, 'big') + (12).to_bytes(16, 'big') + U64(fortress) + U64(0))
    h = b'DFMWOJ10' + journal_id + U64(fortress)
    header = h + H(h)

    def body(state: int, native: int) -> bytes:
        return bytes([state]) + BLOB(key) + BLOB(obs) + b'\1' + U32(5) + BLOB(b'df') + BLOB(b'dfhack') + BLOB(outcomes[native])

    def append(raw: bytes, state: int, native: int, number: int) -> bytes:
        head = raw[-40:-8] if len(raw) > 80 else raw[48:80]
        data = body(state, native)
        p = b'DFMWOR10' + U32(len(data)) + U64(number) + head
        return raw + p + data + H(DOMAIN + journal_id + p + data) + b'DFMWEND0'

    def validate(raw: bytes) -> list[int]:
        if len(raw) < 80 or raw[:48] != header[:48] or raw[48:80] != H(raw[:48]):
            raise ValueError('header')
        head, offset, states, native_states = raw[48:80], 80, [], []
        while offset < len(raw):
            if len(raw) - offset < 92: raise ValueError('tail')
            p = raw[offset:offset+52]
            size, number = struct.unpack_from('>IQ', p, 8)
            if p[:8] != b'DFMWOR10' or size > 19 * 1024 or number != len(states)+1 or p[20:] != head:
                raise ValueError('prefix')
            end = offset + 52 + size
            data = raw[offset+52:end]
            if len(raw) < end+40 or raw[end+32:end+40] != b'DFMWEND0' or raw[end:end+32] != H(DOMAIN+journal_id+p+data):
                raise ValueError('checksum or torn frame')
            state = data[0]
            matches = [n for n in outcomes if data == body(state, n)]
            if len(matches) != 1: raise ValueError('different complete sealed evidence')
            native = matches[0]
            shapes = {1: [0], 2: [0], 3: [0, 1], 4: [2], 5: [4], 6: [0]}
            if state not in shapes or native not in shapes[state]: raise ValueError('state shape')
            if not states and state in [2, 6]: raise ValueError('missing preparation')
            if states:
                old, old_native = states[-1], native_states[-1]
                allowed = (state, native) == (old, old_native)
                if not allowed and old == 1: allowed = state in [2, 6]
                if not allowed and old in [2, 3]: allowed = state in [3, 4, 5]
                if old_native == 1 and native != 1: allowed = False
                if not allowed: raise ValueError('illegal transition')
            states.append(state); native_states.append(native)
            head, offset = raw[end:end+32], end+40
        return states

    prepared = append(header, 1, 0, 1)
    started = append(prepared, 2, 0, 2)
    done = append(started, 4, 2, 3)
    unknown = append(started, 3, 1, 3)
    assert validate(done) == [1, 2, 4]
    assert validate(append(started, 3, 0, 3)) == [1, 2, 3]
    assert validate(append(prepared, 6, 0, 2)) == [1, 6]

    def rejects(raw: bytes) -> None:
        try: validate(raw)
        except (ValueError, IndexError, struct.error): return
        raise AssertionError('invalid reference journal accepted')

    for i in range(len(done)):
        bad = bytearray(done); bad[i] ^= 1; rejects(bytes(bad))
    boundaries = {80, len(prepared), len(started)}
    for end in range(len(done)):
        if end not in boundaries: rejects(done[:end])
    invalid = [append(done, 1, 0, 4), append(done, 2, 0, 4), append(done, 6, 0, 4),
               append(unknown, 4, 2, 4), append(prepared, 4, 2, 2), append(header, 2, 0, 1),
               append(started, 1, 0, 3), append(started, 6, 0, 3)]
    for raw in invalid: rejects(raw)
    paths = sorted((ROOT / 'crates/dfmcp-adapter/src/work_order_control').rglob('*.rs'))
    paths += [ROOT/'crates/dfmcp-adapter/src/work_order_control.rs', Path(__file__)]
    report = {'schema':'dfmcp.creation-journal-reference/1', 'status':'passed_reference_only',
              'journal_bytes':len(done), 'byte_corruptions_rejected':len(done),
              'incomplete_prefixes_rejected':len(done)-len(boundaries), 'rehashed_illegal_histories_rejected':len(invalid),
              'native_created_fixture_matches':True, 'rust_compiled':False, 'rust_tests_executed':False,
              'filesystem_crash_or_mcp_or_native_executed':False,
              'source_sha256':{str(p.relative_to(ROOT)):H(p.read_bytes()).hex() for p in paths}}
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == '__main__': main()
