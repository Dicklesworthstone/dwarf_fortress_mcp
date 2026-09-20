#!/usr/bin/env python3
"""Independent watch-book framing/replay model. Not a Rust or filesystem test."""
from __future__ import annotations
import hashlib
import json
import struct
from pathlib import Path
from check_progress_watch_reference import model

ROOT = Path(__file__).resolve().parents[1]
H = lambda data: hashlib.sha256(data).digest()
U64 = lambda n: struct.pack('>Q', n)
TEXT = lambda s: struct.pack('>H', len(s)) + s
MAGIC, FRAME, END = b'DFMPWB01', b'DFMPWR01', b'DFMPWE01'

def archive_vectors():
    fixture = bytes.fromhex((ROOT/'crates/dfmcp-adapter/tests/fixtures/work_order_progress_v1_12.hex').read_text())
    site = struct.unpack_from('>I', fixture, 32)[0]
    n = struct.unpack_from('>H', fixture, 45)[0]
    folder = fixture[47:47+n]
    fortress = int.from_bytes(H(b'dfmcp-live-fortress-id-v1\0'+folder+b'\0'+struct.pack('>I', site))[:8], 'big') | 1
    aid = H(b'dfmcp-progress-archive-identity/1\0'+(1).to_bytes(16,'big')*2+U64(fortress))
    head = b'DFMWPA12'+U64(fortress)+aid
    previous = H(b'dfmcp-progress-archive-header/1\0'+head)
    out = head+previous
    records = {}
    for number, (tick, flags) in enumerate(((10,0),(11,1),(12,1)),1):
        o=bytearray(fixture)
        o[16:24]=U64(number);o[24:32]=U64(tick)
        o[68:72]=struct.pack('>i',3);o[76:80]=struct.pack('>I',flags)
        body=U64(1)+U64(7)+TEXT(b'df')+TEXT(b'dfhack')+b'\2'+struct.pack('>II',3,8)+struct.pack('>I',len(o))+o
        prefix=b'DFMWPR12'+struct.pack('>I',len(body))+U64(number)+previous
        digest=H(b'dfmcp-progress-archive-frame/1\0'+aid+prefix+body)
        out+=prefix+body+digest+b'DFMWPE12'
        records[number]={'digest':digest,'witness':H(o),'tick':tick,'truth':bool(flags&1)}
        previous=digest
    return fortress, aid, records, out


def ref(number, records):
    return U64(number)+records[number]['digest']

def definition(aid, key, records, origin=1):
    spec=bytes([len(key)])+key+struct.pack('>IBIQQB',3,1,0,30,1,2)
    digest=H(b'dfmcp-progress-watch-definition/1\0'+aid+spec+ref(origin,records)+U64(1)+records[origin]['witness'])
    return spec,digest

def framed(bid, previous, number, body):
    prefix=FRAME+struct.pack('>I',len(body))+U64(number)+previous
    digest=H(b'dfmcp-progress-watch-frame/1\0'+bid+prefix+body)
    return prefix+body+digest+END

def build(fortress,aid,events):
    bid=H(b'dfmcp-progress-watch-book-id/1\0'+aid+(1).to_bytes(16,'big')*2)
    header=MAGIC+U64(fortress)+aid+bid
    previous=H(b'dfmcp-progress-watch-header/1\0'+header);out=header+previous
    for number,body in enumerate(events,1):
        frame=framed(bid,previous,number,body);out+=frame;previous=frame[-40:-8]
    return out

class Rejected(ValueError):pass

def check(test):
    if not test:raise Rejected('invalid book')

