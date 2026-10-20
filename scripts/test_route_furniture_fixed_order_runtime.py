"""Full command/runtime modules, real solver and review codec, injected transport.

Acquisition, operations decoding, transport budget and endpoint parsing are
explicit doubles; no native TCP, real game, or complete room lifecycle is tested.
Legacy search modules are trap stubs in this focused new-mode suite.
"""
from contextlib import ExitStack
from dataclasses import dataclass
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from types import ModuleType, SimpleNamespace
import unittest
from unittest.mock import patch

from furniture_allocation import Request, Slot
from furniture_plan import canonical, integer, require
import route_furniture_fixed_order as fixed
import test_route_furniture_fixed_order as core
from test_route_furniture_fixed_order import item, raw_map

ROOT = Path(__file__).resolve().parent


def forbidden(*_args, **_kwargs):
    raise AssertionError('unexpected transport or legacy-search call')


class Budget:
    """Test budget, not execution evidence for the native budget implementation."""
    def __init__(self, milliseconds=60000):
        integer(milliseconds, 1, 60000)
        self.deadline = time.monotonic() + milliseconds / 1000
        self.work_steps = 20_000_000
        self.calls, self.network_bytes, self.disk_bytes = 320, 32*1024*1024, 1024*1024*1024
    def remaining(self):
        left = self.deadline - time.monotonic()
        require(left > 0, 'test deadline exhausted')
        return left
    def work(self):
        self.remaining()
        require(self.work_steps > 0, 'test work exhausted')
        self.work_steps -= 1
    def reserve(self, kind, count):
        self.remaining()
        require(0 <= count <= getattr(self, kind), 'test reservation exhausted')
        setattr(self, kind, getattr(self, kind)-count)


@dataclass(frozen=True)
class Manifest:
    generation: int
    df_version: str = 'df'
    dfhack_version: str = 'dfhack'
    @property
    def software(self):
        return self.df_version, self.dfhack_version


def load_stack(directory=ROOT):
    """Import COMPLETE source files in a scoped namespace; no AST extraction."""
    wire = ModuleType('construction_monitor_rpc')
    wire.Budget, wire.Client = Budget, object
    wire.OPERATIONS_TOKEN = 'DFMCP_OPERATIONS_PAGED_TOKEN'
    wire.PROFILES = {'operations': ('test', 'test', 4)}
    wire.endpoint = lambda s: (s.split(':')[0], int(s.split(':')[1]))
    receipt = ModuleType('construction_receipt')
    receipt.Manifest, receipt.decode_operations = Manifest, forbidden
    old_optimal, old_joint = ModuleType('route_furniture_optimal'), ModuleType('route_furniture_joint')
    old_optimal.MAX_SEARCH_NODES = old_joint.MAX_SEARCH_NODES = 4096
    old_optimal.allocate_optimal_routes = old_joint.allocate_joint_routes = forbidden
    values = {'construction_monitor_rpc':wire, 'construction_receipt':receipt,
              'route_furniture_optimal':old_optimal, 'route_furniture_joint':old_joint}
    loaded = {}
    with patch.dict(sys.modules, values):
        for name in ('route_furniture_review', 'route_furniture_inventory',
                     'route_furniture_review_runtime', 'allocate_route_furniture', 'review_route_furniture'):
            path = (ROOT if name == 'route_furniture_review' else Path(directory)) / (name+'.py')
            alias = '_fixed_order_test_' + name
            spec = importlib.util.spec_from_file_location(alias, path)
            module = importlib.util.module_from_spec(spec)
            sys.modules[alias] = module
            spec.loader.exec_module(module)
            sys.modules[name] = module
            loaded[name] = module
    return SimpleNamespace(model=loaded['route_furniture_review'],
        inventory=loaded['route_furniture_inventory'], runtime=loaded['route_furniture_review_runtime'],
        allocate_cli=loaded['allocate_route_furniture'], review_cli=loaded['review_route_furniture'])


