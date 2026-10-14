"""Complete route-batch source with explicit acquisition/route/custody/effect ports.

Actual authority/budget, owner review/advance, prefix gate and native-value /
operations codecs execute. This is NOT a private-journal or TCP/game test.
"""
from contextlib import contextmanager
from dataclasses import replace
import hashlib
import importlib.util
import os
from pathlib import Path
import re
import sys
import types
import unittest
from unittest.mock import patch

from build_placement_wire import Plan, Record, canonical, integer, require
from construction_receipt import Manifest
from test_construction_prefix import receipt, fields, operations, job
from test_construction_frontier import route_result, reseal


def module(name, **values):
    out = types.ModuleType(name)
    out.__dict__.update(values)
    return out


@contextmanager
def owner_fixture():
    f = types.SimpleNamespace(events=[], reads=0, modes=['complete'], raw=None, clients=[],
                              route_status='ready', route_work=100, expired=False)
    previous = receipt()
    before = replace(Record.decode(receipt(1)).plan.before, tick=20, next_job=1000, next_building=2000)
    reply = types.SimpleNamespace(capture=before, manifest=Manifest(11,'df','dfhack'),
                                  unresolved=False, retained_records=1)
    def capture_raw(mode):
        b,i = fields(previous)
        _,next_item = fields(receipt(1))
        next_item.update(holder=None, flags=64, position=before.item.pos)
        jobs=[]
        if mode=='pending':
            b['stage']=0
            jobs=[job()]
        if mode=='suspended':
            b['stage']=0
            jobs=[job(suspended=True)]
        if mode=='missing':
            return operations(records=(),items=[next_item])
        if mode=='error':raise OSError('simulated acquisition failure')
        return operations(records=(previous,),buildings=[b],items=[i,next_item],jobs=jobs)

    class ReadAuthority:
        def __init__(self,*args):pass
    class Region:
        def __init__(self,*args):pass
    class ReadClient:
        def __init__(self,authority,region,budget):
            self.closed=False
            self.manifests=dict(map=Manifest(7,'df','dfhack'),operations=Manifest(9,'df','dfhack'))
            f.clients.append(self)
        def __enter__(self):return self
        def __exit__(self,*args):self.closed=True
        def capture_once(self):
            f.events.append('read')
            mode=f.modes[min(f.reads,len(f.modes)-1)];f.reads+=1
            f.raw=capture_raw(mode)
            self.closed=True
            return self.manifests['map'],b'map-port',self.manifests['operations'],f.raw
    class NativeClient:
        def __init__(self,*args):pass
        def __enter__(self):return self
        def __exit__(self,*args):pass
        def observe(self):
            f.events.append('observe')
            return reply
    class Source:
        names=('endpoint','df_version','dfhack_version','map_generation','inventory_generation',
               'map_sha256','inventory_sha256','tick','dimensions','origin','size','horizons')
        def __init__(self,*args):self.__dict__.update(zip(self.names,args))
    def digest_text(value):
        require(type(value) is str and re.fullmatch('[0-9a-f]{64}',value) is not None,'invalid digest')
    def decode_terrain(raw,guard):
        assert all(c.closed for c in f.clients)
        guard()
        return types.SimpleNamespace(tick=20,paused=True,digest=hashlib.sha256(raw).hexdigest(),
                                     dimensions=(128,128,10),origin=(0,0,5),size=(64,64,1))
    def candidates_from(observed,request,guard):
        guard()
        return tuple(i for i in observed.items.values() if i.flags==64),{}
    def route_assess(review,batch_id,prefix,before,candidates,types_,raw,source,guard,**kwargs):
        assert all(c.closed for c in f.clients)
        assert source.inventory_sha256==hashlib.sha256(f.raw).hexdigest()
        guard()
        value=route_result(f.raw,prefix,status=f.route_status,work_units=f.route_work)
        value.update(batch_id=batch_id,native_before_digest=before.witness.hex())
        return reseal(value)
    placement=module('build_placement_client')
    native=module('build_placement_rpc',TOKEN='DFMCP_BUILD_TOKEN',Client=NativeClient,
        endpoint=lambda _:('127.0.0.1',5000),METHODS=('Handshake','ReadObservation','QueryPlacement',
                                                 'PreparePlacement','CommitPlacement','CancelPlacement'))
    ports={'build_placement_client':placement,'build_placement_rpc':native,
           'furniture_plan':module('furniture_plan',canonical=canonical,integer=integer,require=require),
           'route_furniture_review':module('route_furniture_review',Review=object,Source=Source,digest_text=digest_text),
           'route_furniture_frontier':module('route_furniture_frontier',assess=route_assess),
           'route_furniture_batch_store':module('route_furniture_batch_store',Batch=object),
           'route_furniture_inventory':module('route_furniture_inventory',Authority=ReadAuthority,Region=Region,
                                              Client=ReadClient,candidates_from=candidates_from),
           'route_furniture_allocation':module('route_furniture_allocation',decode_terrain=decode_terrain)}
    env={k:v for k,v in os.environ.items() if not k.startswith('DFMCP_')}
    env.update(DFMCP_ALLOW_UNADMITTED_ROUTE_FURNITURE_BATCH='1',DFMCP_ROUTE_FURNITURE_ALLOW_PLACE='1',
               DFMCP_BUILD_TOKEN='b'*32,DFMCP_MAP_TOKEN='m'*32,DFMCP_OPERATIONS_PAGED_TOKEN='o'*32)
    name='_construction_batch_owner_under_test'
    spec=importlib.util.spec_from_file_location(name,Path(__file__).with_name('route_furniture_batch.py'))
    owner=importlib.util.module_from_spec(spec);ports[name]=owner
    with patch.dict(sys.modules,ports),patch.dict(os.environ,env,clear=True):
        spec.loader.exec_module(owner)
        class BatchPort:
            def __init__(self):
                self.id='a'*64;self.review=types.SimpleNamespace(digest='b'*64)
                self.address=('127.0.0.1',5000);self.native=reply.manifest
                self.source=Source('127.0.0.1:5000','df','dfhack',7,9,'0'*64,'1'*64,10,
                                   (128,128,10),(0,0,5),(64,64,1),(100,200,300))
                self.request=types.SimpleNamespace(folder='region1',site=2)
                step=types.SimpleNamespace(name='next',kind='bed',item=11,target=before.selection.target)
                self.plan=types.SimpleNamespace(ordered=(step,),steps=(step,))
                self.budget=owner.Budget(60000);self.writable=True;self.stopped=False
                self.pending=False;self.done=False;self.entries=[]
                self.effects=types.SimpleNamespace(journals={'prefix-0':types.SimpleNamespace(raw=previous)})
            def check(self):
                f.events.append('custody_check')
            def bind(self,capture,manifest):
                require(capture.raw==before.raw and manifest==self.native,'wrong capture')
            def key(self,step):return 'prefix-1'
            def placed_prefix(self):return (previous,)
            def audit(self):
                return dict(batch_id=self.id,review_digest=self.review.digest,advance_allowed=not self.pending and not self.done,
                            pending_step='next' if self.pending else None,next_step='next' if not self.done else None,
                            inventory_digest='c'*64,inventory_verified=True,steps=[])
            def register(self,step,evidence):
                self.entries.append(dict(slot=step.name,route_evidence_digest=evidence))
                f.events.append('register')
        batch=BatchPort()
        def start(effects,authority,selection,key,expected,guard):
            f.events.append('start')
            p=Plan(key,before)
            require(expected==p.digest.hex(),'wrong native plan')
            guard(p,reply,'before_intent')
            batch.pending=True
            effects.journals[key]=types.SimpleNamespace(raw=b'original-intent-port')
            f.events.append('intent_port')
            guard(p,reply,'before_prepare')
            f.events.append('prepare_port')
            f.events.append('dispatch_intent_port')
            guard(p,reply,'before_commit')
            f.events.append('commit_port')
            batch.pending=False;batch.done=True
            return {'phase':'placed'},None
        placement.start=start
        f.owner,f.batch,f.authority,f.before=owner,batch,owner.Authority.load(True),before
        yield f


