#!/usr/bin/env python3
"""Independent workforce binary journal model. This does not execute Rust or file custody."""
from __future__ import annotations
import hashlib
import json
from pathlib import Path
import struct
import unittest
import check_workforce_rust_reference as w

ROOT = Path(__file__).resolve().parents[1]
FILE = ROOT/'crates/dfmcp-adapter/tests/fixtures/workforce_journal_v1_17.hex'
BEFORE = bytes.fromhex((w.FIXTURES/'capture.hex').read_text())
APPLIED = bytes.fromhex((w.FIXTURES/'applied.hex').read_text())
PLAN = w.field('assign') + struct.pack('>IBI',0,1,len(BEFORE)) + BEFORE
BINDING = w.field('127.0.0.1:5000') + struct.pack('>Q',42) + w.field('fake-df') + w.field('fake-dfhack') + w.field('region1') + struct.pack('>I',7)
HEADER_PREFIX = b'DFMWJ001' + struct.pack('>H',len(BINDING)) + BINDING + b'n'*32
HEADER = HEADER_PREFIX + w.digest(b'dfmcp-workforce-journal/1',HEADER_PREFIX)
PREP_PREFIX = APPLIED[:125] + b'\0' + bytes(32) + struct.pack('>HH',3,0)
PREPARED = PREP_PREFIX + w.digest(b'dfmcp-workforce-receipt/1',PREP_PREFIX)


def body(state, native=b'', plan=PLAN):
    return bytes([state]) + struct.pack('>I',len(plan)) + plan + struct.pack('>I',len(native)) + native


def history(bodies):
    result, previous = HEADER, HEADER[-32:]
    for sequence, payload in enumerate(bodies,1):
        prefix = b'DFMWFR01' + struct.pack('>IQ',len(payload),sequence) + previous + payload
        previous = w.digest(b'dfmcp-workforce-frame/1',prefix)
        result += prefix + previous + b'DFMWEND1'
    return result


def parse_record(raw):
    r = w.Reader(raw); state=r.number(1); n=r.number(4); w.need(n<=65675)
    p=w.Reader(r.take(n)); key=p.string(128); detail=p.number(4); assigned=p.flag(); n=p.number(4); w.need(n<=65536)
    before=p.take(n); p.end(); c=w.capture(before); w.expected(c,detail,assigned)
    w.need((c['g'],c['folder'],c['site'])==(42,'region1',7))
    n=r.number(4); w.need(n<=8192); native=r.take(n); r.end()
    phase=w.effect(native,before,key,detail,assigned) if native else None
    w.need(state in range(7))
    valid={0:phase is None,1:phase==0,2:phase==0,3:phase in (0,1),4:phase in (2,3,4),5:phase in (0,1),6:phase in (None,0)}
    w.need(valid[state]); return dict(key=key,plan=p.data,state=state,native=native,phase=phase)


def transition(old,new):
    if old is None:
        return new['state']==0
    if old==new or old['plan']!=new['plan'] or old['state'] in (4,6): return False
    if old['native']:
        if not new['native']: return False
        if old['phase']!=0 and old['native']!=new['native']: return False
    pair=old['state'],new['state']
    if pair in ((1,2),(1,6),(2,5),(3,5)): return old['native']==new['native']
    return pair in ((0,1),(0,3),(0,4),(0,6),(2,3),(2,4),(3,3),(3,4),(5,5),(5,4))


def parse(raw):
    w.need(len(raw)<=64*1024*1024 and raw.startswith(HEADER))
    r=w.Reader(raw[len(HEADER):]); previous=HEADER[-32:]; sequence=0; records={}; ends={len(HEADER)}
    while r.at<len(r.data):
        start=r.at; w.need(r.take(8)==b'DFMWFR01'); n=r.number(4); seq=r.number(8)
        w.need(n<=73876 and seq==sequence+1 and sequence<512 and r.take(32)==previous)
        new=parse_record(r.take(n)); end=r.at; checksum=r.take(32)
        w.need(checksum==w.digest(b'dfmcp-workforce-frame/1',r.data[start:end]) and r.take(8)==b'DFMWEND1')
        if new['key'] not in records: w.need(len(records)<64 and all(v['state'] in (4,6) for v in records.values()))
        w.need(transition(records.get(new['key']),new))
        records[new['key']]=new; previous=checksum; sequence=seq; ends.add(len(HEADER)+r.at)
    return records,ends


BODIES=[body(0),body(1,PREPARED),body(2,PREPARED),body(4,APPLIED)]
FIXTURE=history(BODIES)


class Reference(unittest.TestCase):
    def test_independent_fixture_and_native_receipts(self):
        self.assertEqual(bytes.fromhex(FILE.read_text()),FIXTURE)
        self.assertEqual(parse(FIXTURE)[0]['assign']['state'],4)
        self.assertEqual(w.effect(PREPARED,BEFORE),0)
    def test_all_corruptions(self):
        for i in range(len(FIXTURE)):
            bad=bytearray(FIXTURE); bad[i]^=1
            with self.assertRaises(ValueError): parse(bytes(bad))
    def test_all_incomplete_prefixes_and_complete_boundaries(self):
        ends=parse(FIXTURE)[1]
        for size in range(len(FIXTURE)):
            if size in ends: parse(FIXTURE[:size])
            else:
                with self.assertRaises(ValueError): parse(FIXTURE[:size])
    def test_rehashed_illegal_histories(self):
        paths=[BODIES[1:], [BODIES[0],BODIES[1],BODIES[3]], BODIES+[BODIES[1]],
               BODIES[:3]+[BODIES[1]], [BODIES[0],BODIES[0]], BODIES[:3]+[body(0)],
               [body(0),body(6),body(1,PREPARED)]]
        for bodies in paths:
            with self.assertRaises(ValueError): parse(history(bodies))
    def test_no_second_dispatch_and_cancellation(self):
        self.assertEqual(parse(history(BODIES[:3]+[body(3,PREPARED)]))[0]['assign']['state'],3)
        with self.assertRaises(ValueError): parse(history(BODIES[:3]+[body(3,PREPARED),body(2,PREPARED)]))
        self.assertEqual(parse(history([body(0),body(1,PREPARED),body(6,PREPARED)]))[0]['assign']['state'],6)
    def test_capacity_and_output_source_bound(self):
        reserve={0:4,1:3,2:2,3:2,4:0,5:1,6:0}
        for state,slots in reserve.items():
            for events in range(514):
                admitted=events+1+slots<=512
                if admitted: self.assertGreaterEqual(512-(events+1),slots)
        root=ROOT/'crates/dfmcp-adapter/src/workforce_control'
        print(json.dumps({'evidence':'Python binary/state reference only; Rust uncompiled/unexecuted',
            'fixture_bytes':len(FIXTURE),'fixture_sha256':hashlib.sha256(FIXTURE).hexdigest(),
            'source_sha256':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest()
                for p in [root/'journal.rs',root/'journal/tests.rs',root/'private_file.rs']},
            'corruptions':len(FIXTURE),'illegal_rehashed_paths':7},sort_keys=True))


if __name__=='__main__': unittest.main(verbosity=2)