def validate(raw,fortress,aid,records):
    check(112<=len(raw)<=128*1024 and raw[:8]==MAGIC)
    check(raw[8:16]==U64(fortress) and raw[16:48]==aid and raw[80:112]==H(b'dfmcp-progress-watch-header/1\0'+raw[:80]))
    bid=raw[48:80];previous=raw[80:112];offset=112;number=0;frontier=0;defs={};cancels={}
    while offset<len(raw):
        check(number<64 and len(raw)-offset>=92 and raw[offset:offset+8]==FRAME)
        size,n=struct.unpack_from('>IQ',raw,offset+8);check(size<=512 and n==number+1)
        end=offset+52+size+40;check(end<=len(raw))
        prefix=raw[offset:offset+52];body=raw[offset+52:end-40];digest=raw[end-40:end-8]
        check(prefix[20:]==previous and raw[end-8:end]==END and digest==H(b'dfmcp-progress-watch-frame/1\0'+bid+prefix+body))
        check(len(body)>=2 and 1<=body[1]<=64)
        tag,key=body[0],body[2:2+body[1]];check(len(key)==body[1] and all(chr(c).isascii() and (chr(c).isalnum() or c in b'-_.') for c in key))
        p=2+len(key)
        if tag==1:
            check(len(body)==p+26+72 and key not in defs and len(defs)<32)
            order,goal,threshold,deadline,cadence,stable=struct.unpack_from('>IBIQQB',body,p)
            check(order==3 and goal==1 and threshold==0 and deadline==30 and cadence==1 and stable==2)
            at=struct.unpack_from('>Q',body,p+26)[0];check(at in records)
            spec=body[1:p+26]
            expected=H(b'dfmcp-progress-watch-definition/1\0'+aid+spec+ref(at,records)+U64(1)+records[at]['witness'])
            check(body[p+26:p+66]==ref(at,records) and body[-32:]==expected)
            defs[key]=(at,expected)
        elif tag==2:
            check(len(body)==p+72 and key in defs and key not in cancels)
            at=struct.unpack_from('>Q',body,p+32)[0];check(at in records)
            check(body[p:p+32]==defs[key][1] and body[p+32:]==ref(at,records) and at>=defs[key][0])
            origin=defs[key][0]
            events=[(records[i]['tick']-records[origin]['tick'],records[i]['truth'],'normal') for i in range(origin+1,at+1)]
            check(model(events,deadline=30-records[origin]['tick'])[0]=='pending')
            cancels[key]=at
        else:raise Rejected('unknown event')
        check(at>=frontier);frontier=at;number=n;offset=end;previous=digest
    return defs,cancels

def reject(raw,*args):
    try:validate(raw,*args)
    except (Rejected,ValueError,struct.error,IndexError):return
    raise AssertionError('accepted invalid history')

def main():
    fortress,aid,records,archive=archive_vectors();spec,digest=definition(aid,b'key',records)
    register=b'\1'+spec+ref(1,records)+digest
    cancel=lambda number,key=b'key',dig=digest:b'\2'+bytes([len(key)])+key+dig+ref(number,records)
    raw=build(fortress,aid,[register]);validate(raw,fortress,aid,records)
    validate(build(fortress,aid,[register,cancel(2)]),fortress,aid,records)
    for i in range(len(raw)):
        bad=bytearray(raw);bad[i]^=1;reject(bad,fortress,aid,records)
    prefixes=0
    for size in range(len(raw)):
        if size==112:continue
        reject(raw[:size],fortress,aid,records);prefixes+=1
    illegal=[[register,cancel(3)],[register,register],[cancel(1)],
             [register,cancel(2),cancel(2)],[register,cancel(2,b'other')],
             [register,cancel(2,dig=bytes(32))], [register,b'\xff'+register[1:]]]
    for events in illegal:reject(build(fortress,aid,events),fortress,aid,records)
    reject(raw,fortress,bytes(32),records)
    # Produce exact executable-test vectors, with the native observation codec as
    # the leaf and independent SHA-256 for both framing formats and definitions.
    fixtures=ROOT/'crates/dfmcp-adapter/tests/fixtures'
    for name,value in [('progress_watch_book_v1',raw),('progress_watch_archive_v1',archive)]:
        path=fixtures/(name+'.hex')
        if path.exists():assert bytes.fromhex(path.read_text())==value
        else:path.write_text(value.hex()+'\n')
    files=['crates/dfmcp-adapter/src/work_order_progress/watch_book.rs',
           'crates/dfmcp-adapter/src/work_order_progress/watch_file.rs',
           'crates/dfmcp-adapter/src/work_order_progress/watch_book_tests.rs',
           'scripts/check_progress_watch_book_reference.py']
    print(json.dumps({'schema':'dfmcp.progress-watch-book-reference/1','status':'passed_reference_only',
          'book_bytes':len(raw),'archive_bytes':len(archive),'byte_corruptions_rejected':len(raw),
          'incomplete_prefixes_rejected':prefixes,'rehashed_illegal_histories_rejected':len(illegal),
          'foreign_archive_rejected':True,'rust_compiled':False,'rust_tests_executed':False,
          'filesystem_or_mcp_or_native_executed':False,
          'scope':'Independent fixed-plan Python book/archive framing and replay model, not Rust execution',
          'source_sha256':{p:hashlib.sha256((ROOT/p).read_bytes()).hexdigest() for p in files}},indent=2,sort_keys=True))

if __name__=='__main__':main()