class ConstructionBatchGateTests(unittest.TestCase):
    def test_default_acquisition_path_checks_pending_predecessor_and_closes_source(self):
        with owner_fixture() as f:
            f.modes=['pending']
            out=f.owner.acquire_frontier(f.batch,f.before,f.authority,f.batch.budget)
            self.assertEqual(out['status'],'construction_pending')
            self.assertEqual(f.reads,1)
            self.assertTrue(all(c.closed for c in f.clients))
            self.assertLess(f.batch.budget.work_steps,14_000_000)
            self.assertNotIn('start',f.events)

    def test_pending_review_exports_diagnostics_but_no_confirmation(self):
        with owner_fixture() as f:
            f.modes=['suspended']
            out=f.owner.review_step(f.batch,f.authority)
            self.assertIsNone(out['expected_plan'])
            self.assertIsNone(out['confirm_review'])
            self.assertFalse(out['advance_allowed'])
            packet=f.owner.packet('review',out)
            self.assertEqual(packet['agent_turn']['briefing']['execution_frontier_status'],'construction_pending')
            self.assertEqual(packet['agent_turn']['briefing']['predecessor_construction_status'],'pending')
            self.assertEqual(out['route_frontier']['construction_prefix']['rows'][0]['reason'],'suspended')
            self.assertNotIn('start',f.events)

    def test_current_installed_predecessor_allows_one_original_step(self):
        with owner_fixture() as f:
            review=f.owner.review_step(f.batch,f.authority)
            out=f.owner.advance(f.batch,review['expected_plan'],review['confirm_review'],f.authority)
            self.assertEqual(out['placement']['phase'],'placed')
            self.assertEqual(f.events.count('commit_port'),1)
            self.assertEqual(f.reads,3)
            self.assertLess(f.batch.budget.work_steps,2_000_000)
            self.assertEqual(out['route_frontier']['construction_prefix']['status'],'quiescent')

    def test_regression_before_child_intent_cannot_prepare_or_dispatch(self):
        with owner_fixture() as f:
            f.modes=['complete','pending']
            review=f.owner.review_step(f.batch,f.authority)
            with self.assertRaises(ValueError):
                f.owner.advance(f.batch,review['expected_plan'],review['confirm_review'],f.authority)
            self.assertFalse(f.batch.pending)
            self.assertNotIn('intent_port',f.events)
            self.assertNotIn('prepare_port',f.events)
            self.assertNotIn('commit_port',f.events)

    def test_final_precommit_regression_retains_pending_key_without_dispatch_or_retry(self):
        with owner_fixture() as f:
            f.modes=['complete','complete','pending']
            review=f.owner.review_step(f.batch,f.authority)
            with self.assertRaises(ValueError):
                f.owner.advance(f.batch,review['expected_plan'],review['confirm_review'],f.authority)
            self.assertTrue(f.batch.pending)
            self.assertIn('dispatch_intent_port',f.events)
            self.assertNotIn('commit_port',f.events)
            starts=f.events.count('start')
            with self.assertRaises(ValueError):
                f.owner.advance(f.batch,review['expected_plan'],review['confirm_review'],f.authority)
            self.assertEqual(f.events.count('start'),starts)

    def test_final_acquisition_failure_never_falls_back_to_old_positive_predicate(self):
        with owner_fixture() as f:
            f.modes=['complete','complete','error']
            review=f.owner.review_step(f.batch,f.authority)
            with self.assertRaises(OSError):
                f.owner.advance(f.batch,review['expected_plan'],review['confirm_review'],f.authority)
            self.assertTrue(f.batch.pending)
            self.assertNotIn('commit_port',f.events)

    def test_shared_allowance_cannot_be_replenished_for_construction(self):
        with owner_fixture() as f:
            f.route_work=f.owner.ANALYSIS_RESERVATION
            with self.assertRaises(ValueError):
                f.owner.review_step(f.batch,f.authority)
            self.assertEqual(f.reads,1)
            self.assertNotIn('start',f.events)

    def test_old_route_only_confirmation_does_not_authorize_new_execution(self):
        with owner_fixture() as f:
            review=f.owner.review_step(f.batch,f.authority)
            old={'evidence_digest':review['route_frontier']['route_evidence_digest']}
            seal=f.owner.confirmation(f.batch,Plan('prefix-1',f.before),'c'*64,old)
            self.assertNotEqual(seal,review['confirm_review'])
            with self.assertRaises(ValueError):
                f.owner.advance(f.batch,review['expected_plan'],seal,f.authority)
            self.assertFalse(f.batch.pending)
            self.assertNotIn('commit_port',f.events)

    def test_actual_current_authority_still_gates_the_effect_boundary(self):
        with owner_fixture() as f:
            review=f.owner.review_step(f.batch,f.authority)
            os.environ[f.owner.PLACE]='0'
            with self.assertRaises(ValueError):
                f.owner.advance(f.batch,review['expected_plan'],review['confirm_review'],f.authority)
            self.assertNotIn('start',f.events)


if __name__=='__main__':
    unittest.main()
