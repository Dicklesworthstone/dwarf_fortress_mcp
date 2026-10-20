"""Actual optimized new-review creation/reopen, with injected native acquisition.

Use real request/review codecs and planner; the map bytes are decoded for real.
The operations-roster decoder and native client remain injected, not qualified.
"""
from __future__ import annotations

from dataclasses import replace
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from furniture_plan import canonical
from furniture_allocation import Candidate, Request, Slot
import route_furniture_review as model
import route_furniture_joint as joint
import route_furniture_optimal as optimal
from test_route_furniture_optimal import blocked_case, capture, item
from test_route_furniture_optimal_runtime import Budget, Harness, ROOT, TOKEN, load


class ReviewHarness(Harness):
    def __enter__(self):
        super().__enter__()
        self.runtime=load('_dfmcp_optimal_review_runtime',ROOT/'route_furniture_review_runtime.py')
        sys.modules['route_furniture_review_runtime']=self.runtime
        self.review_cli=load('_dfmcp_optimal_review_cli',ROOT/'review_route_furniture.py')
        return self
    def create(self, **kwargs):
        return json.loads(self.runtime.create(self.request,self.region,self.authority,Budget(),
            **{'sequence_deliveries':True,'minimize_delivery_cost':True,**kwargs}))
    def review_args(self,path,*options):
        return ['create',*self.argv(path,*options)]