class Fixture:
    def __init__(self, stack=None, request=None, candidates=None, terrain=None):
        self.stack = load_stack() if stack is None else stack
        r, cs, raw = core.FixedOrderTests().example()
        self.request = r if request is None else request
        self.candidates = cs if candidates is None else candidates
        self.raw = raw if terrain is None else terrain
        region = self.stack.inventory.routes.decode_terrain(self.raw)
        self.region = self.stack.inventory.Region(region.origin, region.size)
        self.observed = SimpleNamespace(folder='fort', site=7, paused=True, tick=3,
            horizons=(100,100,100000), digest=hashlib.sha256(b'operations-fixture').hexdigest(),
            jobs={}, buildings={}, attachments=(), items={c.id:SimpleNamespace(
                id=c.id, kind=c.kind.upper(), flags=64, container=None, holder=None, stack=1,
                material=c.material[0], material_index=c.material[1], position=c.position,
                subtype=c.subtype, native_type={'bed':1,'chair':2,'table':3}[c.kind]) for c in self.candidates})
        self.map_manifest, self.ops_manifest = Manifest(11), Manifest(17)
        self.calls, self.sources = 0, []
        self.failure = None
    def __enter__(self):
        self.scope = ExitStack()
        inv = self.stack.inventory
        environment = {k:v for k,v in os.environ.items() if not k.startswith('DFMCP_')}
        environment.update({inv.OPT_IN:'1', inv.ENDPOINT:'127.0.0.1:5000',
                            inv.MAP_TOKEN:'m'*32, inv.wire.OPERATIONS_TOKEN:'o'*32})
        self.scope.enter_context(patch.dict(os.environ, environment, clear=True))
        fixture = self
        class Client:
            def __init__(self, authority, region, budget):
                authority.guard()
                budget.remaining()
                fixture.calls += 1
                self.closed = False
                self.manifests = {'map':fixture.map_manifest, 'operations':fixture.ops_manifest}
                fixture.sources.append(self)
            def __enter__(self):
                return self
            def __exit__(self, *_args):
                self.closed = True
            def capture_once(self):
                if fixture.failure:
                    raise fixture.failure
                self.closed = True
                return fixture.map_manifest, fixture.raw, fixture.ops_manifest, b'operations-fixture'
        self.scope.enter_context(patch.object(inv, 'Client', Client))
        self.scope.enter_context(patch.object(inv, 'decode_operations', lambda raw, guard: self.observed))
        self.authority = inv.Authority.load()
        return self
    def __exit__(self, *args):
        return self.scope.__exit__(*args)
    def allocate(self, **options):
        return self.stack.inventory.run(self.request, self.region, self.authority, Budget(), **options)
    def create(self, **options):
        return self.stack.runtime.create(self.request, self.region, self.authority, Budget(), **options)


class Output:
    def __init__(self, short=False):
        self.buffer, self.calls, self.short = self, [], short
    def write(self, raw):
        self.calls.append(raw)
        return len(raw)-1 if self.short else len(raw)
    def flush(self):
        pass


