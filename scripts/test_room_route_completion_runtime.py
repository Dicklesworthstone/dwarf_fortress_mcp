"""Real private monitor/replay/CLI and acquisition orchestration, explicit ports.

Original batch custody, native receipt/assessment and room diagnosis use the
core test producers. TCP, operations projection and native methods are not
qualified here. The unchanged private-file and foreground-wait owners do run.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from contextlib import contextmanager
from dataclasses import replace
import io
import ipaddress
import json
import os
from pathlib import Path
import sys
import tempfile
import time
from types import ModuleType, SimpleNamespace as NS
import unittest
from unittest.mock import patch

from furniture_plan import integer, require
from test_room_route_completion import environment, load, Manifest, map_bytes


class Budget:
    def __init__(self, timeout=60000):
        self.deadline=time.monotonic()+integer(timeout,1,60000)/1000
        self.disk_bytes=1024*1024*1024
        self.work_steps=20_000_000
        self.calls=400
    def remaining(self):
        left=self.deadline-time.monotonic();require(left>0,'timeout');return left
    def work(self):
        self.remaining();require(self.work_steps>0,'work');self.work_steps-=1
    def reserve(self,kind,count):
        self.remaining();integer(count,0,1024*1024*1024)
        require(count<=getattr(self,kind),'budget');setattr(self,kind,getattr(self,kind)-count)


def endpoint(value):
    host,port=value.split(':');address=ipaddress.IPv4Address(host)
    require(address.is_loopback and f'{address}:{int(port)}'==value,'endpoint')
    return str(address),integer(int(port),1,65535)


@contextmanager
def runtime(count=2,**timing):
    with environment(count,**timing) as (model,goal,sample), patch.dict(sys.modules):
        wire=sys.modules['build_placement_wire']
        wire.Rejected=ValueError
        def text(raw,maximum):
            require(type(raw) is bytes and 1<=len(raw)<=maximum and b'\0' not in raw,'text')
            return raw.decode()
        wire.text=text
        condition=model.construction
        receipt=sys.modules['construction_receipt']
        for name in ('LinkedSample','Progress','MAX_SAMPLE','advance','begin_read','cancel'):
            setattr(receipt,name,getattr(condition,name))
        transport=ModuleType('construction_monitor_rpc')
        transport.Budget,transport.endpoint=Budget,endpoint
        sys.modules['construction_monitor_rpc']=transport
        custody=load('_room_custody_test','construction_monitor_store.py')
        sys.modules['construction_monitor_store']=custody
        sys.modules['room_route_completion']=model
        store=load('_room_store_test','room_route_completion_store.py')
        sys.modules['room_route_completion_store']=store
        native=ModuleType('room_route_completion_rpc')
        native.Budget=Budget
        calls=[]
        class Authority:
            address=('127.0.0.1',5000)
            revoked=False
            @classmethod
            def load(cls):
                calls.append('authority');auth.guard();return auth
            def guard(self): require(not self.revoked,'revoked')
        auth=Authority()
        native.Authority=Authority
        tick=[2]
        def acquire(a,g,budget):
            calls.append('acquire');a.guard();budget.reserve('calls',17)
            result=sample(tick[0]);tick[0]+=1;return result
        native.acquire=acquire
        sys.modules['room_route_completion_rpc']=native
        original=sys.modules['route_furniture_completion']
        original.origin_path=lambda path: path if Path(path).is_absolute() else (_ for _ in ()).throw(ValueError('path'))
        def verify(child,batch):
            batch.check();require(batch.complete and child.encode(lambda:None)==batch.original.encode(lambda:None),'original')
        original.verify_origin=verify
        foreground=load('_room_wait_test','construction_wait.py')
        sys.modules['construction_wait']=foreground
        cli=load('_room_cli_test','track_room_route_completion.py')
        batch=NS(original=goal.original,room_handoff=goal.handoff,complete=True,closed=False,
            budget=Budget(),address=auth.address,corrupt=False)
        def check(): require(not batch.corrupt and not batch.closed,'batch custody')
        batch.check=check
        batch.close=lambda:setattr(batch,'closed',True)
        def open_batch(path,expected,budget):
            require(expected==goal.batch_id,'batch id');batch.budget=budget;batch.closed=False;batch.check();return batch
        cli.open_batch=open_batch
        yield NS(model=model,goal=goal,sample=sample,store=store,custody=custody,cli=cli,
                 native=native,auth=auth,batch=batch,calls=calls,tick=tick,foreground=foreground)


class StoreTests(unittest.TestCase):
    def test_persistent_joint_success_across_reopen_and_fixed_goal(self):
        with runtime() as r,tempfile.TemporaryDirectory() as root:
            path=root+'/monitor'
            with r.store.Journal(path,Budget(),writable=True,create=(r.goal,r.auth.address)) as j:
                j.start_read();j.accept(r.sample(2),lambda _:None)
                self.assertEqual(j.state.progress.phase,'candidate')
            with r.store.Journal(path,Budget(),writable=True) as j:
                self.assertFalse(j.read_owned)
                self.assertEqual(j.state.goal.digest,r.goal.digest)
                self.assertEqual(j.state.goal.condition.deadline,100)
                j.start_read();j.accept(r.sample(3),lambda _:None)
                self.assertEqual(j.state.progress.phase,'satisfied')
            raw=Path(path).read_bytes()
            with r.store.Journal(path,Budget()) as j:
                self.assertEqual(j.state.progress.phase,'satisfied')
                self.assertFalse(j.cancel(lambda _:None))
            self.assertEqual(Path(path).read_bytes(),raw)

    def test_interrupted_read_is_persistent_but_ownership_is_not(self):
        with runtime() as r,tempfile.TemporaryDirectory() as root:
            path=root+'/monitor'
            with r.store.Journal(path,Budget(),writable=True,create=(r.goal,r.auth.address)) as j:
                j.start_read();j.accept(r.sample(2),lambda _:None);j.start_read()
            with r.store.Journal(path,Budget(),writable=True) as j:
                self.assertTrue(j.state.progress.reading)
                with self.assertRaises(ValueError): j.accept(r.sample(3),lambda _:None)
                j.start_read()
                self.assertEqual((j.state.progress.streak,j.state.progress.interruptions),(0,1))
                j.accept(r.sample(3),lambda _:None)
                self.assertEqual(j.state.progress.phase,'candidate')

    def test_wrong_original_endpoint_refuses_before_file_creation(self):
        with runtime() as r,tempfile.TemporaryDirectory() as root:
            path=root+'/monitor'
            with self.assertRaises(ValueError):
                r.store.Journal(path,Budget(),writable=True,create=(r.goal,('127.0.0.1',5001)))
            self.assertFalse(Path(path).exists())

    def test_render_refusal_keeps_unknown_read_and_does_not_publish_sample(self):
        with runtime() as r,tempfile.TemporaryDirectory() as root:
            path=root+'/monitor'
            with r.store.Journal(path,Budget(),writable=True,create=(r.goal,r.auth.address)) as j:
                j.start_read();before=Path(path).read_bytes()
                def fail(_): raise ValueError('output cannot fit')
                with self.assertRaises(ValueError): j.accept(r.sample(),fail)
                self.assertTrue(j.fenced)
            self.assertEqual(Path(path).read_bytes(),before)
            with r.store.Journal(path,Budget()) as j:
                self.assertTrue(j.state.progress.reading)
                self.assertEqual(j.state.progress.observations,0)

    def test_changed_bytes_path_and_nonprivate_mode_fence_owner(self):
        for change in ('byte','replace','mode'):
            with self.subTest(change=change),runtime() as r,tempfile.TemporaryDirectory() as root:
                path=root+'/monitor'
                with r.store.Journal(path,Budget(),writable=True,create=(r.goal,r.auth.address)) as j:
                    raw=Path(path).read_bytes()
                    if change=='byte': Path(path).write_bytes(raw[:-1]+bytes([raw[-1]^1]))
                    elif change=='replace':
                        os.rename(path,path+'.old');Path(path).write_bytes(raw);os.chmod(path,0o600)
                    else: os.chmod(path,0o644)
                    with self.assertRaises(ValueError): j.check()
                    self.assertTrue(j.fenced)

    def test_exclusive_creation_lock_and_readonly_owner(self):
        with runtime() as r,tempfile.TemporaryDirectory() as root:
            path=root+'/monitor'
            with r.store.Journal(path,Budget(),writable=True,create=(r.goal,r.auth.address)):
                with self.assertRaises(OSError): r.store.Journal(path,Budget())
                with self.assertRaises(FileExistsError): r.store.Journal(path,Budget(),writable=True,create=(r.goal,r.auth.address))
            with r.store.Journal(path,Budget()) as j:
                with self.assertRaises(ValueError): j.start_read()
                with self.assertRaises(ValueError): j.cancel(lambda _:None)

    def test_lost_sync_acknowledgement_fences_without_repair(self):
        with runtime() as r,tempfile.TemporaryDirectory() as root:
            path=root+'/monitor'
            with r.store.Journal(path,Budget(),writable=True,create=(r.goal,r.auth.address)) as j:
                j.start_read()
                with patch.object(r.custody.os,'fsync',side_effect=OSError('lost acknowledgement')):
                    with self.assertRaises(OSError): j.accept(r.sample(),lambda _:None)
                self.assertTrue(j.fenced)
            raw=Path(path).read_bytes()
            with r.store.Journal(path,Budget()) as j:
                self.assertEqual(j.state.progress.observations,1)
                self.assertFalse(j.read_owned)
            self.assertEqual(raw,Path(path).read_bytes())

    def test_every_truncated_or_corrupt_last_frame_is_rejected(self):
        with runtime() as r,tempfile.TemporaryDirectory() as root:
            path=root+'/monitor'
            with r.store.Journal(path,Budget(),writable=True,create=(r.goal,r.auth.address)) as j:
                j.start_read();prefix=Path(path).read_bytes();j.accept(r.sample(),lambda _:None)
            raw=Path(path).read_bytes();start=len(prefix)
            def replay(value):
                return r.store.replay_reader(lambda offset,n:value[offset:offset+n],len(value),Budget())
            for end in range(start+1,len(raw)):
                with self.assertRaises(ValueError): replay(raw[:end])
            for pos in range(start,len(raw)):
                changed=bytearray(raw);changed[pos]^=1
                with self.assertRaises(ValueError): replay(bytes(changed))
            self.assertEqual(replay(prefix).progress.observations,0)
            self.assertEqual(replay(raw).progress.observations,1)


class WorkflowTests(unittest.TestCase):
    def invoke(self,r,root,operation,*flags):
        args=[operation,'--journal',root+'/monitor','--batch-directory','/original',
              '--batch-id',r.goal.batch_id,'--timeout-ms','60000',*flags]
        stream=io.BytesIO()
        with patch.object(r.cli.sys,'stdout',NS(buffer=stream)):
            code=r.cli.main(args)
        return code,json.loads(stream.getvalue())

    def test_start_then_sample_complete_with_original_parent_visible(self):
        with runtime() as r,tempfile.TemporaryDirectory() as root:
            code,out=self.invoke(r,root,'start','--deadline-tick','100')
            self.assertEqual(code,0);self.assertEqual(out['result']['progress']['phase'],'candidate')
            code,out=self.invoke(r,root,'sample','--expected-goal',r.goal.digest)
            self.assertEqual(code,0);self.assertEqual(out['result']['progress']['phase'],'satisfied')
            self.assertEqual(out['result']['room_handoff_digest'],r.goal.handoff.digest)
            self.assertEqual(out['result']['original_room_plan'],{'original':'all'})
            self.assertEqual(r.calls.count('acquire'),2)

    def test_inspect_and_cancel_need_no_native_credentials(self):
        with runtime() as r,tempfile.TemporaryDirectory() as root:
            self.invoke(r,root,'start','--deadline-tick','100')
            count=len(r.calls);r.auth.revoked=True
            for operation in ('inspect','cancel','inspect'):
                code,out=self.invoke(r,root,operation,'--expected-goal',r.goal.digest)
                self.assertEqual(code,0);self.assertFalse(out['result']['native_contacted_this_call'])
            self.assertEqual(out['result']['progress']['phase'],'cancelled')
            self.assertEqual(len(r.calls),count)

    def test_wait_uses_same_owner_and_does_not_advance_game_time(self):
        with runtime() as r,tempfile.TemporaryDirectory() as root:
            self.invoke(r,root,'start','--deadline-tick','100')
            code,out=self.invoke(r,root,'wait','--expected-goal',r.goal.digest,'--wait-samples','2','--poll-ms','10')
            self.assertEqual(code,0)
            self.assertEqual(out['result']['wait']['stop_reason'],'terminal')
            self.assertEqual(out['result']['wait']['samples_published'],1)
            self.assertFalse(out['result']['wait']['game_time_advanced_by_wait'])

    def test_extra_map_call_floor_is_reserved_before_read_intent(self):
        with runtime() as r,tempfile.TemporaryDirectory() as root:
            self.invoke(r,root,'start','--deadline-tick','100')
            before=Path(root+'/monitor').read_bytes();calls=r.calls.count('acquire')
            class SmallBudget(Budget):
                def __init__(self,timeout): super().__init__(timeout);self.calls=16
            with patch.object(r.native,'Budget',SmallBudget):
                code,out=self.invoke(r,root,'wait','--expected-goal',r.goal.digest,'--wait-samples','2')
            self.assertEqual(code,0);self.assertEqual(out['result']['wait']['stop_reason'],'rpc_allowance')
            self.assertEqual(r.calls.count('acquire'),calls)
            self.assertEqual(Path(root+'/monitor').read_bytes(),before)

    def test_acquisition_failure_remains_reading_without_retry(self):
        with runtime() as r,tempfile.TemporaryDirectory() as root:
            with patch.object(r.native,'acquire',side_effect=OSError('private native detail')) as acquire:
                code,out=self.invoke(r,root,'start','--deadline-tick','100')
            self.assertEqual(code,2);self.assertIsNone(out['result']);self.assertEqual(acquire.call_count,1)
            with r.store.Journal(root+'/monitor',Budget()) as j:
                self.assertTrue(j.state.progress.reading);self.assertEqual(j.state.progress.observations,0)

    def test_reopen_cannot_renew_goal_or_change_original_identity(self):
        with runtime() as r,tempfile.TemporaryDirectory() as root:
            self.invoke(r,root,'start','--deadline-tick','100')
            before=Path(root+'/monitor').read_bytes();count=r.calls.count('acquire')
            for flags in (('--expected-goal','f'*64),('--expected-goal',r.goal.digest,'--stable-samples','1'),
                          ('--expected-goal',r.goal.digest,'--deadline-tick','200')):
                code,out=self.invoke(r,root,'sample',*flags)
                self.assertEqual(code,2);self.assertIsNone(out['result'])
            self.assertEqual(r.calls.count('acquire'),count);self.assertEqual(Path(root+'/monitor').read_bytes(),before)

    def test_late_authority_loss_suppresses_serialized_success(self):
        with runtime() as r,tempfile.TemporaryDirectory() as root:
            original=r.cli.output
            def render(value):
                raw=original(value)
                if value['result'] is not None and 'bytes' in value['result']['journal'] and value['result']['journal']['bytes']<r.store.MAX_FILE:
                    r.auth.revoked=True
                return raw
            with patch.object(r.cli,'output',render):
                code,out=self.invoke(r,root,'start','--deadline-tick','100')
            self.assertEqual(code,2);self.assertIsNone(out['result'])

    def test_original_custody_is_required_even_for_terminal_output(self):
        with runtime(stable_samples=1,stable_span=0) as r,tempfile.TemporaryDirectory() as root:
            self.invoke(r,root,'start','--deadline-tick','100')
            r.batch.corrupt=True
            code,out=self.invoke(r,root,'inspect','--expected-goal',r.goal.digest)
            self.assertEqual(code,2);self.assertIsNone(out['result'])

    def test_short_output_is_not_retried_and_closes_owners(self):
        with runtime() as r,tempfile.TemporaryDirectory() as root:
            class Short:
                calls=0
                def write(self,raw):self.calls+=1;return len(raw)-1
                def flush(self):pass
            stream=Short()
            with patch.object(r.cli.sys,'stdout',NS(buffer=stream)):
                code=r.cli.main(['start','--journal',root+'/monitor','--batch-directory','/original',
                                '--batch-id',r.goal.batch_id,'--deadline-tick','100'])
            self.assertEqual(code,2);self.assertEqual(stream.calls,1);self.assertTrue(r.batch.closed)
            with r.store.Journal(root+'/monitor',Budget()):pass


if __name__=='__main__':
    unittest.main()
