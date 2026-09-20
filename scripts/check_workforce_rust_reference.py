#!/usr/bin/env python3
"""Independent binary/readback reference for workforce Rust source, NOT Rust execution."""
from __future__ import annotations
import copy
import hashlib
import json
from pathlib import Path
import struct
import unittest

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / 'tests/native/workforce/vectors'


def need(value):
    if not value:
        raise ValueError('reference contract refused')


def digest(domain, data):
    return hashlib.sha256(domain + b'\0' + data).digest()


class Reader:
    def __init__(self, data): self.data, self.at = data, 0
    def take(self, n):
        need(n <= len(self.data) - self.at)
        result = self.data[self.at:self.at+n]; self.at += n; return result
    def number(self, n): return int.from_bytes(self.take(n), 'big')
    def string(self, maximum, empty=False):
        n = self.number(2); need(n <= maximum and (empty or n))
        text = self.take(n).decode(); need('\0' not in text); return text
    def flag(self):
        n = self.number(1); need(n <= 1); return n
    def mask(self, n):
        b = self.take(n); need(all(x <= 1 for x in b)); return list(b)
    def end(self): need(self.at == len(self.data))


def field(text):
    b = text.encode(); return len(b).to_bytes(2, 'big') + b


def citizen(r, n):
    a, b = r.number(4), r.number(4); need(a <= 2**31-1 and b <= 2**31-1)
    return [a, b, r.flag(), r.mask(n)]


def capture(data):
    need(len(data) <= 65536)
    r = Reader(data); need(r.take(8) == b'DFMWF017')
    g, s, t, site = r.number(8), r.number(8), r.number(8), r.number(4)
    need(0 < g < 2**64-1 and t <= (2**32-1)*403200+403199 and site <= 2**31-1)
    folder, paused, auto = r.string(512), r.flag(), r.flag()
    n = r.number(2); need(1 <= n <= 128)
    keys = [r.string(64) for _ in range(n)]; need(len(set(keys)) == n)
    count = r.number(2); need(count <= 64); details = []; total = 0
    for _ in range(count):
        name, flags, selected, mask = r.string(256, True), r.number(4), r.flag(), r.mask(n)
        count = r.number(2); total += count; need(total <= 4096)
        ids = [r.number(4) for _ in range(count)]
        need(ids == sorted(set(ids)) and all(x <= 2**31-1 for x in ids))
        details.append([name, flags, selected, mask, ids])
    count = r.number(2); need(1 <= count <= 32)
    units = [citizen(r, n) for _ in range(count)]; ids = [u[0] for u in units]
    need(ids == sorted(set(ids))); r.end()
    return dict(g=g, s=s, t=t, site=site, folder=folder, paused=paused, auto=auto, keys=keys, details=details, units=units)


def unit_bytes(u): return struct.pack('>IIB', *u[:3]) + bytes(u[3])


def encode(c):
    out = b'DFMWF017' + struct.pack('>QQQI', c['g'], c['s'], c['t'], c['site']) + field(c['folder'])
    out += bytes([c['paused'], c['auto']]) + len(c['keys']).to_bytes(2, 'big') + b''.join(field(k) for k in c['keys'])
    out += len(c['details']).to_bytes(2, 'big')
    for name, flags, selected, bits, ids in c['details']:
        out += field(name) + struct.pack('>IB', flags, selected) + bytes(bits) + len(ids).to_bytes(2, 'big')
        out += b''.join(i.to_bytes(4, 'big') for i in ids)
    out += len(c['units']).to_bytes(2, 'big') + b''.join(unit_bytes(u) for u in c['units'])
    need(capture(out) == c); return out


def expected(c, detail, assigned):
    need(c['paused'] and c['auto'] and c['s'] < 2**64-1 and 0 <= detail < len(c['details']))
    d = c['details'][detail]; need(d[2] and any(d[3]) and all(u[2] for u in c['units']))
    ids = {u[0] for u in c['units']}; members = set(d[4]); changed = ids-members if assigned else ids & members
    need(changed); out = copy.deepcopy(c); out['s'] += 1
    out['details'][detail][4] = sorted(members | ids if assigned else members - ids); encode(out)
    return out, changed


