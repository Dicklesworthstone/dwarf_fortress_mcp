"""Real placement client + batch + POSIX + subprocess tests, with a joined TCP peer.

The peer simulates furniture/1.19 only. It is not DFHack/SDK/live-game evidence.
"""
from __future__ import annotations

from contextlib import contextmanager
from dataclasses import replace
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

import furniture_batch as b
import furniture_plan as p
import build_placement_rpc as rpc
import build_placement_wire as w
import build_placement_store as store
import build_placement_client as client


def varint(n):
    out = bytearray()
    while n > 127:
        out.append((n & 127) | 128); n >>= 7
    out.append(n)
    return bytes(out)


def proto(fields):
    out = bytearray()
    for field, value in sorted(fields.items()):
        if isinstance(value, bytes):
            out += varint(8*field+2) + varint(len(value)) + value
        else:
            out += varint(8*field) + varint(value)
    return bytes(out)


class Peer:
    def __init__(self, plan):
        self.plan = plan
        self.items = {s.item: client.KINDS[s.kind] for s in plan.steps}
        self.records, self.placed, self.calls, self.commits = {}, {}, [], []
        self.generation, self.sequence, self.tick = 7, 4, 900
        self.next_building, self.next_job = 100, 200
        self.folder, self.site, self.dimensions = 'test-fort', 2, (256, 256, 16)
        self.df, self.dfhack = 'df-test', 'dfhack-test'
        self.lost = self.missing = self.indeterminate = self.refused = self.full = False
        self.extra_unresolved = False
        self.before_reply = None
        self.capture_change = lambda capture: capture
        self.errors, self.connections = [], 0
        self.closed = threading.Event()
        self.server = socket.socket()
        self.server.bind(('127.0.0.1',0)); self.server.listen(4); self.server.settimeout(.05)
        self.address = self.server.getsockname()
        self.thread = threading.Thread(target=self.serve)
        self.thread.start()

    def capture(self, selected):
        tiles=[]
        x,y,z=selected.target
        for py in range(y-1,y+2):
            for px in range(x-1,x+2):
                building=self.placed.get((px,py,z))
                tiles.append(w.Tile(2,42,3,occupied=building is not None,building=building))
        available = selected.item not in {r.plan.before.selection.item for r in self.records.values() if r.phase=='placed'}
        item=w.Item(2,(150,150,2),self.items[selected.item],3,-1,0,2,
                    on_ground=available,in_job=not available,ground=w.Tile(2,42,3))
        return self.capture_change(w.Capture(self.generation,self.sequence,self.tick,self.site,self.dimensions,
            self.next_building,self.next_job,len(self.placed),self.folder,True,
            selected.target not in self.placed,True,selected,tuple(tiles),item))

    @staticmethod
    def read(sock, size):
        out=bytearray()
        while len(out)<size:
            data=sock.recv(size-len(out))
            if not data: raise EOFError()
            out += data
        return bytes(out)

    @staticmethod
    def send(sock, raw):
        # Deliberately split native headers and protobuf payloads.
        for start in range(0,len(raw),37): sock.sendall(raw[start:start+37])

    def serve(self):
        while not self.closed.is_set():
            try: sock,_=self.server.accept()
            except socket.timeout: continue
            except OSError: break
            self.connections+=1
            with sock:
                sock.settimeout(3); sock.setsockopt(socket.IPPROTO_TCP,socket.TCP_NODELAY,1)
                try: self.connection(sock)
                except (EOFError,ConnectionError,OSError): pass
                except BaseException as error: self.errors.append(error)

    def connection(self,sock):
        assert self.read(sock,12)==b'DFHack?\n'+struct.pack('<i',1)
        self.send(sock,b'DFHack!\n'+struct.pack('<i',1))
        while not self.closed.is_set():
            method,size=struct.unpack('<h2xi',self.read(sock,8)); assert 0<=size<=2048
            fields=rpc.decode(self.read(sock,size))
            if method==0:
                assert fields[2]==b'dfmcp.build.v1_19.Request'
                assert fields[3]==b'dfmcp.build.v1_19.Reply'
                assert fields[4]==b'dfmcp_build_v1_19'
                name=fields[1].decode(); payload=proto({1:rpc.METHODS.index(name)+2})
            else:
                operation=rpc.METHODS[method-2]; self.calls.append(operation)
                assert fields[1]==b't'*32
                assert fields[3]==1 and fields[4]==19
                extra={}
                if operation=='ReadPlacement':
                    extra[9]=self.capture(w.Selection(*(fields[i] for i in range(5,10)))).raw
                elif operation=='PreparePlacement':
                    key=fields[10].decode()
                    if key not in self.records:
                        plan=w.Plan(key,self.capture(w.Selection(*(fields[i] for i in range(5,10)))))
                        assert fields[11]==plan.before.witness and fields[12]==plan.digest
                        self.records[key]=w.Record(plan,'prepared','none'); extra[11]=0
                    else: extra[11]=1
                    extra[10]=self.records[key].raw
                elif operation in ('CommitPlacement','QueryPlacement','CancelPlacement'):
                    key=fields[10].decode(); record=self.records.get(key)
                    if record is not None:
                        assert record.plan.digest==fields[12]
                        if operation!='QueryPlacement': assert fields[13]==record.plan.token
                        if operation=='CommitPlacement' and record.phase=='prepared':
                            if self.refused:
                                record=w.Record(record.plan,'refused','stale')
                            elif self.indeterminate:
                                self.commits.append(key)
                                record=w.Record(record.plan,'indeterminate','native_failure')
                            else:
                                self.commits.append(key); before=record.plan.before
                                assert before.raw==self.capture(before.selection).raw
                                insertion=w.Insertion(before.next_building,before.next_job,before.selection.item,
                                    before.selection.kind,before.selection.target,before.item.material,before.item.material_index,
                                    0,3,True,True,True,False)
                                record=w.Record(record.plan,'placed','none',before.expected_after(),insertion)
                                self.placed[before.selection.target]=before.next_building
                                self.next_building+=1; self.next_job+=1; self.sequence+=1
                            self.records[key]=record
                        elif operation=='CancelPlacement' and record.phase=='prepared':
                            record=w.Record(record.plan,'cancelled','cancelled'); self.records[key]=record
                        if not (self.missing and operation=='QueryPlacement'): extra[10]=record.raw
                    if operation=='CommitPlacement' and self.lost:
                        self.lost=False
                        return
                unresolved=self.extra_unresolved or any(r.phase=='indeterminate' for r in self.records.values())
                count=256 if self.full else max(len(self.records),int(unresolved))
                response={1:1,2:0,3:fields[2],4:1,5:19,6:self.generation,
                          7:self.df.encode(),8:self.dfhack.encode(),12:int(unresolved),13:count,**extra}
                if self.before_reply: self.before_reply(operation,response)
                payload=proto(response)
            self.send(sock,struct.pack('<h2xi',-1,len(payload))+payload)

    def close(self):
        self.closed.set(); self.server.close(); self.thread.join(5)
        assert not self.thread.is_alive(), 'test peer failed to join'
        if self.errors: raise self.errors[0]


