#!/usr/bin/env python3
"""Independent canonical fixture reference, NOT execution of Rust or DFHack."""
from pathlib import Path
import hashlib
import json
import struct
import sys

ROOT = Path(__file__).resolve().parents[1]
def sha(domain, data):
    return hashlib.sha256(domain + b'\0' + data).digest()
def field(data):
    return struct.pack('>H', len(data)) + data
def capture(folder=b'region1', seq=3, tick=100, paused=1, status=0, remaining=5):
    return (b'DFMOR014' + struct.pack('>QQQI',41,seq,tick,7) + field(folder)
            + struct.pack('>BIIBBiiI',paused,9,10,1,1,5,remaining,status))
def fixture(key=b'approval', folder=b'region1'):
    before=capture(folder)
    plan=b'DFMOP014'+struct.pack('>IIBIII',20,1000,1,0,2,2)+field(before)
    digest=sha(b'dfmcp-order-run-plan/1',plan)
    token=sha(b'dfmcp-order-run-token/1',field(key)+digest)[:16]
    sample=capture(folder,4,105,0,1,3)
    record=(b'DFMOE014'+field(key)+field(plan)+digest+token+bytes([3,3,1,1,1,1])
            +struct.pack('>QIQ',105,2,105)+field(sample))
    return before,plan,record+sha(b'dfmcp-order-run-receipt/1',record)
def main():
    before,plan,record=fixture()
    path=ROOT/'crates/dfmcp-adapter/tests/fixtures/order_run_predicate_v1_14.hex'
    encoded=record.hex()+'\n'
    if sys.argv[1:] == ['--create']:
        with path.open('x') as out:
            out.write(encoded)
    assert path.read_text()==encoded
    maximal=fixture(b'k'*128,b'x'*512)
    assert [len(v) for v in maximal]==[573,604,1425]
    assert len(field(b'k'*128)+field(maximal[1]))==736
    identity=hashlib.sha256(b'dfmcp-live-fortress-id-v1\0region1\0'+struct.pack('>I',7)).digest()
    print(json.dumps({'scope':'independent canonical reference only','fixture_bytes':len(record),
          'plan_digest':sha(b'dfmcp-order-run-plan/1',plan).hex(),
          'receipt_digest':record[-32:].hex(),'fortress_id':int.from_bytes(identity[:8],'big')|1,
          'max_capture_plan_record_bytes':[len(v) for v in maximal]},sort_keys=True))
if __name__=='__main__':
    main()
