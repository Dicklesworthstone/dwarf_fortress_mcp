"""Actual allocation runtime/CLI with injected native captures and roster decoding.

Real solver, map decoder, candidate projection, request files and publication;
NOT a native transport, full-roster decoder, DFHack or live-game test.
"""
from __future__ import annotations

from contextlib import ExitStack
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import ModuleType, SimpleNamespace
import unittest
from unittest.mock import patch

from furniture_plan import canonical, require
from test_route_furniture_optimal import detour_case
import route_furniture_allocation as routes
from route_furniture_sequence import allocate_sequenced_routes
import route_furniture_optimal as optimal

ROOT = Path(__file__).resolve().parent
TOKEN = 'synthetic-read-token-' + 'x' * 32


class Budget:
    """Injected finite allowance; no test clocks or native work are represented."""
    def __init__(self, timeout_ms=10000):
        require(type(timeout_ms) is int and 1 <= timeout_ms <= 60000, 'invalid timeout')
        self.work_steps, self.disk_bytes = 20_000_000, 1 << 30
        self.calls, self.network_bytes = 272, 20 << 20
        self.valid = True
    def remaining(self):
        require(self.valid, 'deadline expired')
        return 10.0
    def work(self):
        self.remaining()
        require(self.work_steps > 0, 'work exhausted')
        self.work_steps -= 1
    def reserve(self, name, count):
        self.remaining()
        require(count >= 0 and count <= getattr(self, name), 'allowance exhausted')
        setattr(self, name, getattr(self, name) - count)


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


class Harness:
    """Load complete production modules, replacing only their native boundary."""
    def __enter__(self):
        self.stack = ExitStack()
        self.stack.enter_context(patch.dict(sys.modules))
        self.stack.enter_context(patch.dict(os.environ, {
            'DFMCP_ALLOW_UNADMITTED_ROUTE_FURNITURE': '1',
            'DFMCP_ROUTE_FURNITURE_ENDPOINT': '127.0.0.1:5000',
            'DFMCP_MAP_TOKEN': TOKEN,
            'DFMCP_OPERATIONS_PAGED_TOKEN': TOKEN,
        }, clear=True))
        wire = ModuleType('construction_monitor_rpc')
        wire.Client, wire.Budget = object, Budget
        wire.OPERATIONS_TOKEN = 'DFMCP_OPERATIONS_PAGED_TOKEN'
        wire.PROFILES = {'operations': ('dfmcp_operations_v1_4', 'dfmcp.operations.v1_4', 4)}
        wire.endpoint = lambda text: ('127.0.0.1', 5000)
        receipt = ModuleType('construction_receipt')
        receipt.Manifest = SimpleNamespace
        receipt.decode_operations = lambda *_: self.observed
        sys.modules.update(construction_monitor_rpc=wire, construction_receipt=receipt)
        self.inventory = load('_dfmcp_optimal_inventory_test', ROOT / 'route_furniture_inventory.py')
        sys.modules['route_furniture_inventory'] = self.inventory
        self.cli = load('_dfmcp_optimal_cli_test', ROOT / 'allocate_route_furniture.py')
        self.request, self.items, self.raw = detour_case()
        self.region = self.inventory.Region((1, 1, 1), (4, 4, 1))
        self.authority = self.inventory.Authority.load()
        self.observed = SimpleNamespace(folder='fort', site=7, tick=10, paused=True,
            horizons=(100,100,100), attachments=(), jobs={}, buildings={},
            digest=hashlib.sha256(b'fixture-roster').hexdigest(), items={})
        for candidate in self.items:
            self.observed.items[candidate.id] = SimpleNamespace(id=candidate.id,
                kind=candidate.kind.upper(), flags=64, container=None, holder=None, stack=1,
                material=candidate.material[0], material_index=candidate.material[1],
                position=candidate.position, subtype=candidate.subtype,
                native_type={'bed':1,'chair':2,'table':3}[candidate.kind])
        self.opened = self.captures = 0
        self.last = None
        harness = self
        class Source:
            def __init__(self, authority, region, budget):
                harness.opened += 1
                self.closed, self.budget = False, budget
                harness.last = self
                manifest = SimpleNamespace(generation=7, software=('df','dfhack'),
                                           df_version='df',dfhack_version='dfhack')
                self.manifests = {'map':manifest,'operations':manifest}
            def capture_once(self):
                harness.captures += 1
                self.closed = True
                manifest = SimpleNamespace(generation=7, software=('df','dfhack'),
                                           df_version='df',dfhack_version='dfhack')
                return manifest, harness.raw, manifest, b'fixture-roster'
            def __enter__(self):
                return self
            def __exit__(self,*_):
                self.closed = True
        self.inventory.Client = Source
        return self
    def __exit__(self,*args):
        return self.stack.__exit__(*args)
    def run(self, **options):
        return self.inventory.run(self.request,self.region,self.authority,Budget(),**options)
    def argv(self, path, *options):
        return ['--request-file',str(path),'--origin','1','1','1','--size','4','4','1',*options]


