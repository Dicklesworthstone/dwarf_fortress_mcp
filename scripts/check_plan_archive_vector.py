#!/usr/bin/env python3
"""Independent byte construction for the Rust exact-plan archive golden vector.

This checks the persisted fixture and old semantic identity against an independently
written encoder. It does NOT execute or qualify the Rust implementation.
"""
from pathlib import Path
import hashlib
import struct

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / 'crates/dfmcp-intent/tests/fixtures'

def digest(raw):
    return hashlib.sha256(raw).digest()

def u32(n):
    return struct.pack('>I', n)

def u64(n):
    return struct.pack('>Q', n)

def u128(n):
    return n.to_bytes(16, 'big')

def framed(raw):
    return u64(len(raw)) + raw

def text(raw):
    raw = raw.encode('utf-8')
    return u32(len(raw)) + raw

def vector():
    source = digest(b'archive-source')
    source_anchor = u64(42) + u64(3) + u64(7) + u64(100)
    intent = u128(9)
    action = b'\x00\x00'
    before, after = b'\x06\x01', b'\x06\x00'
    compensation = b'\x00\x01'
    summary = b'Restore the original pause plan'
    key = digest(b'dfmcp-step-idempotency-v1' + intent + u32(0) + source + action).hex().encode()
    preimage = (framed(b'dfmcp-prepared-plan-v1') + intent + source_anchor + framed(source)
        + framed(summary) + framed(after) + u64(1) + u32(0) + framed(action)
        + u64(1) + framed(before) + u64(1) + framed(after)
        + b'\x01' + framed(compensation) + b'\x00' + u64(0) + b'\x01\x09' + framed(key)
        + b'\x01' + u64(1) + b'\x09' + b'\x00' + u64(200))
    semantic = digest(preimage)
    plan_id = semantic[:16] if semantic[:16] != b'\0' * 16 else u128(1)
    body = (plan_id + intent + source_anchor + source + text(summary.decode()) + after
        + u32(1) + u32(0) + action + u32(1) + before + u32(1) + after
        + b'\x01' + compensation + b'\x00' + u32(0) + b'\x01\x09' + text(key.decode())
        + b'\x01' + u32(1) + b'\x09' + b'\x00' + u64(200) + semantic)
    frame = b'DFMPAR01' + u32(len(body)) + body
    return frame + digest(b'dfmcp-exact-plan-archive/1\0' + frame), semantic, preimage

def main():
    expected, semantic, _ = vector()
    actual = bytes.fromhex((FIXTURES / 'plan_archive_pause_v1.hex').read_text().strip())
    assert actual == expected, 'persisted exact-plan archive fixture differs'
    assert (FIXTURES / 'plan_archive_pause_v1.plan-digest').read_text().strip() == semantic.hex()
    assert struct.unpack('>I', actual[8:12])[0] == len(actual) - 44
    assert actual[-32:] == digest(b'dfmcp-exact-plan-archive/1\0' + actual[:-32])
    print(f'Independent archive vector: {len(actual)} bytes; semantic {semantic.hex()}')
    print('Fixture, frame length and object checksum passed. Rust source not executed.')

if __name__ == '__main__':
    main()
