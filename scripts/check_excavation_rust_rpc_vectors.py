#!/usr/bin/env python3
"""Independent 1.18 protobuf request fixtures. This does not execute Rust."""
import argparse
import hashlib
import json
from pathlib import Path
import struct

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / 'crates/dfmcp-adapter/tests/fixtures/excavation_run_rpc_v1_18.txt'


def vectors():
    def hashed(domain, data):
        return hashlib.sha256(domain + b'\0' + data).digest()
    def short(data):
        return struct.pack('>H', len(data)) + data
    clock = b'DFMRO013' + struct.pack('>QQQBBB', 41, 3, 806500, 1, 1, 1)
    region = (15, 15, 2, 2, 2)
    capture = (b'DFMEC018' + clock + struct.pack('>4I', 2, 64, 64, 8)
               + short(b'region1') + struct.pack('>5IH', *region, 4) + bytes((2, 2, 0, 1)) * 4)
    spec = (100, 1000, 2, 2, 1, 10)
    plan = hashed(b'dfmcp-excavation-run-plan/1', struct.pack('>6I', *spec) + capture)
    token = hashed(b'dfmcp-excavation-run-token/1', short(b'golden') + plan)[:16]
    assert plan.hex() == '5055fc61017140b5d058ea0cafd418e0831e16d51b5fdfbca95a7403dcc1e787'
    assert token.hex() == 'f55fb500d29b2132caece85ed6b7d390'

    def varint(n):
        out = []
        while n > 127:
            out.append(n % 128 + 128)
            n //= 128
        return bytes(out + [n])
    def protobuf(fields):
        out = b''
        for tag, value in sorted(fields.items()):
            if isinstance(value, bytes):
                out += varint(tag * 8 + 2) + varint(len(value)) + value
            else:
                out += varint(tag * 8) + varint(value)
        assert len(out) <= 2048
        return out
    base = {1: b't' * 32, 2: b'n' * 32, 3: 1, 4: 18}
    selection = dict(zip(range(11, 16), region))
    key = {5: b'golden', 9: plan}
    return {name: protobuf({**base, **fields}) for name, fields in (
        ('Handshake', {}), ('ObserveRun', selection),
        ('PrepareRun', {**key, 6: spec[0], 7: spec[1], 8: capture, **selection,
                        **dict(zip(range(16, 20), spec[2:]))}),
        ('CommitRun', {**key, 10: token}), ('QueryRun', key), ('CancelRun', {**key, 10: token}))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    values = vectors()
    rendered = ''.join(f'{name} {data.hex()}\n' for name, data in values.items()).encode('ascii')
    if args.write:
        FIXTURE.parent.mkdir(parents=True, exist_ok=True)
        FIXTURE.write_bytes(rendered)
    if FIXTURE.read_bytes() != rendered:
        raise ValueError('excavation RPC fixture differs from independent construction')
    print(json.dumps({'result': 'passed', 'scope': 'independent Python request-vector construction',
                      'rust_compiled': False, 'rust_executed': False,
                      'vectors': {name: {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
                                  for name, data in values.items()}}, indent=2, sort_keys=True))


if __name__ == '__main__':
    main()
