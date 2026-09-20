#!/usr/bin/env python3
"""Actual Python codec, POSIX journal and joined loopback RPC-double regressions.

This is not a live DFHack, SDK/protobuf ABI, Rust/MCP, or power-loss campaign.
All temporary directories and binary test products are retained.
"""
from __future__ import annotations

import copy
from contextlib import redirect_stdout
import hashlib
import io
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import workforce_client as c
import workforce_wire as w
from test_workforce_native import reference_vectors

ROOT=Path(__file__).resolve().parents[1]
V={k:bytes.fromhex(v) for k,v in reference_vectors().items()}
PLAN=w.make_plan('assign',0,True,V['capture'])
BINDING={'endpoint':'127.0.0.1:5000','generation':42,'folder':'region1','site':7,
         'df_version':'test-df','dfhack_version':'test-dfhack'}
ENV={'DFMCP_ALLOW_UNADMITTED_WORKFORCE_V1_17':'1','DFMCP_WORKFORCE_ALLOW_LABOR':'1','DFMCP_WORKFORCE_TOKEN':'t'*32}


def timeout():return time.monotonic()+20

def path():
    parent=Path(tempfile.mkdtemp(prefix='dfmcp-workforce-client-'));os.chmod(parent,0o700)
    return parent/'workforce.jsonl'


def rehash(data):return data[:-32]+hashlib.sha256(b'dfmcp-workforce-receipt/1\0'+data[:-32]).digest()


def native_record(phase):
    if phase=='applied':return V['applied']
    raw=bytearray(V['prepared']);raw[8+2+len('assign')+32+16+5+32+24]=w.PHASES.index(phase)
    return rehash(bytes(raw))


class Model:
    """Injected source model; real client tests use TCP separately below."""
    def __init__(self, endpoint=BINDING['endpoint']):
        self.endpoint=endpoint;self.manifest={k:BINDING[k] for k in ('generation','df_version','dfhack_version')}
        self.calls=[];self.current=V['capture'];self.record=None;self.lost=None;self.hook=None
    def call(self,operation,plan=None,unit_ids=None):
        self.calls.append(operation)
        if self.hook:self.hook(operation)
        if operation=='ObserveWorkforce':return {'capture_hex':self.current.hex(),'manifest':self.manifest}
        if operation=='PrepareAssignment':self.record=native_record('prepared')
        if operation=='CommitAssignment':self.record=native_record('applied')
        if operation=='CancelAssignment' and self.record==native_record('prepared'):self.record=native_record('cancelled')
        if operation==self.lost:raise OSError('lost native reply')
        result={'manifest':self.manifest}
        if self.record is not None:result['effect_hex']=self.record.hex()
        return result


def proto(fields):
    def n(v):
        out=b''
        while v>127:out+=bytes([v%128+128]);v//=128
        return out+bytes([v])
    out=b''
    for tag,value in fields.items():
        out+=n(tag*8+2)+n(len(value))+value if type(value)is bytes else n(tag*8)+n(value)
    return out


def request(raw):
    # Independent decoder preserves unpacked repeated IDs, unlike response fields.
    cursor=0;values={}
    def n():
        nonlocal cursor
        value=0;shift=0
        while True:
            b=raw[cursor];cursor+=1;value+=(b&127)<<shift
            if b<128:return value
            shift+=7
            assert shift<70
    while cursor<len(raw):
        tag=n();field,kind=tag>>3,tag&7
        if kind==0:value=n()
        else:
            assert kind==2;size=n();value=raw[cursor:cursor+size];cursor+=size
        if field==11:values.setdefault(field,[]).append(value)
        else:assert field not in values;values[field]=value
    return values