class RuntimeTests(unittest.TestCase):
    def test_allocation_uses_one_closed_source_and_preserves_request(self):
        with Fixture() as f:
            raw = f.allocate(fixed_delivery_order=True)
            value = json.loads(raw)
            self.assertEqual(f.calls, 1)
            self.assertTrue(all(c.closed for c in f.sources))
            self.assertEqual(value['request'], f.request.json())
            self.assertEqual([r['item'] for r in value['result']['assignments']], [1,3])
            self.assertTrue(value['agent_turn']['briefing']['fixed_order_assignment_optimality_proven'])
            self.assertFalse(value['runtime_admitted'])

    def test_review_round_trip_freezes_exact_new_items_and_original_dependencies(self):
        with Fixture() as f:
            value = json.loads(f.create(fixed_delivery_order=True))
            original = canonical(f.request.json())
            review = f.stack.model.Review.decode(canonical(value['review']), value['review_digest'])
            _, request, _, plan = review.unpack()
            self.assertEqual(canonical(request.json()), original)
            self.assertEqual([s.item for s in plan.steps], [1,3])
            self.assertTrue(all(not s.after for s in plan.steps))
            self.assertIn('fixed_order_assignment_optimality_proven', value['agent_turn']['coverage'])
            self.assertFalse(any('preserves selected items' in s for s in value['agent_turn']['uncertainty']))

    def test_blocked_fixed_order_exports_no_review_or_partial_plan(self):
        _, cs, _ = core.FixedOrderTests().example()
        with Fixture(candidates=cs[:2]) as f:
            value = json.loads(f.create(fixed_delivery_order=True))
            self.assertTrue(value['ok'])
            self.assertEqual(value['result']['status'], 'delivery_order_shortage')
            for key in ('review','review_digest'):
                self.assertIsNone(value[key])
            self.assertIsNone(value['result']['plan'])
            self.assertEqual(value['result']['assignments'], [])

    def test_saved_check_never_replaces_a_missing_original_item(self):
        with Fixture() as f:
            value = json.loads(f.create(fixed_delivery_order=True))
            review = f.stack.model.Review.decode(canonical(value['review']),value['review_digest'])
            before = review.raw
            replacement = f.observed.items.pop(3)
            f.observed.items[4] = SimpleNamespace(**{**vars(replacement),'id':4})
            with patch.object(fixed, 'allocate_fixed_order_routes', side_effect=forbidden):
                checked = json.loads(f.stack.runtime.check(review,f.authority,Budget()))
            self.assertEqual(checked['result']['status'], 'unestablished')
            self.assertIsNone(checked['result']['plan'])
            self.assertEqual(checked['result']['rows'][1]['item'], 3)
            self.assertEqual(review.raw,before)

    def test_full_32_selection_review_stays_within_existing_bounds(self):
        r = Request('fort',7,tuple(Slot(f's{i:02}','bed',(i+1,2,0)) for i in range(32)))
        cs = tuple(item(i,(i+1,1,0)) for i in range(32))
        with Fixture(request=r,candidates=cs,terrain=raw_map((34,3,1),origin=(0,0,0))) as f:
            raw = f.create(fixed_delivery_order=True)
            value = json.loads(raw)
            self.assertLessEqual(len(raw),f.stack.inventory.MAX_OUTPUT)
            review = f.stack.model.Review.decode(canonical(value['review']),value['review_digest'])
            self.assertEqual(len(review.unpack()[3].steps),32)
            self.assertEqual(review.unpack()[1].json(),r.json())

    def test_api_conflicting_options_refuse_before_acquisition(self):
        for bad in (1,None,'true'):
            with Fixture() as f, self.assertRaises(ValueError):
                f.allocate(fixed_delivery_order=bad)
            self.assertEqual(f.calls,0)
        for method in ('allocate','create'):
            cases = ({'sequence_deliveries':True},{'minimize_delivery_cost':True},
                     {'require_optimal_delivery':True},{'max_search_nodes':1})
            if method=='create':
                cases += ({'search_delivery_items':True},)
            for options in cases:
                with Fixture() as f, self.assertRaises(ValueError):
                    getattr(f,method)(fixed_delivery_order=True,**options)
                self.assertEqual(f.calls,0)

    def test_api_source_changes_and_lost_capture_do_not_publish_or_retry(self):
        for mode in ('allocate','create'):
            for failure in ('tick','software','lost'):
                with Fixture() as f:
                    if failure=='tick':
                        f.observed.tick=4
                    elif failure=='software':
                        f.ops_manifest=Manifest(17,'different')
                    else:
                        f.failure=OSError('lost capture')
                    with self.assertRaises((ValueError,OSError)):
                        getattr(f,mode)(fixed_delivery_order=True)
                    self.assertEqual(f.calls,1)
                    self.assertTrue(all(c.closed for c in f.sources))

    def test_one_shrinking_solver_allowance_keeps_review_export_reservation(self):
        actual = fixed.allocate_fixed_order_routes
        for create in (False,True):
            with Fixture() as f:
                budget=Budget()
                seen=[]
                def inspect(*args,**kw):
                    self.assertTrue(all(c.closed for c in f.sources))
                    self.assertEqual(budget.work_steps, f.stack.runtime.EXPORT_WORK if create else 0)
                    seen.append(kw['maximum_work'])
                    return actual(*args,**kw)
                function=f.stack.runtime.create if create else f.stack.inventory.run
                with patch.object(fixed,'allocate_fixed_order_routes',side_effect=inspect):
                    function(f.request,f.region,f.authority,budget,fixed_delivery_order=True)
                self.assertEqual(len(seen),1)
                self.assertLess(seen[0],20_000_000)
                self.assertGreaterEqual(budget.work_steps,0)
        with Fixture() as f:
            budget=Budget();budget.work_steps=f.stack.runtime.EXPORT_WORK
            with self.assertRaises(ValueError):
                f.stack.runtime.create(f.request,f.region,f.authority,budget,fixed_delivery_order=True)
            self.assertEqual(f.calls,0)

    def test_authority_revocation_during_final_rendering_prevents_success(self):
        for method in ('allocate','create'):
            with Fixture() as f:
                output=f.stack.inventory.bounded_output
                def revoked(value):
                    raw=output(value)
                    os.environ.pop(f.stack.inventory.OPT_IN)
                    return raw
                with patch.object(f.stack.inventory,'bounded_output',side_effect=revoked):
                    with self.assertRaises(ValueError):
                        getattr(f,method)(fixed_delivery_order=True)
                self.assertEqual(f.calls,1)

    def cli(self,f,path,review=False,extra=()):
        common=['--request-file',str(path),'--origin',*map(str,f.region.origin),
                '--size',*map(str,f.region.size),'--fixed-delivery-order',*extra]
        module=f.stack.review_cli if review else f.stack.allocate_cli
        output=Output()
        with patch.object(module.sys,'stdout',output):
            code=module.main((['create'] if review else [])+common)
        return code,output

    def test_both_actual_cli_mains_read_immutable_files_and_export_complete_plans(self):
        with tempfile.TemporaryDirectory() as directory, Fixture() as f:
            path=Path(directory)/'request.json';path.write_bytes(canonical(f.request.json()))
            original=path.read_bytes()
            for review in (False,True):
                code,out=self.cli(f,path,review)
                self.assertEqual(code,0)
                self.assertEqual(len(out.calls),1)
                value=json.loads(out.calls[0]);self.assertTrue(value['ok'])
                self.assertEqual(value['request'],f.request.json())
                self.assertEqual([r['item'] for r in value['result']['assignments']],[1,3])
                self.assertEqual(path.read_bytes(),original)

    def test_cli_conflicts_and_saved_review_flag_are_pre_file_pre_native_refusals(self):
        for review in (False,True):
            for extra in (('--sequence-deliveries',),('--minimize-delivery-cost',),
                          ('--require-optimal-delivery',),('--delivery-search-nodes','4096')):
                with Fixture() as f:
                    module=f.stack.review_cli if review else f.stack.allocate_cli
                    reader='read_file' if review else 'read_request'
                    with patch.object(module,reader,side_effect=forbidden):
                        code,out=self.cli(f,'/private-do-not-disclose',review,extra)
                    self.assertEqual(code,2);self.assertEqual(f.calls,0)
                    self.assertEqual(len(out.calls),1)
                    self.assertNotIn(b'private-do-not-disclose',out.calls[0])
        with Fixture() as f:
            out=Output()
            with patch.object(f.stack.review_cli,'read_file',side_effect=forbidden), \
                 patch.object(f.stack.review_cli.sys,'stdout',out):
                code=f.stack.review_cli.main(['check','--review-file','/private',
                    '--expected-review','a'*64,'--fixed-delivery-order'])
            self.assertEqual(code,2);self.assertEqual(f.calls,0)

    def test_failed_or_short_output_has_one_attempt_and_never_reacquires(self):
        with tempfile.TemporaryDirectory() as directory:
            for review in (False,True):
                with Fixture() as f:
                    path=Path(directory)/'request';path.write_bytes(canonical(f.request.json()))
                    module=f.stack.review_cli if review else f.stack.allocate_cli
                    out=Output(short=True)
                    args=['--request-file',str(path),'--origin','1','1','0','--size','5','3','1',
                          '--fixed-delivery-order']
                    with patch.object(module.sys,'stdout',out):
                        code=module.main((['create'] if review else [])+args)
                    self.assertEqual(code,2);self.assertEqual(len(out.calls),1);self.assertEqual(f.calls,1)

    def test_legacy_basic_and_sequenced_modes_do_not_enter_fixed_order_solver(self):
        for method in ('allocate','create'):
            for options in ({},{'sequence_deliveries':True}):
                with Fixture() as f, patch.object(fixed,'allocate_fixed_order_routes',side_effect=forbidden):
                    value=json.loads(getattr(f,method)(**options))
                    self.assertTrue(value['ok'])
                    self.assertNotIn('fixed_order_delivery',value['result'])
                    self.assertNotIn('delivery_order_preserved',value['agent_turn']['briefing'])

    def test_cli_main_subprocesses_retain_the_new_mode(self):
        code=('from test_route_furniture_fixed_order_runtime import *; '
              't=RuntimeTests(); t.test_both_actual_cli_mains_read_immutable_files_and_export_complete_plans(); '
              'print("complete")')
        env=dict(os.environ,PYTHONPATH=str(ROOT),PYTHONDONTWRITEBYTECODE='1')
        result=subprocess.run([sys.executable,'-W','error','-c',code],env=env,
                              capture_output=True,text=True,timeout=20,check=True)
        self.assertEqual(result.stdout.strip(),'complete');self.assertFalse(result.stderr)


if __name__ == '__main__':
    unittest.main()
