"""Delivery-order integration using actual workflow source and explicit I/O seams.

The existing inventory dependency is injected while loading each complete source
module. Acquisition, budget and outer packet are doubles; selectors, sequence,
review codecs, CLI/file read, create/check/finish functions run unchanged. This
suite does NOT execute the native transport or claim a full runtime-import test.
"""
from __future__ import annotations

from dataclasses import replace
import hashlib
import itertools
from pathlib import Path
import random
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

from furniture_allocation import Candidate, Request, Slot
from furniture_plan import FurniturePlan, Step, canonical
import route_furniture_allocation as routes
import route_furniture_sequence as sequencing
import route_furniture_review as reviews
from test_cross_level_furniture import terrain, item, oracle_distances
from test_cross_level_furniture_review import source

ROOT = Path(__file__).resolve().parent


def corridor(dependency=False):
    raw, cells = terrain(size=(6,1,1), stairs=False)
    r = Request('fort',1,(Slot('a-near','bed',(3,1,0)),
        Slot('z-far','chair',(5,1,0),after=('a-near',) if dependency else ())))
    items = (item(1,(1,1,0)),item(2,(2,1,0),kind='chair'))
    return r, items, raw, cells


def feasible(plan, candidates, cells, names, limit):
    by_id, by_name = {c.id:c for c in candidates},{s.name:s for s in plan.steps}
    current, done = dict(cells),set()
    for name in names:
        s=by_name[name]
        if not set(s.after)<=done:
            return False
        distances=oracle_distances(current,s.target)
        if by_id[s.item].position not in distances or distances[by_id[s.item].position]>limit:
            return False
        current[s.target]=[2,3,0,1,0,1]
        done.add(name)
    return done==set(by_name)


class Budget:
    def __init__(self, _timeout=10000, work=20_000_000):
        self.work_steps=work
        self.expired=False
    def remaining(self):
        if self.expired:
            raise ValueError('deadline')
        return 1
    def work(self):
        self.remaining()
        if self.work_steps<=0:
            raise ValueError('work exhausted')
        self.work_steps-=1
    def reserve(self, _kind, _amount):
        self.remaining()


class Authority:
    address=('127.0.0.1',5000)
    def __init__(self):
        self.revoked=False
        self.checks=0
    def guard(self):
        self.checks+=1
        if self.revoked:
            raise ValueError('revoked')
    @classmethod
    def load(cls):
        return cls()


class Region:
    def __init__(self, origin, size):
        self.origin,self.size=origin,size


def boundary_packet(result=None, request=None, source=None):
    return {'ok':result is not None,'result':result,'source':source,
            'request':None if request is None else request.json(),
            'agent_turn':{'coverage':{},'uncertainty':[]}}


def bounded(value):
    raw=canonical(value)+b'\n'
    if len(raw)>96*1024:
        raise ValueError('output bound')
    return raw


def workflow():
    boundary=types.ModuleType('route_furniture_inventory')
    boundary.Region,boundary.Authority,boundary.routes=Region,Authority,routes
    boundary.wire=types.SimpleNamespace(Budget=Budget)
    boundary.packet,boundary.bounded_output=boundary_packet,bounded
    boundary.ACQUISITION_POLICY='injected-test-acquisition'
    boundary.CANDIDATE_POLICY='injected-test-projection'
    def load(name, filename, injections):
        module=types.ModuleType(name)
        module.__file__=str(ROOT/filename)
        # Exact production bytes, with no AST/source rewriting. Scope the import
        # seam so unrelated tests keep their normal modules and native policies.
        with patch.dict(sys.modules,injections):
            exec(compile((ROOT/filename).read_bytes(),str(ROOT/filename),'exec'),module.__dict__)
        return module
    runtime=load('_delivery_runtime','route_furniture_review_runtime.py',
                 {'route_furniture_inventory':boundary})
    cli=load('_delivery_cli','review_route_furniture.py',
             {'route_furniture_inventory':boundary,'route_furniture_review_runtime':runtime})
    return runtime,cli,boundary