class NativeDouble:
    def __init__(self,connections=1,lose_commit=False,bad_nonce=False,alias=False,notifications=0,delay=0):
        self.server=socket.socket();self.server.bind(('127.0.0.1',0));self.server.listen(2);self.server.settimeout(3)
        host,port=self.server.getsockname();self.endpoint=f'{host}:{port}'
        self.connections=connections;self.lose_commit=lose_commit;self.bad_nonce=bad_nonce;self.alias=alias
        self.notifications=notifications;self.delay=delay;self.error=None;self.calls=[];self.effects=0;self.record=None
        self.worker=threading.Thread(target=self.run,daemon=False)
    @staticmethod
    def read(s,n):
        out=b''
        while len(out)<n:
            part=s.recv(n-len(out))
            if not part:raise EOFError()
            out+=part
        return out
    def reply(self,s,fields):
        raw=proto(fields);raw=struct.pack('<h2xi',-1,len(raw))+raw
        for i in range(0,len(raw),13):s.sendall(raw[i:i+13])
    def run(self):
        try:
            for _ in range(self.connections):
                with self.server.accept()[0] as s:
                    s.settimeout(3)
                    try:
                        assert self.read(s,12)==b'DFHack?\n\1\0\0\0'
                        if self.delay:time.sleep(self.delay)
                        s.sendall(b'DFHack!\n\1\0\0\0');methods={}
                        while True:
                            method,size=struct.unpack('<h2xi',self.read(s,8));assert 0<=size<=2048
                            req=request(self.read(s,size))
                            if method==0:
                                assert req[2]==b'dfmcp.workforce.v1_17.Request' and req[3]==b'dfmcp.workforce.v1_17.Reply'
                                assert req[4]==b'dfmcp_workforce_v1_17'
                                name=req[1].decode();assert name in w.METHODS;bound=2 if self.alias else len(methods)+2;methods[bound]=name
                                for _ in range(self.notifications):s.sendall(struct.pack('<h2xi',-3,0))
                                self.reply(s,{1:bound});continue
                            name=methods[method];self.calls.append(name)
                            assert req[1]==b't'*32 and req[3]==1 and req[4]==17
                            extra={'Handshake':set(),'ObserveWorkforce':{11},'PrepareAssignment':{5,6,7,8,9,11},
                                   'CommitAssignment':{5,9,10},'QueryAssignment':{5,9},'CancelAssignment':{5,9,10}}[name]
                            assert set(req)=={1,2,3,4}|extra
                            if 11 in req:assert req[11]==[2,5]
                            if 5 in req:assert req[5]==b'assign' and req[9].hex()==PLAN['plan_digest']
                            if 10 in req:assert req[10].hex()==PLAN['prepare_token']
                            result={1:1,2:0,3:b'x'*32 if self.bad_nonce else req[2],4:1,5:17,6:42,
                                    7:b'test-df',8:b'test-dfhack',11:0,12:int(self.record is not None)}
                            if name=='ObserveWorkforce':result[9]=V['capture']
                            elif name=='PrepareAssignment':
                                assert req[6]==0 and req[7]==1 and req[8]==hashlib.sha256(V['capture']).digest()
                                self.record=V['prepared'];result[10]=self.record;result[12]=1
                            elif name=='CommitAssignment':
                                self.effects+=1;self.record=V['applied'];result[10]=self.record;result[12]=1
                                if self.lose_commit:s.sendall(struct.pack('<h2xi',-1,1000));break
                            elif name=='CancelAssignment':self.record=native_record('cancelled');result[10]=self.record;result[12]=1
                            elif name=='QueryAssignment' and self.record is not None:result[10]=self.record
                            self.reply(s,result)
                    except (EOFError,BrokenPipeError,ConnectionResetError):pass
        except BaseException as e:self.error=e
        finally:self.server.close()
    def __enter__(self):self.worker.start();return self
    def __exit__(self,*_):
        self.worker.join(5)
        if self.worker.is_alive():self.server.close();self.worker.join(4)
        if self.worker.is_alive():raise AssertionError('native double did not drain')
        if self.error:raise self.error


