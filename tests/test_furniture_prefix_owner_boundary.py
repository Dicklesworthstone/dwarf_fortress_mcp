"""Run the existing review/advance functions with injected lower-level doubles.

AST selection isolates these unmodified public functions from eager native and
room imports; it does not replace their source, guards or confirmation checks.
No real private-file or native-client qualification is claimed by this suite.
"""
import ast
from copy import deepcopy
from dataclasses import dataclass
import hashlib
import os
from pathlib import Path
import sys
from types import SimpleNamespace as NS
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import furniture_plan as model
import run_furniture_batch as runner
from test_furniture_prefix import advance as transition, resign
from test_run_furniture_batch import OwnerAPI


def actual_boundary(api):
    source = Path(os.environ.get('FURNITURE_PREFIX_TEST_SOURCE', ROOT / 'scripts' / 'furniture_batch.py'))
    tree = ast.parse(source.read_text(encoding='utf-8'), filename=str(source))
    names = {'select_next', 'review_seal', 'review', 'advance'}
    nodes = [node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name in names]
    if {n.name for n in nodes} != names or len(nodes) != len(names):
        raise AssertionError('complete original public boundary source required')
    original_batch = api.Batch
    source_manifest = NS(**api.state['source'])
    api.journals = {}
    api.registrations = []
    api.sequence = 0
    api.substitute = False
    api.regress = False

    def selection(step):
        return NS(kind={'bed': 1, 'chair': 2, 'table': 3}[step.kind], item=step.item,
                  x=step.target[0], y=step.target[1], z=step.target[2])

    def capture(selected):
        seq = api.sequence - int(api.regress)
        return NS(selection=selected, generation=source_manifest.generation,
            identity=(source_manifest.generation, 1, (128, 128, 4), 'fixture'),
            tick=100 + api.sequence, sequence=seq, next_building=300 + api.sequence,
            next_job=400 + api.sequence, blockers=api.blockers, view=lambda: {'fixture': True})

    @dataclass(frozen=True)
    class NativePlan:
        key: str
        before: object

        @property
        def digest(self):
            return bytes.fromhex(f'{100+self.before.selection.item:064x}')

    class Batch(original_batch):
        def __init__(self, path, budget, writable=False):
            super().__init__(path, budget)
            self.source = source_manifest
            self.value = {'site': 1, 'dimensions': [128, 128, 4], 'folder': 'fixture', 'first_tick': 100}
            self.room_handoff = self.terrain_handoff = None
            self.effects = NS(journals=api.journals, get=lambda key: api.journals[key], budget=budget)
            self.entries = [{'step': name} for name in api.registrations]

        @property
        def stopped(self):
            return api.state['stopped']

        def bind_before(self, value, manifest):
            if manifest != source_manifest or value.identity != (source_manifest.generation, 1, (128, 128, 4), 'fixture'):
                raise ValueError('synthetic source differs')

        def bind(self, plan, manifest):
            self.bind_before(plan.before, manifest)

        def register(self, step):
            self.entries.append({'step': step.name})
            api.registrations.append(step.name)
            api.state['steps'][api.state['placed']]['registered'] = True
            resign(api.state)

    class Client:
        def __init__(self, authority, budget, selected):
            self.selected = selected
            self.budget = budget
            for _ in range(7):
                budget.reserve_call()
            self.closed = False

        def __enter__(self):
            return self

        def __exit__(self, *_args):
            self.closed = True
            api.events.append(('source_closed',))

        def observe(self):
            self.budget.reserve_call()
            api.events.append(('read', self.selected.item))
            return NS(manifest=source_manifest, capture=capture(self.selected), unresolved=False, retained_records=0)

    def start(owner, authority, selected, key, expected_plan, guard):
        # Lower-level native calls/journal writes are deliberately injected.
        for _ in range(11):
            owner.budget.reserve_call()
        plan = NativePlan('replacement-key' if api.substitute else key, capture(selected))
        known = NS(manifest=source_manifest, unresolved=False, retained_records=0)
        before = deepcopy(api.state)
        guard(plan, known, 'before_intent')
        api.events.append(('intent', key))
        api.state = transition(before, api.keys, 'unknown')
        api.state['steps'][before['placed']]['registered'] = False
        resign(api.state)
        journal = NS(raw=b'original intent', state=NS(plan=plan, terminal=None))
        owner.journals[key] = journal
        guard(plan, known, 'before_prepare')
        api.events.append(('prepare', key))
        guard(plan, known, 'before_commit')
        api.events.append(('commit', key))
        api.state = transition(before, api.keys, api.phase)
        api.sequence += 1
        journal.raw += b' retained outcome'
        journal.state.terminal = None if api.phase == 'unknown' else NS(after=capture(selected))
        api.hook('native_outcome')

    namespace = {'Batch': Batch, 'model': model, 'require': model.require, 'canonical': model.canonical,
        'sha': lambda raw: hashlib.sha256(raw).hexdigest(), 'Authority': api.Authority, 'Client': Client,
        'selection': selection, 'Plan': NativePlan, 'placement': NS(start=start),
        'encoded': api.encoded, 'output_limit': api.output_limit,
        'exact_hex': lambda value, size: bytes.fromhex(value) if len(value) == size*2 else (_ for _ in ()).throw(ValueError())}
    future = ast.ImportFrom(module='__future__', names=[ast.alias(name='annotations')], level=0)
    selected_tree = ast.fix_missing_locations(ast.Module(body=[future, *nodes], type_ignores=[]))
    exec(compile(selected_tree, str(source), 'exec'), namespace)
    api.Batch = Batch
    api.review, api.advance = namespace['review'], namespace['advance']
    return api