def captured(r, items, raw):
    kinds={'bed':1,'chair':2,'table':3}
    return raw,source(raw),items,{i.id:kinds[i.kind] for i in items},{'candidate':len(items)}


class SequenceOracleTests(unittest.TestCase):
    def test_corridor_reorders_without_reallocation_or_constraint_loss(self):
        r,items,raw,cells=corridor()
        old=canonical(r.json())
        e,out=sequencing.allocate_sequenced_routes(r,items,raw)
        seq=out['delivery_sequence']
        self.assertEqual(seq['order'],['z-far','a-near'])
        self.assertEqual(e.slots[0].after,('z-far',))
        self.assertEqual(canonical(r.json()),old)
        self.assertEqual([row['item'] for row in out['assignments']],[1,2])
        self.assertFalse(seq['future_routes_guaranteed'])
        self.assertTrue(feasible(FurniturePlan.from_json(out['plan']),items,cells,seq['order'],20))

    def test_required_blocking_precedence_is_not_weakened(self):
        r,items,raw,_=corridor(True)
        e,out=sequencing.allocate_sequenced_routes(r,items,raw)
        self.assertEqual(out['status'],'delivery_order_blocked')
        self.assertIsNone(out['plan'])
        self.assertEqual(out['delivery_sequence']['routes'],[])
        self.assertEqual(out['delivery_sequence']['order'],[])
        self.assertEqual(e,r)
        self.assertIsNone(out['shortage'])

    def test_512_obstacle_masks_against_every_fixed_assignment_order(self):
        kinds=('bed','chair','table')
        targets=((1,1,0),(2,2,0),(3,3,0))
        starts=((3,1,0),(1,3,0),(2,1,0))
        r=Request('fort',1,tuple(Slot(n,k,t) for n,k,t in zip('abc',kinds,targets)))
        items=tuple(item(i,p,kind=k) for i,(p,k) in enumerate(zip(starts,kinds)))
        plan=FurniturePlan(tuple(Step(s.name,s.kind,i,s.target) for i,s in enumerate(r.slots)))
        for mask in range(512):
            changes={(i%3+1,i//3+1,0):(2,2,0,0,0,1) for i in range(9) if mask&(1<<i)}
            raw,cells=terrain(size=(3,3,1),stairs=False,changes=changes)
            _,out=sequencing.allocate_sequenced_routes(r,items,raw,max_route_steps=8)
            possible=[p for p in itertools.permutations('abc') if feasible(plan,items,cells,p,8)]
            self.assertEqual(out['status']=='allocated',bool(possible),mask)
            if possible:
                self.assertIn(tuple(out['delivery_sequence']['order']),possible)
            else:
                self.assertIsNone(out['plan'])

    def test_240_random_dependency_graphs_and_limits_against_all_permutations(self):
        rng=random.Random(91817)
        points=list(itertools.product(range(1,5),range(1,5),(0,)))
        for case in range(240):
            targets,starts=rng.sample(points,4),rng.choices(points,k=4)
            order=rng.sample(list('abcd'),4)
            edges={n:tuple(p for p in order[:order.index(n)] if rng.random()<.3) for n in order}
            r=Request('fort',1,tuple(Slot(n,'bed',t,after=edges[n],material=(419+i,-1))
                                    for i,(n,t) in enumerate(zip('abcd',targets))))
            items=tuple(item(i,p,material=(419+i,-1)) for i,p in enumerate(starts))
            plan=FurniturePlan(tuple(Step(s.name,s.kind,i,s.target,s.after) for i,s in enumerate(r.slots)))
            changes={p:(2,2,0,0,0,1) for p in points if rng.random()<.13}
            raw,cells=terrain(size=(4,4,1),stairs=False,changes=changes)
            limit=rng.randint(0,10)
            e,out=sequencing.allocate_sequenced_routes(r,items,raw,max_route_steps=limit)
            possible=[p for p in itertools.permutations('abcd') if feasible(plan,items,cells,p,limit)]
            self.assertEqual(out['status']=='allocated',bool(possible),case)
            if possible:
                self.assertIn(tuple(out['delivery_sequence']['order']),possible)
                for old,new in zip(r.slots,e.slots):
                    self.assertTrue(set(old.after)<=set(new.after))
                    self.assertEqual(replace(new,after=old.after),old)

    def test_32_step_export_and_shared_budget_boundary(self):
        raw,_=terrain(size=(10,10,1),stairs=False)
        targets=[(x,y,0) for y in (3,5,7,9) for x in range(2,10)]
        r=Request('fort',1,tuple(Slot(f's{i:02d}','bed',p) for i,p in enumerate(targets)))
        items=tuple(item(i,(1,1,0)) for i in range(32))
        e,out=sequencing.allocate_sequenced_routes(r,items,raw)
        self.assertEqual(len(out['delivery_sequence']['routes']),32)
        review=reviews.create(e,out,source(raw),{i:1 for i in range(32)})
        self.assertEqual(len(review.unpack()[3].steps),32)
        used=out['work_units']
        self.assertEqual(sequencing.allocate_sequenced_routes(r,items,raw,maximum_work=used)[1]['status'],'allocated')
        with self.assertRaises(ValueError):
            sequencing.allocate_sequenced_routes(r,items,raw,maximum_work=used-1)


class DeliveryWorkflowTests(unittest.TestCase):
    def run_create(self, runtime, r, items, raw, **options):
        with patch.object(runtime,'acquire',return_value=captured(r,items,raw)) as read:
            result=runtime.create(r,Region((1,1,0),(6,1,1)),Authority(),Budget(),**options)
            self.assertEqual(read.call_count,1)
        import json
        return json.loads(result)

    def test_opt_in_exports_effective_review_and_complete_order_report(self):
        runtime,_,_=workflow()
        r,items,raw,_=corridor()
        out=self.run_create(runtime,r,items,raw,sequence_deliveries=True)
        self.assertIn('delivery_sequence',out['result'])
        self.assertEqual(out['result']['delivery_sequence']['order'],['z-far','a-near'])
        retained=reviews.Review.decode(canonical(out['review']),out['review_digest'])
        self.assertEqual(retained.unpack()[1].json(),out['request'])
        self.assertEqual([s.name for s in retained.unpack()[3].ordered],['z-far','a-near'])
        self.assertEqual(out['source']['pure_result_digest'],out['result']['decision_digest'])
        self.assertTrue(out['agent_turn']['coverage']['delivery_order_checked'])
        self.assertFalse(out['agent_turn']['coverage']['placement_gate_enforced'])
        self.assertEqual(out['result']['delivery_sequence']['original_request'],r.json())

    def test_legacy_default_does_not_invoke_sequencer_or_change_packet(self):
        runtime,_,_=workflow()
        r,items,raw,_=corridor()
        with patch.object(runtime.sequencing,'allocate_sequenced_routes',side_effect=AssertionError('unexpected')):
            implicit=self.run_create(runtime,r,items,raw)
            explicit=self.run_create(runtime,r,items,raw,sequence_deliveries=False)
        self.assertEqual(implicit,explicit)
        self.assertNotIn('delivery_sequence',implicit['result'])
        self.assertEqual(implicit['request'],r.json())

    def test_blocked_sequence_has_no_exported_review_or_plan(self):
        runtime,_,_=workflow()
        r,items,raw,_=corridor(True)
        out=self.run_create(runtime,r,items,raw,sequence_deliveries=True)
        self.assertTrue(out['ok'])  # Established diagnostic, not feasible construction.
        self.assertEqual(out['result']['status'],'delivery_order_blocked')
        self.assertIsNone(out['review'])
        self.assertIsNone(out['result']['plan'])
        self.assertFalse(out['agent_turn']['coverage']['delivery_order_checked'])

    def test_precontact_option_and_export_budget_checks(self):
        runtime,_,_=workflow()
        r,items,raw,_=corridor()
        with patch.object(runtime,'acquire',side_effect=AssertionError('contact')):
            for value in (1,'true',None,[],{}):
                with self.assertRaises(ValueError):
                    runtime.create(r,Region((1,1,0),(6,1,1)),Authority(),Budget(),sequence_deliveries=value)
            with self.assertRaises(ValueError):
                runtime.create(r,Region((1,1,0),(6,1,1)),Authority(),Budget(work=runtime.EXPORT_WORK),sequence_deliveries=True)

    def test_final_authority_and_output_failure_cannot_publish_cached_review(self):
        runtime,_,boundary=workflow()
        r,items,raw,_=corridor()
        authority=Authority()
        def revoke(value):
            encoded=bounded(value)
            authority.revoked=True
            return encoded
        with patch.object(runtime,'acquire',return_value=captured(r,items,raw)), patch.object(boundary,'bounded_output',side_effect=revoke):
            with self.assertRaises(ValueError):
                runtime.create(r,Region((1,1,0),(6,1,1)),authority,Budget(),sequence_deliveries=True)
        with patch.object(runtime,'acquire',return_value=captured(r,items,raw)), patch.object(boundary,'bounded_output',side_effect=ValueError('overflow')):
            with self.assertRaises(ValueError):
                runtime.create(r,Region((1,1,0),(6,1,1)),Authority(),Budget(),sequence_deliveries=True)

    def test_reopened_review_check_does_not_reorder_or_reallocate(self):
        runtime,_,_=workflow()
        r,items,raw,_=corridor()
        initial=self.run_create(runtime,r,items,raw,sequence_deliveries=True)
        review=reviews.Review.decode(canonical(initial['review']),initial['review_digest'])
        with patch.object(runtime,'acquire',return_value=captured(r,items,raw)), patch.object(runtime.sequencing,'allocate_sequenced_routes',side_effect=AssertionError('reorder')):
            result=runtime.check(review,Authority(),Budget())
        import json
        out=json.loads(result)
        self.assertEqual(out['result']['original_plan_digest'],review.unpack()[3].digest)
        self.assertEqual(review.json(),initial['review'])

    def test_real_cli_create_reads_unchanged_input_and_passes_opt_in(self):
        runtime,cli,_=workflow()
        r,items,raw,_=corridor()
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'request.json'
            before=canonical(r.json());path.write_bytes(before)
            args=['create','--request-file',str(path),'--origin','1','1','0','--size','6','1','1','--sequence-deliveries']
            with patch.object(runtime,'acquire',return_value=captured(r,items,raw)),patch.object(cli,'publish',return_value=0) as publish:
                self.assertEqual(cli.main(args),0)
                import json
                out=json.loads(publish.call_args.args[0])
                self.assertEqual(out['result']['delivery_sequence']['status'],'reordered')
                self.assertEqual(publish.call_count,1)
            self.assertEqual(path.read_bytes(),before)

    def test_cli_check_rejects_reordering_before_files_or_native_contact(self):
        runtime,cli,_=workflow()
        with patch.object(cli,'read_file',side_effect=AssertionError('file read')),patch.object(runtime,'check',side_effect=AssertionError('contact')),patch.object(cli,'publish',return_value=0) as publish:
            args=['check','--review-file','old.json','--expected-review','0'*64,'--sequence-deliveries']
            self.assertEqual(cli.main(args),2)
            self.assertEqual(publish.call_count,1)

    def test_ordered_export_retains_cross_floor_limits(self):
        runtime,_,_=workflow()
        raw,_=terrain()
        r=Request('fort',1,(Slot('bed','bed',(4,2,2),max_vertical_distance=2),))
        out=self.run_create(runtime,r,(item(),),raw,sequence_deliveries=True)
        review=reviews.Review.decode(canonical(out['review']),out['review_digest'])
        self.assertEqual(review.unpack()[1].slots[0].max_vertical_distance,2)
        self.assertEqual(out['result']['delivery_sequence']['routes'][0]['route_steps'],6)


if __name__=='__main__':
    unittest.main()