class OptimalRuntimeTests(unittest.TestCase):
    def setUp(self):
        self.h = self.enterContext(Harness())

    def test_complete_runtime_selects_optimizer_after_source_close_and_reserves_once(self):
        original = optimal.allocate_optimal_routes
        observed = []
        budget = Budget()
        def checked(*args, **kwargs):
            self.assertTrue(self.h.last.closed)
            self.assertEqual(budget.work_steps, 0)
            observed.append(kwargs['maximum_work'])
            return original(*args, **kwargs)
        with patch.object(optimal,'allocate_optimal_routes',side_effect=checked):
            out=json.loads(self.h.inventory.run(self.h.request,self.h.region,self.h.authority,budget,
                sequence_deliveries=True,minimize_delivery_cost=True,require_optimal_delivery=True))
        self.assertEqual((self.h.opened,self.h.captures),(1,1))
        self.assertEqual(out['result']['delivery_optimization']['best_complete_cost'],7)
        self.assertEqual(out['source']['solver_work_reserved'],observed[0])
        self.assertLess(observed[0],routes.MAX_WORK)
        self.assertTrue(out['agent_turn']['briefing']['delivery_cost_optimality_proven'])
        self.assertEqual(out['agent_turn']['briefing']['delivery_cost_gap'],0)
        self.assertEqual(out['source']['pure_allocation_digest'],out['result']['decision_digest'])
        self.assertFalse(out['runtime_admitted'])

    def test_original_modes_do_not_call_optimizer_and_keep_original_pure_results(self):
        with patch.object(optimal,'allocate_optimal_routes',side_effect=AssertionError('unexpected optimizer')):
            for sequencing in (False,True):
                out=json.loads(self.h.run(sequence_deliveries=sequencing))
                allowance=out['source']['solver_work_reserved']
                expected=(allocate_sequenced_routes(self.h.request,self.h.items,self.h.raw,
                          maximum_work=allowance)[1] if sequencing else
                          routes.allocate_routes(self.h.request,self.h.items,self.h.raw,maximum_work=allowance))
                self.assertEqual(out['result'],expected)
                self.assertNotIn('delivery_cost_optimality_proven',out['agent_turn']['briefing'])

    def test_node_limit_preserves_full_incumbent_unless_strict_export_requested(self):
        for strict in (False,True):
            out=json.loads(self.h.run(sequence_deliveries=True,minimize_delivery_cost=True,
                require_optimal_delivery=strict,max_search_nodes=1))
            proof=out['result']['delivery_optimization']
            self.assertFalse(proof['cost_optimality_proven'])
            self.assertEqual(proof['absolute_cost_gap'],2)
            self.assertEqual(out['result']['plan'] is None,strict)
            self.assertEqual(out['result']['status'],'delivery_optimization_incomplete' if strict else 'allocated')
            self.assertTrue(out['ok'])  # Established report is not an optimal plan.

    def test_invalid_opt_ins_and_limits_refuse_before_contact(self):
        for options in ({'minimize_delivery_cost':True}, {'require_optimal_delivery':True},
            {'sequence_deliveries':1}, {'minimize_delivery_cost':1}, {'require_optimal_delivery':1},
            {'max_search_nodes':1}, {'max_search_nodes':0}, {'max_search_nodes':4097},
            {'max_search_nodes':True}, {'max_search_nodes':None}):
            with self.subTest(options=options),self.assertRaises(ValueError):
                self.h.run(**options)
        self.assertEqual(self.h.opened,0)

    def test_changed_source_refuses_before_optimizer(self):
        for field,value in (('tick',11),('paused',False),('folder','other'),('site',8)):
            original=getattr(self.h.observed,field)
            setattr(self.h.observed,field,value)
            with patch.object(optimal,'allocate_optimal_routes',side_effect=AssertionError('source bypass')):
                with self.assertRaises(ValueError):
                    self.h.run(sequence_deliveries=True,minimize_delivery_cost=True)
            setattr(self.h.observed,field,original)

    def test_complete_candidate_projection_precedes_selection(self):
        self.h.observed.items[2].flags=65
        out=json.loads(self.h.run(sequence_deliveries=True,minimize_delivery_cost=True))
        self.assertNotIn(2,[row['item'] for row in out['result']['assignments']])
        self.assertEqual(out['source']['projection']['counts']['projected_flags'],1)
        self.assertEqual(out['result']['candidate_count'],3)

    def test_revocation_during_solver_and_final_serialization_never_returns_result(self):
        original=optimal.allocate_optimal_routes
        def revoked(*args,**kwargs):
            os.environ['DFMCP_MAP_TOKEN']='changed'
            return original(*args,**kwargs)
        with patch.object(optimal,'allocate_optimal_routes',side_effect=revoked):
            with self.assertRaises(ValueError):
                self.h.run(sequence_deliveries=True,minimize_delivery_cost=True)
        os.environ['DFMCP_MAP_TOKEN']=TOKEN
        output=self.h.inventory.bounded_output
        def render(value):
            raw=output(value)
            os.environ['DFMCP_MAP_TOKEN']='changed'
            return raw
        with patch.object(self.h.inventory,'bounded_output',side_effect=render):
            with self.assertRaises(ValueError):
                self.h.run(sequence_deliveries=True,minimize_delivery_cost=True)
        self.assertEqual(self.h.captures,2)

    def test_work_exhaustion_does_not_renew_allowance_or_publish_incumbent(self):
        budget=Budget()
        budget.work_steps=80
        with self.assertRaises(ValueError):
            self.h.inventory.run(self.h.request,self.h.region,self.h.authority,budget,
                sequence_deliveries=True,minimize_delivery_cost=True)
        self.assertEqual(budget.work_steps,0)
        self.assertEqual(self.h.captures,1)

    def test_cli_reads_real_request_and_routes_strict_options_to_real_solver(self):
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'request.json'
            path.write_bytes(canonical(self.h.request.json()))
            stdout=io.BytesIO()
            with patch.object(self.h.cli.sys,'stdout',SimpleNamespace(buffer=stdout)):
                code=self.h.cli.main(self.h.argv(path,'--sequence-deliveries','--minimize-delivery-cost',
                    '--require-optimal-delivery','--delivery-search-nodes','4096'))
            self.assertEqual(code,0)
            out=json.loads(stdout.getvalue())
            self.assertTrue(out['result']['delivery_optimization']['cost_optimality_proven'])
            self.assertEqual(out['result']['delivery_optimization']['best_complete_cost'],7)
            self.assertEqual(path.read_bytes(),canonical(self.h.request.json()))

    def test_cli_invalid_flags_do_not_read_input_or_contact_source(self):
        for flags in (('--minimize-delivery-cost',),('--require-optimal-delivery',),
                      ('--delivery-search-nodes','1')):
            stdout=io.BytesIO()
            with patch.object(self.h.cli,'read_request',side_effect=AssertionError('unexpected file read')):
                with patch.object(self.h.cli.sys,'stdout',SimpleNamespace(buffer=stdout)):
                    code=self.h.cli.main(self.h.argv('/private/do-not-disclose',*flags))
            self.assertEqual(code,2)
            self.assertIsNone(json.loads(stdout.getvalue())['result'])
            self.assertNotIn(b'/private',stdout.getvalue())
        self.assertEqual(self.h.opened,0)

    def test_short_stdout_attempt_is_not_repeated(self):
        class Short:
            calls=0
            def write(self,raw):
                self.calls+=1
                return len(raw)-1
            def flush(self):
                raise AssertionError('short write must not flush')
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'r.json'
            path.write_bytes(canonical(self.h.request.json()))
            stdout=Short()
            with patch.object(self.h.cli.sys,'stdout',SimpleNamespace(buffer=stdout)):
                code=self.h.cli.main(self.h.argv(path,'--sequence-deliveries','--minimize-delivery-cost'))
            self.assertEqual(code,2)
            self.assertEqual(stdout.calls,1)
            self.assertEqual(self.h.captures,1)

    def test_real_subprocess_cli_main_is_deterministic_with_injected_capture_boundary(self):
        outputs=[]
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'r.json'
            path.write_bytes(canonical(self.h.request.json()))
            for seed in ('0','1','79'):
                command=[sys.executable,'-W','error',str(Path(__file__).resolve()),'--fixture-cli',
                    *self.h.argv(path,'--sequence-deliveries','--minimize-delivery-cost','--require-optimal-delivery')]
                child=subprocess.run(command,capture_output=True,timeout=15,
                    env={**os.environ,'PYTHONHASHSEED':seed,'PYTHONDONTWRITEBYTECODE':'1'})
                self.assertEqual(child.returncode,0,child.stderr.decode())
                self.assertFalse(child.stderr)
                outputs.append(child.stdout)
            self.assertEqual(outputs[0],outputs[1])
            self.assertEqual(outputs[1],outputs[2])
            self.assertEqual(json.loads(outputs[0])['result']['delivery_optimization']['best_complete_cost'],7)


if __name__=='__main__':
    if '--fixture-cli' in sys.argv:
        args=sys.argv[sys.argv.index('--fixture-cli')+1:]
        with Harness() as harness:
            raise SystemExit(harness.cli.main(args))
    unittest.main()