class CodecTests(unittest.TestCase):
    def test_cpp_vectors_and_independent_fixture(self):
        run=subprocess.run([sys.executable,str(ROOT/'scripts/test_workforce_native.py')],capture_output=True,text=True,timeout=90,check=True)
        self.assertIn('8497 assertions',run.stdout)
        for name,raw in V.items():self.assertEqual((ROOT/f'tests/native/workforce/vectors/{name}.hex').read_text().strip(),raw.hex())
        self.assertEqual(w.encode_capture(w.capture(V['capture'])),V['capture'])
        self.assertEqual(w.effect(V['prepared'],PLAN)['phase'],'prepared')
        applied=w.effect(V['applied'],PLAN);self.assertEqual(applied['changed_ids'],[2]);self.assertEqual(applied['phase'],'applied')
        self.assertFalse(applied['job_completion_proven'])
    def test_each_effect_corruption_and_torn_prefix(self):
        for name in ('prepared','applied'):
            raw=V[name]
            for i in range(len(raw)):
                bad=bytearray(raw);bad[i]^=1
                with self.assertRaises((ValueError,TypeError)):w.effect(bytes(bad),PLAN)
                with self.assertRaises(ValueError):w.effect(raw[:i],PLAN)
    def test_rehashed_invalid_readback_and_flags(self):
        raw=V['applied'];phase=8+2+6+32+16+5+32+24
        # Remove an enabled required bit, alter unchanged unit, claim another historical unit, or forge hash.
        for offset in (phase,phase+1,phase+37+4,phase+37+9,phase+37+12+9):
            bad=bytearray(raw);bad[offset]^=1
            with self.assertRaises(ValueError):w.effect(rehash(bytes(bad)),PLAN)
    def test_unknown_is_not_post_state(self):
        for phase in ('unknown','refused','cancelled'):
            value=w.effect(native_record(phase),PLAN);self.assertEqual(value['phase'],phase);self.assertIsNone(value['after_witness'])
        with self.assertRaises(ValueError):w.make_plan('x',0,True,w.encode_capture({**w.capture(V['capture']),'automatic':False}))
    def test_plan_bounds_and_all_eight_unit_subsets(self):
        for mask in range(256):
            before=w.capture(V['capture']);before['units']=[{'id':i,'historical_id':100+i,'eligible':True,'labors':[bool(mask&(1<<i)),0,0]} for i in range(8)]
            before['details'][0]['members']=[i for i in range(8) if mask&(1<<i)]
            raw=w.encode_capture(before)
            for assigned in (False,True):
                if mask==(255 if assigned else 0):
                    with self.assertRaises(ValueError):w.make_plan('x',0,assigned,raw)
                else:
                    plan=w.make_plan('x',0,assigned,raw);self.assertEqual(w.validate_plan(plan),plan)
        for detail in (-1,64,True):
            with self.assertRaises(ValueError):w.make_plan('x',detail,True,V['capture'])
    def test_protobuf_negative_envelopes(self):
        for data in (b'\x08\x01\x08\x01',b'\x08\x80\x00',b'\x08'+b'\xff'*10,b'\x1a\x05x',b'\x00'):
            with self.assertRaises(ValueError):w.decode(data)
        for endpoint in ('localhost:5000','192.0.2.1:5000','127.0.0.1:05000','127.0.0.1:0'):
            with self.assertRaises(ValueError):w.address(endpoint)