def effect(raw, before, key='assign', detail=0, assigned=1):
    c = capture(before); post, changed = expected(c, detail, assigned)
    plan = digest(b'dfmcp-workforce-plan/1', struct.pack('>IB', detail, assigned) + hashlib.sha256(before).digest())
    token = digest(b'dfmcp-workforce-token/1', field(key) + plan)[:16]
    r = Reader(raw); need(len(raw) <= 8192 and r.take(8) == b'DFMWE017')
    need(r.string(128) == key and r.take(32) == plan and r.take(16) == token)
    need(r.number(4) == detail and r.flag() == assigned and r.take(32) == hashlib.sha256(before).digest())
    need([r.number(8) for _ in range(3)] == [c['g'], c['s'], c['t']])
    phase, after, columns, count = r.number(1), r.take(32), r.number(2), r.number(2)
    need(phase <= 4 and columns == len(c['keys']) and count <= 32)
    units = [citizen(r, columns) for _ in range(count)]; receipt = r.take(32); r.end()
    need(receipt == digest(b'dfmcp-workforce-receipt/1', raw[:-32]))
    if phase == 2:
        need(len(units) == len(post['units']))
        for old, new in zip(post['units'], units):
            if old[0] in changed:
                need(not assigned or all(not b or new[3][i] for i, b in enumerate(c['details'][detail][3])))
                old[3] = new[3]
            need(old == new)
        need(after == hashlib.sha256(encode(post)).digest())
    else:
        need(not units and after == bytes(32))
    return phase


class Reference(unittest.TestCase):
    def setUp(self):
        self.before = bytes.fromhex((FIXTURES/'capture.hex').read_text())
        self.applied = bytes.fromhex((FIXTURES/'applied.hex').read_text())
    def test_cpp_vector_and_roundtrip(self):
        self.assertEqual(encode(capture(self.before)), self.before)
        self.assertEqual(effect(self.applied, self.before), 2)
    def test_every_effect_byte_corruption(self):
        for at in range(len(self.applied)):
            bad = bytearray(self.applied); bad[at] ^= 1
            with self.assertRaises(ValueError): effect(bytes(bad), self.before)
    def test_all_incomplete_prefixes(self):
        for size in range(len(self.applied)):
            with self.assertRaises(ValueError): effect(self.applied[:size], self.before)
        for size in range(len(self.before)):
            with self.assertRaises(ValueError): capture(self.before[:size])
    def test_all_membership_subsets(self):
        for subset in range(256):
            c = capture(self.before); c['units'] = [[i, i+100, 1, [0,0,0]] for i in range(8)]
            c['details'][0][4] = [i for i in range(8) if subset & 1 << i]
            for assigned in (0,1):
                if subset == (255 if assigned else 0):
                    with self.assertRaises(ValueError): expected(c,0,assigned)
                else:
                    after, changed = expected(c,0,assigned)
                    self.assertEqual(after['details'][0][4], list(range(8)) if assigned else [])
                    self.assertEqual(len(changed), 8-subset.bit_count() if assigned else subset.bit_count())
    def test_rehashed_lie_rejected(self):
        # The sample field is structurally valid, and its receipt is freshly hashed.
        # Changing a readback mask cannot substitute for the expected post-witness.
        raw = bytearray(self.applied); raw[-33] ^= 1
        raw[-32:] = digest(b'dfmcp-workforce-receipt/1', raw[:-32])
        with self.assertRaises(ValueError): effect(bytes(raw),self.before)
    def test_wrong_identity_cannot_reuse_receipt(self):
        for name,value in [('g',43),('site',8),('folder','other'),('s',1),('t',101)]:
            c = capture(self.before); c[name] = value
            with self.assertRaises(ValueError): effect(self.applied,encode(c))
    def test_rust_registered_source_and_limits(self):
        root = ROOT/'crates/dfmcp-adapter/src/workforce_control'
        mod = (root/'mod.rs').read_text(); rpc = (root/'rpc.rs').read_text()
        self.assertIn('pub mod rpc;',mod)
        self.assertIn('pub const MAX_PLAN: usize = 65_675;',mod)
        self.assertIn('full post-configuration witness mismatch',mod)
        self.assertIn('self.deadline = self.deadline.min(candidate)',rpc)
        self.assertNotIn('Command::',rpc)
        print(json.dumps({'evidence':'independent Python reference; Rust uncompiled/unexecuted',
            'source_sha256':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest()
                for p in sorted(root.rglob('*.rs'))},'membership_cases':512,
            'corruption_cases':len(self.applied),'incomplete_prefixes':len(self.applied)+len(self.before)},sort_keys=True))


if __name__ == '__main__': unittest.main(verbosity=2)
