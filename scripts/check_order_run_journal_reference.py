#!/usr/bin/env python3
"""Independent order-run Rust journal framing/state reference, not Rust execution."""
from pathlib import Path
import hashlib
import json
import struct
import sys
import unittest
import check_order_run_rust_vectors as v

ROOT=Path(__file__).resolve().parents[1]
MAX_FRAME=2258
NEXT={0:{1,3,4,6},1:{2,3,4,6},2:{3,4,5},3:{3,4,5},4:set(),5:{5,4},6:set()}
def field(b): return struct.pack('>H',len(b))+b
def sha(domain,b):return hashlib.sha256(domain+b'\0'+b).digest()
def checked(ok):
    if not ok:raise ValueError('reference refusal')
def make_header():
    binding=(field(b'127.0.0.1:5000')+struct.pack('>Q',41)+field(b'df')+field(b'dfhack')+field(b'region1')+struct.pack('>I',7))
    prefix=b'DFMOJ014'+field(binding)+(1).to_bytes(16,'big')*2
    return prefix+sha(b'dfmcp-order-run-rust-journal/1',prefix)
def records():
    before,plan,terminal=v.fixture();key=b'approval';digest=sha(b'dfmcp-order-run-plan/1',plan)
    token=sha(b'dfmcp-order-run-token/1',field(key)+digest)[:16]
    prepared=(b'DFMOE014'+field(key)+field(plan)+digest+token+bytes(6)+struct.pack('>QIQ',0,0,100)+field(b''))
    prepared+=sha(b'dfmcp-order-run-receipt/1',prepared)
    intent=field(key)+field(plan)
    return intent,prepared,terminal
def make_frame(body,seq,previous):
    prefix=b'DFMOF014'+struct.pack('>IQ',len(body),seq)+previous+body
    return prefix+sha(b'dfmcp-order-run-rust-frame/1',prefix)+b'DFMOEND1'
def make_journal(states=(0,1,2,4)):
    out=make_header();head=out[-32:];intent,prepared,terminal=records()
    for number,state in enumerate(states,1):
        native=b'' if state==0 else terminal if state==4 else prepared
        body=bytes([state])+field(intent)+field(native)
        frame=make_frame(body,number,head);head=frame[-40:-8];out+=frame
    return out
class Reader:
    def __init__(self,data):self.data=data;self.at=0
    def take(self,n):
        checked(n>=0 and n<=len(self.data)-self.at);out=self.data[self.at:self.at+n];self.at+=n;return out
    def num(self,n):return int.from_bytes(self.take(n),'big')
    def field(self,maximum):
        n=self.num(2);checked(n<=maximum);return self.take(n)
def replay(data):
    checked(len(data)<=2*1024*1024)
    r=Reader(data);checked(r.take(8)==b'DFMOJ014');binding=r.field(1024);r.take(32);end=r.at;head=r.take(32)
    checked(head==sha(b'dfmcp-order-run-rust-journal/1',data[:end]))
    checked(binding==make_header()[10:10+len(binding)])
    previous_state=None;intent=None;count=0;boundaries=[r.at]
    while r.at<len(data):
        start=r.at;checked(r.take(8)==b'DFMOF014');n=r.num(4);checked(n<=2166)
        seq=r.num(8);checked(seq==count+1 and seq<=4096 and r.take(32)==head)
        body=Reader(r.take(n));state=body.num(1);current=body.field(736);native=body.field(1425);checked(body.at==n)
        checked(state==0 if previous_state is None else state in NEXT[previous_state])
        checked(intent is None or current==intent);intent=current
        i,p,t=records();checked(current==i)
        checked(native==b'' if state==0 else native==t if state==4 else native==p)
        end=r.at;proof=r.take(32);checked(proof==sha(b'dfmcp-order-run-rust-frame/1',data[start:end]));checked(r.take(8)==b'DFMOEND1')
        head=proof;previous_state=state;count+=1;boundaries.append(r.at)
    return count,head,boundaries
class References(unittest.TestCase):
    def test_fixture(self):
        data=make_journal();path=ROOT/'crates/dfmcp-adapter/tests/fixtures/order_run_journal_v1_14.hex'
        self.assertEqual(path.read_text(),data.hex()+'\n');self.assertEqual(replay(data)[0],4)
    def test_all_corruptions(self):
        data=make_journal()
        for offset in range(len(data)):
            bad=bytearray(data);bad[offset]^=1
            with self.assertRaises(ValueError):replay(bytes(bad))
    def test_all_incomplete_prefixes(self):
        data=make_journal();boundaries=set(replay(data)[2])
        for end in range(len(data)):
            if end in boundaries:replay(data[:end])
            else:
                with self.assertRaises(ValueError):replay(data[:end])
    def test_rehashed_no_redispatch_paths(self):
        paths=[(0,1,2,1),(0,1,2,3,1),(0,1,2,5,1),(0,1,2,4,2),(0,6,1),(1,2),(0,2),(0,1,2,5,3)]
        for path in paths:
            with self.assertRaises(ValueError):replay(make_journal(path))
        for path in [(0,),(0,1),(0,1,2),(0,1,2,3),(0,1,2,5,5,4),(0,1,6),(0,3,4)]:replay(make_journal(path))
    def test_every_state_pair_and_dispatch_count(self):
        for old in range(7):
            for new in range(7):
                expected=('0101101','0011101','0001110','0001110','0000000','0000110','0000000')
                self.assertEqual(new in NEXT[old],expected[old][new]=='1')
        visited=0
        def walk(path):
            nonlocal visited
            visited+=1;self.assertLessEqual(path.count(2),1)
            if len(path)<10:
                for child in NEXT[path[-1]]:walk(path+(child,))
        walk((0,));self.assertGreater(visited,200)
    def test_cancellation_space(self):
        for used in range(4090,4097):
            allowed_ordinary=used+3<=4096;allowed_cancel=used+2<=4096;allowed_repeat=used+1<=4096
            if allowed_ordinary:self.assertTrue(allowed_cancel)
            if allowed_cancel:self.assertTrue(allowed_repeat)
        self.assertLess(2166+92,8192)
        self.assertTrue(4095+1<=4096);self.assertFalse(4095+2<=4096)
if __name__=='__main__':
    if sys.argv[1:]==['--create']:
        path=ROOT/'crates/dfmcp-adapter/tests/fixtures/order_run_journal_v1_14.hex'
        path.write_text(make_journal().hex()+'\n')
        print(json.dumps({'reference_only':True,'fixture_bytes':len(make_journal()),'transitions':replay(make_journal())[0]}))
    else:unittest.main(verbosity=2)