class JournalTests(unittest.TestCase):
    def setUp(self):
        self.env=patch.dict(os.environ,ENV,clear=True);self.env.start();self.addCleanup(self.env.stop)
        self.path=path();self.model=Model()
    def prepared(self):
        with c.Journal(self.path,timeout(),True,BINDING) as j:return c.prepare(j,self.model,PLAN)
    def test_lifecycle_and_offline_reopen(self):
        self.prepared()
        with c.Journal(self.path,timeout(),True) as j:
            e=c.commit(j,self.model,'assign',PLAN['plan_digest']);self.assertEqual(e['state'],'terminal')
        with patch.dict(os.environ,{},clear=True),c.Journal(self.path,timeout()) as j:
            result=c.inspect(j,'assign');self.assertEqual(result['effect']['native_evidence']['phase'],'applied');self.assertFalse(result['native_contacted'])
        self.assertEqual(self.model.calls.count('CommitAssignment'),1)
    def test_sync_order_and_review_confirmation(self):
        self.prepared()
        with c.Journal(self.path,timeout(),True) as j:
            with self.assertRaises(ValueError):c.commit(j,self.model,'assign','0'*64)
            self.assertNotIn('CommitAssignment',self.model.calls)
            def check(op):
                if op=='CommitAssignment':
                    values=[c.decoded(x)[0] for x in self.path.read_bytes().splitlines(keepends=True)]
                    self.assertEqual(values[-1]['entry']['state'],'dispatch_started')
            self.model.hook=check;c.commit(j,self.model,'assign',PLAN['plan_digest'])
    def test_lost_prepare_reply_recovers_without_reprepare(self):
        self.model.lost='PrepareAssignment'
        with c.Journal(self.path,timeout(),True,BINDING) as j:
            with self.assertRaises(OSError):c.prepare(j,self.model,PLAN)
        self.model.lost=None
        with c.Journal(self.path,timeout(),True) as j:
            e,absent=c.reconcile(j,self.model,'assign');self.assertFalse(absent);self.assertEqual(e['state'],'prepared')
        self.assertEqual(self.model.calls.count('PrepareAssignment'),1)
    def test_lost_commit_recovery_never_redispatches(self):
        self.prepared();self.model.lost='CommitAssignment'
        with c.Journal(self.path,timeout(),True) as j:
            with self.assertRaises(OSError):c.commit(j,self.model,'assign',PLAN['plan_digest'])
        self.model.lost=None
        with c.Journal(self.path,timeout(),True) as j:
            with self.assertRaises(ValueError):c.commit(j,self.model,'assign',PLAN['plan_digest'])
            e,_=c.reconcile(j,self.model,'assign');self.assertEqual(e['state'],'terminal')
        self.assertEqual(self.model.calls.count('CommitAssignment'),1)
    def test_crash_before_request_can_retire_native_preparation(self):
        e=self.prepared()
        with c.Journal(self.path,timeout(),True) as j:e['state']='dispatch_started';j.append(e)
        with c.Journal(self.path,timeout(),True) as j:
            e,_=c.reconcile(j,self.model,'assign');self.assertEqual(e['state'],'tracking')
            with self.assertRaises(ValueError):c.commit(j,self.model,'assign',PLAN['plan_digest'])
            e=c.cancel(j,'assign',self.model);self.assertEqual(e['state'],'terminal');self.assertEqual(w.effect(bytes.fromhex(e['effect_hex']),PLAN)['phase'],'cancelled')
        self.assertNotIn('CommitAssignment',self.model.calls)
    def test_unknown_and_absence_stay_unresolved(self):
        e=self.prepared()
        with c.Journal(self.path,timeout(),True) as j:
            e['state']='dispatch_started';j.append(e);self.model.record=None
            e,absent=c.reconcile(j,self.model,'assign');self.assertTrue(absent);self.assertEqual(e['state'],'dispatch_started')
            self.model.record=native_record('unknown');e,_=c.reconcile(j,self.model,'assign');self.assertEqual(e['state'],'tracking')
            original=j.head;c.reconcile(j,self.model,'assign');self.assertEqual(j.head,original)
            self.model.record=V['applied']
            with self.assertRaises(ValueError):c.reconcile(j,self.model,'assign')
            another=w.make_plan('other',0,True,V['capture'])
            with self.assertRaises(ValueError):c.prepare(j,self.model,another)
    def test_local_cancel_and_read_only_authority(self):
        self.prepared();calls=list(self.model.calls)
        with c.Journal(self.path,timeout(),True) as j:
            e=c.cancel(j,'assign');self.assertEqual(e['state'],'cancelled_before_dispatch')
        self.assertEqual(self.model.calls,calls)
        with c.Journal(self.path,timeout()) as j:
            with self.assertRaises(ValueError):j.append({'plan':w.make_plan('next',0,True,V['capture']),'state':'intent','effect_hex':None})
    def test_revocation_and_stale_capture_before_dispatch(self):
        self.prepared()
        with c.Journal(self.path,timeout(),True) as j:
            with patch.dict(os.environ,{'DFMCP_WORKFORCE_ALLOW_LABOR':'0'}):
                with self.assertRaises(ValueError):c.commit(j,self.model,'assign',PLAN['plan_digest'])
            before=w.capture(V['capture']);before['folder']='other';self.model.current=w.encode_capture(before)
            with self.assertRaises(ValueError):c.commit(j,self.model,'assign',PLAN['plan_digest'])
            self.assertEqual(j.get('assign')['state'],'prepared');self.assertNotIn('CommitAssignment',self.model.calls)
    def test_sync_failures_prevent_prepare_and_commit(self):
        for directory in (False,True):
            file=path();original=os.fsync
            def fail(fd):
                if bool(stat.S_ISDIR(os.fstat(fd).st_mode))==directory:raise OSError('injected sync')
                return original(fd)
            import stat
            with patch('os.fsync',side_effect=fail),self.assertRaises(OSError):
                with c.Journal(file,timeout(),True,BINDING) as j:c.prepare(j,self.model,PLAN)
            self.assertEqual(self.model.calls,[])
        self.prepared()
        with c.Journal(self.path,timeout(),True) as j:
            with patch('os.fsync',side_effect=OSError('dispatch sync')),self.assertRaises(OSError):c.commit(j,self.model,'assign',PLAN['plan_digest'])
            self.assertTrue(j.fenced)
        self.assertNotIn('CommitAssignment',self.model.calls)
        with c.Journal(self.path,timeout(),True) as j:
            self.assertEqual(j.get('assign')['state'],'dispatch_started')
            with self.assertRaises(ValueError):c.commit(j,self.model,'assign',PLAN['plan_digest'])
    def test_uncertain_terminal_sync_recovers_exact_receipt(self):
        self.prepared();sync=os.fsync;count=0
        with c.Journal(self.path,timeout(),True) as j:
            def fail_second(fd):
                nonlocal count
                count+=1
                if count==2:raise OSError('terminal sync')
                return sync(fd)
            with patch('os.fsync',side_effect=fail_second),self.assertRaises(OSError):c.commit(j,self.model,'assign',PLAN['plan_digest'])
            self.assertTrue(j.fenced)
        with c.Journal(self.path,timeout(),True) as j:self.assertEqual(j.get('assign')['state'],'terminal')
        self.assertEqual(self.model.calls.count('CommitAssignment'),1)
    def test_partial_write_and_unchanged_torn_tail(self):
        self.prepared();write=os.write;count=0
        with c.Journal(self.path,timeout(),True) as j:
            def partial(fd,data):
                nonlocal count
                count+=1
                if count>1:raise OSError('write stopped')
                return write(fd,data[:11])
            with patch('os.write',side_effect=partial),self.assertRaises(OSError):c.commit(j,self.model,'assign',PLAN['plan_digest'])
        before=self.path.read_bytes()
        with self.assertRaises(ValueError):
            with c.Journal(self.path,timeout(),True):pass
        self.assertEqual(before,self.path.read_bytes());self.assertNotIn('CommitAssignment',self.model.calls)
    def test_custody_modes_links_replacement_and_lock(self):
        self.prepared()
        with c.Journal(self.path,timeout(),True) as j:
            with self.assertRaises(OSError):
                with c.Journal(self.path,timeout(),True):pass
            self.path.rename(self.path.with_suffix('.old'));self.path.write_bytes(j.raw);os.chmod(self.path,0o600)
            with self.assertRaises(ValueError):j.verify()
        os.chmod(self.path,0o644)
        with self.assertRaises(ValueError):
            with c.Journal(self.path,timeout()):pass
        os.chmod(self.path,0o600);os.link(self.path,self.path.with_suffix('.link'))
        with self.assertRaises(ValueError):
            with c.Journal(self.path,timeout()):pass
        fifo=path();os.mkfifo(fifo,0o600)
        with self.assertRaises(ValueError):
            with c.Journal(fifo,timeout()):pass
        symbolic=path();symbolic.symlink_to(self.path)
        with self.assertRaises(OSError):
            with c.Journal(symbolic,timeout()):pass
    def test_same_length_corruption_and_rehashed_illegal_history(self):
        self.prepared()
        with c.Journal(self.path,timeout(),True) as j:
            raw=bytearray(j.raw);raw[-5]^=1;self.path.write_bytes(raw)
            with self.assertRaises(ValueError):j.verify()
        file=path();self.path=file;self.prepared()
        lines=file.read_bytes().splitlines(keepends=True);last,_=c.decoded(lines[-1]);last['entry']['state']='terminal';lines[-1]=c.wrapped(last)[0];file.write_bytes(b''.join(lines))
        with self.assertRaises(ValueError):
            with c.Journal(file,timeout()):pass
    def test_state_pairs_never_reopen_dispatch(self):
        prepared={'plan':PLAN,'state':'prepared','effect_hex':V['prepared'].hex()}
        valid=0
        for oldstate in c.STATES:
            for newstate in c.STATES:
                old={**prepared,'state':oldstate};new={**prepared,'state':newstate}
                if newstate=='terminal':new['effect_hex']=V['applied'].hex()
                if newstate in ('intent','cancelled_before_dispatch'):new['effect_hex']=None
                try:c.transition(old,new,BINDING)
                except ValueError:continue
                valid+=1
                if newstate=='dispatch_started':self.assertEqual(oldstate,'prepared')
                self.assertNotIn(oldstate,('terminal','cancelled_before_dispatch'))
        self.assertGreater(valid,0)
    def test_capacity_preserves_terminal_evidence(self):
        e=self.prepared()
        with c.Journal(self.path,timeout(),True) as j:
            e['state']='dispatch_started';j.append(e)
            with patch.object(c,'MAX_EVENTS',j.events+1):
                pending={**e,'state':'tracking'}
                with self.assertRaises(ValueError):j.append(pending)
                final={**e,'state':'terminal','effect_hex':V['applied'].hex()};j.append(final)
                self.assertEqual(j.get('assign')['state'],'terminal')
    def test_offline_discovery_cursor_and_deadline(self):
        self.prepared()
        with c.Journal(self.path,timeout()) as j:
            page=c.inspect(j);self.assertEqual(page['unsettled_records'],1)
            with self.assertRaises(ValueError):c.inspect(j,after='assign',head='0'*64)
            j.deadline=time.monotonic()-1
            with self.assertRaises(ValueError):j.get('assign')
        with patch.dict(os.environ,{},clear=True),patch.object(w,'Client',side_effect=AssertionError('network in offline')):
            output=io.StringIO()
            with redirect_stdout(output):self.assertEqual(c.main(['inspect','--journal',str(self.path),'--key','assign']),0)
            self.assertFalse(json.loads(output.getvalue())['result']['native_contacted'])

    def test_all_record_pages_and_head_changes(self):
        with c.Journal(self.path,timeout(),True,BINDING) as j:
            for number in range(64):
                plan=w.make_plan(f'worker-{number:02}',0,True,V['capture'])
                j.append({'plan':plan,'state':'intent','effect_hex':None})
                j.append({'plan':plan,'state':'cancelled_before_dispatch','effect_hex':None})
            for limit in range(1,9):
                seen=[];after=None
                while True:
                    page=c.inspect(j,limit=limit,after=after,head=j.head if after else None)
                    seen.extend(e['key'] for e in page['records']);after=page['next_after']
                    if after is None:break
                self.assertEqual(seen,sorted(j.entries))
            with self.assertRaises(ValueError):
                j.append({'plan':w.make_plan('overflow',0,True,V['capture']),'state':'intent','effect_hex':None})

    def test_byte_reservation_before_prepare_and_maximal_projection(self):
        before=w.capture(V['capture']);before['folder']='\1'*512
        before['labor_keys']=['\1'*61+f'{i:03}' for i in range(128)]
        before['details']=[{'name':'\1'*256,'flags':2,'selected_only':True,'labors':[1]*128,'members':[]}]
        before['units']=[{'id':i,'historical_id':2**31-1-i,'eligible':True,'labors':[0]*128} for i in range(32)]
        raw=w.encode_capture(before);plan=w.make_plan('x'*128,0,True,raw)
        reserved=c.reserve_response(plan)
        after,changed=w.expected(before,0,True)
        for u in after['units']:u['labors']=[1]*128
        body=(b'DFMWE017'+w.field(plan['key'].encode())+bytes.fromhex(plan['plan_digest'])+bytes.fromhex(plan['prepare_token'])
            +struct.pack('>IB',0,1)+hashlib.sha256(raw).digest()+struct.pack('>QQQB',42,0,100,2)
            +hashlib.sha256(w.encode_capture(after)).digest()+struct.pack('>HH',128,32)
            +b''.join(w.encode_unit(u) for u in after['units']))
        record=body+hashlib.sha256(b'dfmcp-workforce-receipt/1\0'+body).digest()
        entry={'plan':plan,'state':'terminal','effect_hex':record.hex()}
        actual=len(c.canonical({'ok':True,'profile':'workforce/1.17','runtime_admitted':False,'result':{'effect':c.project(entry)}}))
        self.assertLess(actual,reserved);self.assertLessEqual(reserved,c.OUTPUT_LIMIT)
        with c.Journal(self.path,timeout(),True,BINDING) as j:
            with patch.object(c,'OUTPUT_LIMIT',1),self.assertRaises(ValueError):c.prepare(j,self.model,PLAN)
            self.assertEqual(j.events,0);self.assertEqual(self.model.calls,[])

    def test_prepare_publication_sync_failure_no_native_allocation(self):
        with c.Journal(self.path,timeout(),True,BINDING) as j:
            with patch('os.fsync',side_effect=OSError('intent sync')),self.assertRaises(OSError):c.prepare(j,self.model,PLAN)
            self.assertTrue(j.fenced);self.assertEqual(self.model.calls,[])
        with c.Journal(self.path,timeout(),True) as j:self.assertEqual(j.get('assign')['state'],'intent')

    def test_wrong_fortress_source_and_postmarker_revocation(self):
        self.prepared()
        with c.Journal(self.path,timeout(),True) as j:
            self.model.manifest['generation']=43
            with self.assertRaises(ValueError):c.commit(j,self.model,'assign',PLAN['plan_digest'])
            self.assertEqual(j.get('assign')['state'],'prepared');self.model.manifest['generation']=42
            append=j.append
            def revoke(entry):
                append(entry)
                if entry['state']=='dispatch_started':os.environ.pop('DFMCP_WORKFORCE_ALLOW_LABOR',None)
            with patch.object(j,'append',side_effect=revoke),self.assertRaises(ValueError):c.commit(j,self.model,'assign',PLAN['plan_digest'])
            self.assertEqual(j.get('assign')['state'],'dispatch_started');self.assertNotIn('CommitAssignment',self.model.calls)