@contextmanager
def environment(peer,place='1'):
    env={key:value for key,value in os.environ.items() if not key.startswith('DFMCP_')}
    env.update({rpc.OPT_IN:'1',rpc.TOKEN:'t'*32,rpc.ENDPOINT:f'{peer.address[0]}:{peer.address[1]}',rpc.PLACE:place})
    with patch.dict(os.environ,env,clear=True): yield


def make_plan(count=3):
    return p.FurniturePlan(tuple(p.Step(f'room{i:02}',p.KINDS[i%3],i+1,(10+3*i,10,i%4),
                            (f'room{i-1:02}',) if i else ()) for i in range(count)))


class BatchTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.root=Path(self.temp.name)/'batch'; self.root.mkdir(mode=0o700)
        self.plan=make_plan(); self.peer=Peer(self.plan)
        self.env=environment(self.peer); self.env.__enter__()
        self.initial=b.initialize(str(self.root),self.plan,'test-fort',2)
        self.id=self.initial['batch_id']

    def tearDown(self):
        self.env.__exit__(None,None,None)
        self.peer.close(); self.temp.cleanup()

    def review(self): return b.review(str(self.root),self.id)
    def advance(self,review=None):
        review=review or self.review()
        return b.advance(str(self.root),self.id,review['expected_plan'],review['confirm_review'])
    def inspect(self,step=None): return b.inspect(str(self.root),self.id,step)
    def recover(self,cancel=False,step='room00'):
        return b.recover(str(self.root),self.id,step,cancel)
    def child(self): return next((self.root/'effects').iterdir())
    def cli(self,operation,*args):
        proc=subprocess.run([sys.executable,str(Path(b.__file__)),operation,'--directory',str(self.root),
             '--batch-id',self.id,*args],capture_output=True,timeout=15)
        return proc,json.loads(proc.stdout)

    def test_three_kind_multilevel_real_client_places_exactly_one_per_call(self):
        self.assertFalse(self.peer.commits)
        self.assertNotIn('PreparePlacement',self.peer.calls)
        for i in range(3):
            review=self.review()
            self.assertEqual(review['next_step'],f'room{i:02}')
            self.assertFalse(review['before']['structural_safety_proved'])
            out=self.advance(review)
            self.assertEqual(out['placed'],i+1)
            self.assertEqual(len(self.peer.commits),i+1)
        self.assertEqual(out['status'],'all_placed')
        self.assertEqual(set(self.peer.placed),{s.target for s in self.plan.steps})
        self.assertFalse(out['construction_completion_proven'])
        count=len(self.peer.calls)
        with patch.dict(os.environ,{},clear=True):
            offline=self.inspect('room00')
            receipt=w.Record.decode(bytes.fromhex(offline['receipt']['canonical_record_hex']))
            self.assertEqual(receipt.plan.before.selection, b.selection(self.plan.ordered[0]))
        self.assertEqual(len(self.peer.calls),count)
        self.assertTrue(offline['inventory_verified'])

    def test_repeated_old_confirmation_cannot_place_next_step(self):
        review=self.review(); self.advance(review)
        with self.assertRaises(ValueError): self.advance(review)
        self.assertEqual(len(self.peer.commits),1)
        self.assertEqual(len(list((self.root/'effects').iterdir())),1)

    def test_lost_commit_reply_blocks_batch_then_queries_original_key(self):
        self.peer.lost=True
        with self.assertRaises(ValueError): self.advance()
        out=self.inspect(); self.assertEqual(out['pending_step'],'room00')
        self.assertFalse(out['advance_allowed'])
        with self.assertRaises(ValueError): self.review()
        before=self.child().read_bytes()
        out=self.recover()
        self.assertEqual(out['placed'],1); self.assertEqual(len(self.peer.commits),1)
        self.assertTrue(self.child().read_bytes().startswith(before))
        self.advance(); self.assertEqual(len(self.peer.commits),2)

    def test_absent_native_record_never_means_safe_to_retry(self):
        self.peer.lost=True
        with self.assertRaises(ValueError): self.advance()
        self.peer.missing=True
        out=self.recover()
        self.assertEqual(out['native_query_status'],'unknown_absent_native_record')
        self.assertFalse(out['advance_allowed'])
        self.assertEqual(len(self.peer.commits),1)

    def test_indeterminate_receipt_is_absorbing_and_offline(self):
        self.peer.indeterminate=True
        out=self.advance(); self.assertEqual(out['status'],'pending_recovery')
        self.assertEqual(out['steps'][0]['phase'],'indeterminate')
        count=len(self.peer.calls)
        with patch.dict(os.environ,{},clear=True): out=self.recover(True)
        self.assertFalse(out['native_contacted']); self.assertEqual(len(self.peer.calls),count)
        self.assertFalse(out['advance_allowed'])

    def test_refused_child_halts_even_independent_remaining_steps(self):
        self.peer.refused=True
        out=self.advance(); self.assertEqual(out['status'],'halted_refused')
        with self.assertRaises(ValueError): self.review()
        self.assertEqual(len(self.peer.commits),0)

    def test_stale_source_or_fortress_refused_before_intent(self):
        for field,value in [('generation',8),('folder','other'),('site',3),('df','other'),('dimensions',(257,256,16))]:
            old=getattr(self.peer,field); setattr(self.peer,field,value)
            with self.assertRaises(ValueError): self.review()
            setattr(self.peer,field,old)
        self.assertFalse(list((self.root/'effects').iterdir()))

    def test_changed_capture_and_wrong_batch_confirmation_do_not_prepare(self):
        review=self.review(); self.peer.tick+=1
        with self.assertRaises(ValueError): self.advance(review)
        review=self.review()
        with self.assertRaises(ValueError): b.advance(str(self.root),'0'*64,review['expected_plan'],review['confirm_review'])
        with self.assertRaises(ValueError): b.advance(str(self.root),self.id,review['expected_plan'],'0'*64)
        self.assertFalse(list((self.root/'effects').iterdir()))
        self.assertNotIn('PreparePlacement',self.peer.calls)

    def test_copied_confirmation_does_not_authorize_another_batch(self):
        review=self.review(); other=Path(self.temp.name)/'other'; other.mkdir(mode=0o700)
        initialized=b.initialize(str(other),self.plan,'test-fort',2)
        with self.assertRaises(ValueError): b.advance(str(other),initialized['batch_id'],review['expected_plan'],review['confirm_review'])
        self.assertFalse(list((other/'effects').iterdir()))

    def test_native_fence_and_retention_capacity_refuse_without_intent(self):
        for field in ('full','extra_unresolved'):
            review=self.review(); setattr(self.peer,field,True)
            blocked=self.review(); self.assertIsNone(blocked['confirm_review'])
            with self.assertRaises(ValueError): self.advance(review)
            setattr(self.peer,field,False)
        self.assertFalse(list((self.root/'effects').iterdir()))

    def test_stop_is_persistent_offline_and_preserves_original_recovery(self):
        self.peer.lost=True
        with self.assertRaises(ValueError): self.advance()
        with patch.dict(os.environ,{},clear=True):
            stopped=b.stop(str(self.root),self.id)
            before=(self.root/'stop.json').read_bytes()
            self.assertTrue(stopped['stopped']); self.assertEqual(stopped['pending_step'],'room00')
            b.stop(str(self.root),self.id)
            self.assertEqual((self.root/'stop.json').read_bytes(),before)
        out=self.recover(); self.assertEqual(out['placed'],1); self.assertFalse(out['advance_allowed'])
        with self.assertRaises(ValueError): self.review()

    def test_stop_appearing_after_prepare_prevents_dispatch(self):
        def hook(operation,_response):
            if operation=='PreparePlacement':
                path=self.root/'stop.json'
                path.write_bytes(b.seal({'schema':'dfmcp.furniture-batch-stop/1','batch_id':self.id}))
                path.chmod(0o600)
        self.peer.before_reply=hook
        with self.assertRaises(ValueError): self.advance()
        self.peer.before_reply=None
        self.assertFalse(self.peer.commits)
        self.assertTrue(self.inspect()['stopped'])
        out=self.recover(True); self.assertEqual(out['status'],'halted_cancelled')

    def test_placement_revocation_after_prepare_still_allows_query_only_cancel(self):
        def hook(operation,_response):
            if operation=='PreparePlacement': os.environ[rpc.PLACE]='0'
        self.peer.before_reply=hook
        with self.assertRaises(ValueError): self.advance()
        self.peer.before_reply=None
        self.assertFalse(self.peer.commits)
        out=self.recover(True); self.assertEqual(out['status'],'halted_cancelled')

    def test_deleted_registered_child_or_replaced_inode_never_becomes_new_work(self):
        self.advance(); child=self.child(); raw=child.read_bytes(); child.unlink()
        with self.assertRaises(ValueError): self.inspect()
        child.write_bytes(raw); child.chmod(0o600)
        # Force a different inode even on filesystems that immediately recycle it.
        temp=self.root/'replacement'; temp.write_bytes(raw); temp.chmod(0o600); os.replace(temp,child)
        with self.assertRaises(ValueError): self.inspect()
        self.assertEqual(len(self.peer.commits),1)

    def test_manifest_and_index_corruption_are_not_repaired(self):
        for name in ('batch.json','steps.jsonl'):
            path=self.root/name; original=path.read_bytes()
            path.write_bytes(original[:-1])
            with self.assertRaises(ValueError): self.inspect()
            self.assertEqual(path.read_bytes(),original[:-1])
            path.write_bytes(original)
        self.assertTrue(self.inspect()['inventory_verified'])

    def test_modes_links_foreign_entries_and_directory_lock(self):
        (self.root/'batch.json').chmod(0o644)
        with self.assertRaises(ValueError): self.inspect()
        (self.root/'batch.json').chmod(0o600)
        os.link(self.root/'batch.json',Path(self.temp.name)/'link')
        with self.assertRaises(ValueError): self.inspect()
        (Path(self.temp.name)/'link').unlink()
        (self.root/'foreign').touch()
        with self.assertRaises(ValueError): self.inspect()
        (self.root/'foreign').unlink()
        with b.Batch(str(self.root),rpc.Budget(10000)):
            with self.assertRaises(BlockingIOError): self.inspect()
        effects=self.root/'effects'; effects.rename(self.root/'saved')
        effects.symlink_to(self.root/'saved',target_is_directory=True)
        with self.assertRaises((ValueError,OSError)): self.inspect()

    def test_real_subprocess_advance_and_reopen_query_lost_reply(self):
        review=self.review(); self.peer.lost=True
        proc,out=self.cli('advance','--expected-plan',review['expected_plan'],'--confirm-review',review['confirm_review'])
        self.assertEqual(proc.returncode,2,proc.stderr)
        self.assertFalse(out['ok']); self.assertFalse(out['result']['inventory_verified'])
        proc,out=self.cli('query','--step','room00')
        self.assertEqual(proc.returncode,0,proc.stderr); self.assertEqual(out['result']['placed'],1)
        self.assertEqual(len(self.peer.commits),1)
        with patch.dict(os.environ,{},clear=True):
            proc,out=self.cli('inspect','--step','room00')
        self.assertEqual(proc.returncode,0,proc.stderr)
        self.assertEqual(w.Record.decode(bytes.fromhex(out['result']['receipt']['canonical_record_hex'])).phase,'placed')

    def test_shared_deadline_and_closed_environment(self):
        review=self.review()
        os.environ['DFMCP_ADMITTED_BRIDGE_PROTOCOL']='1.0'
        with self.assertRaises(ValueError): self.advance(review)
        del os.environ['DFMCP_ADMITTED_BRIDGE_PROTOCOL']
        self.assertTrue(self.inspect()['advance_allowed'])
        with patch.object(rpc.Budget,'remaining',side_effect=ValueError('expired')):
            with self.assertRaises(ValueError): self.advance(review)
        self.assertFalse(self.peer.commits)

    def test_all_ten_advance_sync_failures_preserve_one_shot_semantics(self):
        sync=os.fsync
        for point in range(1,11):
            with self.subTest(sync=point):
                path=Path(self.temp.name)/f'failure{point}'; path.mkdir(mode=0o700)
                init=b.initialize(str(path),self.plan,'test-fort',2)
                review=b.review(str(path),init['batch_id']); calls=0
                commits=len(self.peer.commits)
                def fail(fd):
                    nonlocal calls
                    calls+=1
                    if calls==point: raise OSError('injected sync failure')
                    return sync(fd)
                with patch('os.fsync',side_effect=fail):
                    with self.assertRaises(OSError): b.advance(str(path),init['batch_id'],review['expected_plan'],review['confirm_review'])
                self.assertEqual(len(self.peer.commits)-commits,0 if point<=8 else 1)
                out=b.inspect(str(path),init['batch_id'])
                # Complete-but-unacknowledged receipt bytes may be reverified.
                self.assertEqual(out['placed'],int(point>=9))
                if point<=8: self.assertFalse(out['advance_allowed'])
                # Reset only the explicit test peer world for another independent case.
                self.peer.records.clear(); self.peer.placed.clear()
                self.peer.next_building=100; self.peer.next_job=200; self.peer.sequence=4

    def test_orphan_intent_recovery_never_prepares_or_commits(self):
        with patch.object(b.Batch,'register',side_effect=OSError('before index append')):
            with self.assertRaises(OSError): self.advance()
        self.assertNotIn('PreparePlacement',self.peer.calls)
        self.assertEqual(self.inspect()['pending_step'],'room00')
        out=self.recover()
        self.assertTrue(out['steps'][0]['registered'])
        self.assertEqual(out['native_query_status'],'unknown_absent_native_record')
        self.assertNotIn('CommitPlacement',self.peer.calls)
        self.assertNotIn('PreparePlacement',self.peer.calls)

    def test_torn_index_append_fences_without_repair(self):
        real_write=store.write_all
        def torn(fd,raw):
            if b'"intent_sha256"' in raw:
                os.write(fd,raw[:len(raw)//2]); raise OSError('partial index append')
            return real_write(fd,raw)
        with patch.object(store,'write_all',side_effect=torn):
            with self.assertRaises(OSError): self.advance()
        original=(self.root/'steps.jsonl').read_bytes()
        with self.assertRaises(ValueError): self.inspect()
        self.assertEqual((self.root/'steps.jsonl').read_bytes(),original)
        self.assertNotIn('PreparePlacement',self.peer.calls)

    def test_predecessor_horizon_regression_refuses_before_new_intent(self):
        self.advance(); self.peer.sequence=4
        review=self.review()
        with self.assertRaises(ValueError): self.advance(review)
        self.assertEqual(len(list((self.root/'effects').iterdir())),1)
        self.assertEqual(len(self.peer.commits),1)

    def test_original_single_placement_api_still_operates(self):
        root=Path(self.temp.name)/'single'; root.mkdir(mode=0o700)
        selected=b.selection(self.plan.ordered[0]); authority=rpc.Authority.load(True)
        digest=w.Plan('preview',self.peer.capture(selected)).digest.hex()
        with store.PlacementDirectory(str(root),rpc.Budget(10000),True) as owner:
            out,plan=client.start(owner,authority,selected,'single',digest)
            self.assertEqual(out['effect_status'],'placed')
        self.assertEqual(self.peer.commits,['single'])

    def test_32_real_placements_and_complete_output_bound(self):
        self.peer.close(); self.env.__exit__(None,None,None)
        plan=p.FurniturePlan(tuple(p.Step(f'r{i:02}'+('x'*45),p.KINDS[i%3],2147483600+i,
                         (10+3*i,10,i%4)) for i in range(32)))
        self.peer=Peer(plan)
        self.peer.folder='é'*256; self.peer.df='d'*128; self.peer.dfhack='h'*128
        self.peer.generation=2**64-2; self.peer.sequence=2**64-34; self.peer.tick=w.MAX_TICK
        self.peer.next_building=2147483600; self.peer.next_job=2147483600
        self.env=environment(self.peer); self.env.__enter__()
        root=Path(self.temp.name)/'full'; root.mkdir(mode=0o700)
        initialized=b.initialize(str(root),plan,self.peer.folder,2)
        maximum=0
        for _ in plan.steps:
            review=b.review(str(root),initialized['batch_id'])
            maximum=max(maximum,len(b.encoded('review',review)))
            out=b.advance(str(root),initialized['batch_id'],review['expected_plan'],review['confirm_review'])
            maximum=max(maximum,len(b.encoded('advance',out)))
        self.assertEqual(out['placed'],32); self.assertEqual(len(self.peer.commits),32)
        self.assertEqual(len(out['steps']),32); self.assertLess(maximum,b.MAX_OUTPUT)
        self.assertEqual(out['status'],'all_placed')
        inspected=b.inspect(str(root),initialized['batch_id'],plan.ordered[0].name)
        maximum=max(maximum,len(b.encoded('inspect',inspected)))
        self.assertLess(maximum,b.MAX_OUTPUT)
        print('MAXIMUM_32_STEP_RESPONSE_BYTES',maximum)


    def test_all_independent_native_fixtures_still_match_batch_dependencies(self):
        path=Path(__file__).resolve().parents[1]/'bridge/common/tests/fixtures/build_placement_v1_19.json'
        fixtures=json.loads(path.read_text())
        capture=w.Capture.decode(bytes.fromhex(fixtures['capture']))
        plan=w.Plan('golden',capture)
        self.assertEqual(plan.digest.hex(),fixtures['plan'])
        self.assertEqual(plan.token.hex(),fixtures['token'])
        step=p.Step('golden',w.KINDS[capture.selection.kind],capture.selection.item,capture.selection.target)
        self.assertEqual(b.selection(step),capture.selection)
        for name in ('prepared','placed','expired','cancelled','indeterminate'):
            raw=bytes.fromhex(fixtures[name]); record=w.Record.decode(raw,plan)
            self.assertEqual(record.raw,raw)

    def test_all_initialization_sync_failures_preserve_files_without_reinitialization(self):
        sync=os.fsync
        for point in range(1,5):
            path=Path(self.temp.name)/f'init-fail{point}'; path.mkdir(mode=0o700)
            count=0
            def fail(fd):
                nonlocal count
                count+=1
                if count==point: raise OSError('initial publication interrupted')
                return sync(fd)
            with patch('os.fsync',side_effect=fail):
                with self.assertRaises(OSError): b.initialize(str(path),self.plan,'test-fort',2)
            self.assertTrue(list(path.iterdir()))
            with self.assertRaises(ValueError): b.initialize(str(path),self.plan,'test-fort',2)
        self.assertNotIn('PreparePlacement',self.peer.calls)
        self.assertFalse(self.peer.commits)

    def test_failed_stop_sync_keeps_stop_without_renewing_permission(self):
        for point in (1,2):
            path=Path(self.temp.name)/f'stop-fail{point}'; path.mkdir(mode=0o700)
            value=b.initialize(str(path),self.plan,'test-fort',2)
            count=0; sync=os.fsync
            def fail(fd):
                nonlocal count
                count+=1
                if count==point: raise OSError('stop acknowledgment lost')
                return sync(fd)
            with patch('os.fsync',side_effect=fail):
                with self.assertRaises(OSError): b.stop(str(path),value['batch_id'])
            out=b.inspect(str(path),value['batch_id'])
            self.assertTrue(out['stopped']); self.assertFalse(out['advance_allowed'])
        self.assertFalse(self.peer.commits)

    def test_torn_original_child_intent_never_reaches_native_prepare(self):
        real_write=store.write_all
        def torn(fd,raw):
            if b'"kind":"intent"' in raw:
                os.write(fd,raw[:len(raw)//2]); raise OSError('partial original intent')
            return real_write(fd,raw)
        with patch.object(store,'write_all',side_effect=torn):
            with self.assertRaises(OSError): self.advance()
        original=self.child().read_bytes()
        with self.assertRaises(ValueError): self.inspect()
        self.assertEqual(self.child().read_bytes(),original)
        self.assertNotIn('PreparePlacement',self.peer.calls)


if __name__=='__main__': unittest.main()