class OriginalOwnerBoundaryTests(unittest.TestCase):
    def execute(self, api, steps):
        reviewed = runner.review('/original', api.state['batch_id'], runner.Budget(60000, steps), _api=api)
        return runner.run('/original', api.state['batch_id'], reviewed['prefix_review']['confirm_run'],
                          runner.Budget(60000, steps), _api=api)

    def test_existing_public_functions_place_exact_dependency_prefix(self):
        api = actual_boundary(OwnerAPI(4))
        result = self.execute(api, 4)
        self.assertEqual(result['placed'], 4)
        self.assertEqual(result['prefix_run']['stop_reason'], 'all_placed')
        self.assertEqual(api.registrations, ['s00', 's01', 's02', 's03'])
        self.assertEqual(len([e for e in api.events if e[0] == 'source_closed']), 4)
        self.assertEqual(len([e for e in api.events if e[0] == 'commit']), 4)

    def test_existing_review_blockers_prevent_any_native_start(self):
        api = actual_boundary(OwnerAPI(2))
        api.blockers = ['occupied_target']
        result = self.execute(api, 2)
        self.assertEqual(result['prefix_run']['stop_reason'], 'preflight_blocked')
        self.assertEqual(api.journals, {})

    def test_existing_guard_rejects_native_key_substitution_before_intent(self):
        api = actual_boundary(OwnerAPI(2))
        api.substitute = True
        with self.assertRaisesRegex(ValueError, 'substituted batch step'):
            self.execute(api, 2)
        self.assertEqual(api.journals, {})

    def test_existing_guard_retains_predecessor_clock_floor(self):
        api = actual_boundary(OwnerAPI(2))
        api.hooks['native_outcome'] = lambda: setattr(api, 'regress', True)
        with self.assertRaisesRegex(ValueError, 'regressed behind predecessor'):
            self.execute(api, 2)
        self.assertEqual(len(api.journals), 1)
        self.assertEqual(len([e for e in api.events if e[0] == 'commit']), 1)

    def test_existing_public_path_stops_at_retained_uncertainty(self):
        api = actual_boundary(OwnerAPI(2))
        api.phase = 'indeterminate'
        result = self.execute(api, 2)
        self.assertEqual(result['prefix_run']['stop_reason'], 'indeterminate')
        self.assertEqual(len([e for e in api.events if e[0] == 'commit']), 1)
        self.assertEqual(result['pending_step'], 's00')

    def test_existing_public_path_does_not_publish_after_authority_loss(self):
        api = actual_boundary(OwnerAPI(2))
        api.hooks['native_outcome'] = lambda: setattr(api, 'authorized', False)
        with self.assertRaises(ValueError):
            self.execute(api, 2)
        self.assertEqual(len([e for e in api.events if e[0] == 'commit']), 1)
        self.assertEqual(api.active, 0)


if __name__ == '__main__':
    unittest.main()