class OptimalReviewTests(unittest.TestCase):
    def setUp(self):
        self.h=self.enterContext(ReviewHarness())

    def test_optimized_review_preserves_exact_complete_result_on_reopen(self):
        out=self.h.create(require_optimal_delivery=True)
        saved=model.Review.decode(canonical(out['review']),out['review_digest'])
        original,request,source,plan=saved.unpack()
        self.assertEqual(plan.json(),out['result']['plan'])
        self.assertEqual(request.json(),out['request'])
        self.assertEqual(original['allocation_digest'],out['result']['decision_digest'])
        self.assertEqual(source.map_sha256,hashlib.sha256(self.h.raw).hexdigest())
        self.assertEqual(out['result']['delivery_optimization']['best_complete_cost'],7)
        self.assertTrue(out['agent_turn']['coverage']['delivery_cost_optimality_proven'])
        self.assertNotIn('It preserves selected items',' '.join(out['agent_turn']['uncertainty']))
        self.assertEqual(len(original['selections']),3)
        self.assertEqual(self.h.captures,1)

    def test_complete_32_target_optimized_review_fits_existing_packet_and_codec_bounds(self):
        request,items,_=blocked_case()
        slots=tuple(Slot(f't{i:02}','table',(12+i,2,1)) for i in range(30))
        self.h.request=replace(request,slots=request.slots+slots)
        self.h.items=items+tuple(item(100+i,'table',12+i,2) for i in range(30))
        self.h.raw=capture(48,2,hidden=tuple(48+x for x in range(48) if x not in range(11,41)))
        self.h.region=self.h.inventory.Region((1,1,1),(48,2,1))
        self.h.observed.horizons=(1000,1000,1000)
        self.h.observed.items={c.id:SimpleNamespace(id=c.id,kind=c.kind.upper(),flags=64,
            container=None,holder=None,stack=1,material=c.material[0],material_index=c.material[1],
            position=c.position,subtype=c.subtype,native_type={'bed':1,'chair':2,'table':3}[c.kind])
            for c in self.h.items}
        out=self.h.create(require_optimal_delivery=True)
        self.assertEqual(len(out['review']['selections']),32)
        saved=model.Review.decode(canonical(out['review']),out['review_digest'])
        self.assertEqual(len(saved.unpack()[3].steps),32)
        self.assertLess(len(canonical(out)),self.h.inventory.MAX_OUTPUT)
        self.assertTrue(out['result']['delivery_optimization']['cost_optimality_proven'])

    def test_solver_cannot_borrow_artifact_export_reservation(self):
        original=optimal.allocate_optimal_routes
        budget=Budget()
        def checked(*args,**kwargs):
            self.assertEqual(budget.work_steps,self.h.runtime.EXPORT_WORK)
            self.assertTrue(self.h.last.closed)
            return original(*args,**kwargs)
        with patch.object(optimal,'allocate_optimal_routes',side_effect=checked):
            raw=self.h.runtime.create(self.h.request,self.h.region,self.h.authority,budget,
                sequence_deliveries=True,minimize_delivery_cost=True)
        self.assertIsNotNone(json.loads(raw)['review'])
        self.assertGreater(budget.work_steps,0)
        self.assertLess(budget.work_steps,self.h.runtime.EXPORT_WORK)

    def test_strict_limit_emits_no_review_while_nonstrict_preserves_full_incumbent(self):
        for strict in (False,True):
            out=self.h.create(require_optimal_delivery=strict,max_search_nodes=1)
            self.assertEqual(out['review'] is None,strict)
            self.assertFalse(out['result']['delivery_optimization']['cost_optimality_proven'])
            self.assertEqual(out['result']['delivery_optimization']['absolute_cost_gap'],2)
            if not strict:
                self.assertEqual(len(model.Review.decode(canonical(out['review']),out['review_digest']).json()['selections']),3)
            else:
                self.assertIsNone(out['review_digest'])
                self.assertIsNone(out['result']['plan'])

    def test_saved_review_check_never_optimizes_or_substitutes_missing_items(self):
        out=self.h.create(require_optimal_delivery=True)
        saved=model.Review.decode(canonical(out['review']),out['review_digest'])
        frozen=saved.raw
        removed=out['review']['selections'][0]['candidate']['item']
        old=self.h.observed.items.pop(removed)
        self.h.observed.items[99]=SimpleNamespace(**{**old.__dict__,'id':99})
        self.h.observed.digest=hashlib.sha256(b'changed-roster').hexdigest()
        with patch.object(optimal,'allocate_optimal_routes',side_effect=AssertionError('reoptimization')):
            with patch.object(joint,'allocate_joint_routes',side_effect=AssertionError('replacement')):
                checked=json.loads(self.h.runtime.check(saved,self.h.authority,Budget()))
        self.assertIsNone(checked['result']['plan'])
        self.assertFalse(checked['result']['reallocation_performed'])
        self.assertEqual(checked['result']['original_plan_digest'],out['result']['plan_digest'])
        self.assertEqual(saved.raw,frozen)
        self.assertNotIn(99,[row['item'] for row in checked['result']['rows']])
        self.assertIsNone(checked['agent_turn']['briefing'].get('delivery_cost_optimality_proven'))

    def test_original_basic_sequence_and_first_feasible_modes_still_work_unchanged(self):
        with patch.object(optimal,'allocate_optimal_routes',side_effect=AssertionError('unsolicited optimization')):
            for options in ({},{'sequence_deliveries':True},
                            {'sequence_deliveries':True,'search_delivery_items':True}):
                out=json.loads(self.h.runtime.create(self.h.request,self.h.region,self.h.authority,Budget(),**options))
                self.assertIsNotNone(out['review'])
                self.assertNotIn('delivery_optimization',out['result'])
                self.assertEqual('delivery_search' in out['result'],bool(options.get('search_delivery_items')))

    def test_ambiguous_modes_invalid_bounds_and_export_budget_refuse_before_contact(self):
        for options in ({'minimize_delivery_cost':True}, {'require_optimal_delivery':True},
            {'sequence_deliveries':True,'minimize_delivery_cost':True,'search_delivery_items':True},
            {'minimize_delivery_cost':1},{'require_optimal_delivery':1},
            {'sequence_deliveries':True,'minimize_delivery_cost':True,'max_search_nodes':4097}):
            with self.subTest(options=options),self.assertRaises(ValueError):
                self.h.runtime.create(self.h.request,self.h.region,self.h.authority,Budget(),**options)
        budget=Budget()
        budget.work_steps=self.h.runtime.EXPORT_WORK
        with self.assertRaises(ValueError):
            self.h.runtime.create(self.h.request,self.h.region,self.h.authority,budget,
                sequence_deliveries=True,minimize_delivery_cost=True)
        self.assertEqual(self.h.opened,0)

    def test_authority_revocation_after_review_encoding_prevents_publication(self):
        create=model.create
        def revoked(*args,**kwargs):
            saved=create(*args,**kwargs)
            os.environ['DFMCP_MAP_TOKEN']='changed'
            return saved
        with patch.object(model,'create',side_effect=revoked):
            with self.assertRaises(ValueError):
                self.h.create()
        self.assertEqual(self.h.captures,1)

    def test_check_cli_rejects_optimization_before_reading_original_file(self):
        for flags in (('--minimize-delivery-cost',),('--require-optimal-delivery',),
                      ('--sequence-deliveries','--minimize-delivery-cost'),
                      ('--delivery-search-nodes','1')):
            stdout=io.BytesIO()
            with patch.object(self.h.review_cli,'read_file',side_effect=AssertionError('input read')):
                with patch.object(self.h.review_cli.sys,'stdout',SimpleNamespace(buffer=stdout)):
                    code=self.h.review_cli.main(['check','--review-file','/private/not-read',
                        '--expected-review','0'*64,*flags])
            self.assertEqual(code,2)
            self.assertIsNone(json.loads(stdout.getvalue())['result'])
            self.assertNotIn(b'/private',stdout.getvalue())
        self.assertEqual(self.h.opened,0)

    def test_actual_cli_exports_and_reopens_optimized_review_with_unchanged_input_file(self):
        with tempfile.TemporaryDirectory() as directory:
            request=Path(directory)/'request.json'
            request.write_bytes(canonical(self.h.request.json()))
            stdout=io.BytesIO()
            with patch.object(self.h.review_cli.sys,'stdout',SimpleNamespace(buffer=stdout)):
                code=self.h.review_cli.main(self.h.review_args(request,
                    '--sequence-deliveries','--minimize-delivery-cost','--require-optimal-delivery'))
            self.assertEqual(code,0)
            packet=json.loads(stdout.getvalue())
            review=Path(directory)/'review.json'
            original=canonical(packet['review'])
            review.write_bytes(original)
            stdout=io.BytesIO()
            with patch.object(self.h.review_cli.sys,'stdout',SimpleNamespace(buffer=stdout)):
                code=self.h.review_cli.main(['check','--review-file',str(review),
                                           '--expected-review',packet['review_digest']])
            self.assertEqual(code,0)
            result=json.loads(stdout.getvalue())['result']
            self.assertEqual(result['status'],'constraints_met')
            self.assertEqual(result['plan'],packet['result']['plan'])
            self.assertEqual(review.read_bytes(),original)
            self.assertEqual(request.read_bytes(),canonical(self.h.request.json()))

    def test_review_cli_subprocess_executes_proven_optimal_and_strict_incomplete_modes(self):
        with tempfile.TemporaryDirectory() as directory:
            request=Path(directory)/'request.json'
            request.write_bytes(canonical(self.h.request.json()))
            for nodes in ('1','4096'):
                args=self.h.review_args(request,'--sequence-deliveries','--minimize-delivery-cost',
                    '--require-optimal-delivery','--delivery-search-nodes',nodes)
                child=subprocess.run([sys.executable,'-W','error',str(Path(__file__).resolve()),'--fixture-cli',*args],
                    capture_output=True,timeout=15,env={**os.environ,'PYTHONDONTWRITEBYTECODE':'1'})
                self.assertEqual(child.returncode,0,child.stderr.decode())
                self.assertFalse(child.stderr)
                packet=json.loads(child.stdout)
                self.assertEqual(packet['review'] is None,nodes=='1')
                self.assertEqual(packet['result']['delivery_optimization']['cost_optimality_proven'],nodes=='4096')


if __name__=='__main__':
    if '--fixture-cli' in sys.argv:
        args=sys.argv[sys.argv.index('--fixture-cli')+1:]
        with ReviewHarness() as harness:
            raise SystemExit(harness.review_cli.main(args))
    unittest.main()
