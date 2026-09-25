#!/usr/bin/env python3
"""Independent bytes for Rust excavation-run fixtures; does not execute Rust.

These inputs match the C++ bridge golden scenario (generation 41, sequence 3,
region1/site2, a 2x2 rectangle at 15/15/2). Only --write publishes fixtures;
normal execution compares exact checked-in bytes, never repairs them.
"""
from pathlib import Path
import argparse
import hashlib
import struct

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / 'crates/dfmcp-adapter/tests/fixtures'


def digest(domain, data):
    return hashlib.sha256(domain.encode() + b'\0' + data).digest()


def field(value):
    return struct.pack('>H', len(value)) + value


def capture(tick, sequence, paused, cell):
    return (b'DFMEC018DFMRO013' + struct.pack('>QQQBBB', 41, sequence, tick, 1, 1, paused)
            + struct.pack('>IIII', 2, 64, 64, 8) + field(b'region1')
            + struct.pack('>IIIIIH', 15, 15, 2, 2, 2, 4) + cell * 4)


def vectors():
    before = capture(806500, 3, 1, bytes([2, 2, 0, 1]))
    after = capture(806504, 4, 0, bytes([2, 3, 0, 0]))
    spec = struct.pack('>IIIIII', 100, 1000, 2, 2, 1, 10)
    plan = digest('dfmcp-excavation-run-plan/1', spec + before)
    key = field(b'golden')
    token = digest('dfmcp-excavation-run-token/1', key + plan)[:16]
    prefix = b'DFMER018' + key + spec + field(before) + plan + token
    prepared = prefix + bytes(5) + struct.pack('>QBIQQQB', 0, 0, 0, 0, 806500, 806500, 0)
    stopped = (prefix + bytes([3, 3, 1, 1, 1])
               + struct.pack('>QBIQQQB', 806504, 1, 2, 806501, 806504, 806504, 1) + field(after))
    result = {'capture': before, 'plan': plan, 'token': token,
            'intent': b'DFMEP018' + key + spec + field(before),
            'prepared': prepared + digest('dfmcp-excavation-run-receipt/1', prepared),
            'stopped': stopped + digest('dfmcp-excavation-run-receipt/1', stopped)}
    binding = (field(b'127.0.0.1:5000') + struct.pack('>QI', 41, 2) + field(b'region1')
               + struct.pack('>III', 64, 64, 8) + field(b'df') + field(b'dfhack'))
    journal, head = b'DFMEJ018', bytes(32)
    payloads = [b'\0' + binding, b'\1' + field(result['intent']),
                b'\2' + key + field(result['prepared']), b'\3' + key + plan,
                b'\4' + key + field(result['stopped'])]
    for sequence, payload in enumerate(payloads):
        frame = struct.pack('>II', len(payload), sequence) + head + payload
        head = digest('dfmcp-excavation-coordinator/1', frame)
        journal += frame + head
    result['journal'] = journal
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    for name, raw in vectors().items():
        path = FIXTURES / f'excavation_run_{name}_v1_18.hex'
        expected = (raw.hex() + '\n').encode('ascii')
        if args.write:
            # Deliberately refuse overwriting a prior golden file.
            with path.open('xb') as out:
                out.write(expected)
        if path.read_bytes() != expected:
            raise SystemExit('fixture differs: ' + name)
        print(f'{name}: {len(raw)} bytes; independent SHA-256 {hashlib.sha256(raw).hexdigest()}')
    print(f'PASS: {len(vectors())} independent fixtures; Rust compilation/execution NOT performed')


if __name__ == '__main__':
    main()