class NetworkTests(unittest.TestCase):
    def test_fragmented_end_to_end_lifecycle(self):
        file=path()
        with NativeDouble() as native,patch.dict(os.environ,{**ENV,'DFMCP_WORKFORCE_ENDPOINT':native.endpoint},clear=True):
            with w.Client(native.endpoint,b't'*32,timeout()) as client:
                observed=client.call('ObserveWorkforce',unit_ids=[2,5]);self.assertEqual(observed['capture_hex'],V['capture'].hex())
                binding={**BINDING,'endpoint':native.endpoint}
                with c.Journal(file,timeout(),True,binding) as j:
                    c.prepare(j,client,PLAN);e=c.commit(j,client,'assign',PLAN['plan_digest']);self.assertEqual(e['state'],'terminal')
            self.assertEqual(native.effects,1)
    def test_lost_response_reconnect_queries_only(self):
        file=path()
        with NativeDouble(2,lose_commit=True) as native,patch.dict(os.environ,{**ENV,'DFMCP_WORKFORCE_ENDPOINT':native.endpoint},clear=True):
            binding={**BINDING,'endpoint':native.endpoint}
            with w.Client(native.endpoint,b't'*32,timeout()) as client,c.Journal(file,timeout(),True,binding) as j:
                c.prepare(j,client,PLAN)
                with self.assertRaises(ValueError):c.commit(j,client,'assign',PLAN['plan_digest'])
            with w.Client(native.endpoint,b't'*32,timeout()) as client,c.Journal(file,timeout(),True) as j:
                e,_=c.reconcile(j,client,'assign');self.assertEqual(e['state'],'terminal')
            self.assertEqual(native.effects,1);self.assertEqual(native.calls.count('QueryAssignment'),1)
    def test_nonce_alias_notification_and_shared_deadline(self):
        for config in ({'bad_nonce':True},{'alias':True},{'notifications':9},{'delay':0.06}):
            with NativeDouble(**config) as native:
                with self.assertRaises((OSError,ValueError)):
                    with w.Client(native.endpoint,b't'*32,time.monotonic()+(0.02 if 'delay' in config else 5)):pass
    def test_cli_prepare_commit_inspect_process(self):
        file=path()
        with NativeDouble(2) as native:
            env={**os.environ,**ENV,'DFMCP_WORKFORCE_ENDPOINT':native.endpoint}
            for name in list(env):
                if name.startswith('DFMCP_') and name not in c.ALLOWED:del env[name]
            cmd=[sys.executable,str(ROOT/'scripts/workforce_client.py')]
            result=subprocess.run(cmd+['prepare','--journal',str(file),'--key','assign','--units','2,5','--detail','0','--assigned','yes','--world-folder','region1','--site-id','7'],env=env,text=True,capture_output=True,timeout=10,check=True)
            digest=json.loads(result.stdout)['result']['effect']['plan_digest'];self.assertEqual(digest,PLAN['plan_digest'])
            done=subprocess.run(cmd+['commit','--journal',str(file),'--key','assign','--confirm-plan',digest],env=env,text=True,capture_output=True,timeout=10,check=True)
            self.assertEqual(json.loads(done.stdout)['result']['effect']['state'],'terminal')
        result=subprocess.run(cmd+['inspect','--journal',str(file),'--key','assign'],env={},text=True,capture_output=True,timeout=10,check=True)
        self.assertFalse(json.loads(result.stdout)['result']['native_contacted'])


if __name__=='__main__':unittest.main(verbosity=2)
