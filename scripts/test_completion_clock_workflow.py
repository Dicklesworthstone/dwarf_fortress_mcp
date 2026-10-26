"""Actual completion-clock owner with explicit original-custody/native ports.

Scheduling and scope/context code execute unchanged. Monitor/batch replay,
final native acquisition, native clock I/O and scope/receipt files are doubles;
this is workflow evidence, not native TCP or private-file qualification.
"""
from contextlib import contextmanager
from copy import deepcopy
import importlib.util
import io
from pathlib import Path
import runpy
import struct
import sys
import time
import types
import unittest
from unittest.mock import patch

from test_completion_clock_scope import modules, scope_value, canonical, require, integer, digest_text

ROOT = Path(__file__).resolve().parent


@contextmanager
def environment():
    with modules() as (context, model):
        fixture = types.SimpleNamespace()
        fixture.value = scope_value(model)
        fixture.events = []
        fixture.clock_tick, fixture.clock_sequence = 10, 0
        fixture.status = 'construction_quiescent'
        fixture.missing_history = False
        fixture.fail_commit = False
        fixture.after_prepare = lambda: None
        fixture.after_commit = lambda: None
        fixture.after_final = lambda: None
        fixture.render_padding = ''
        fixture.existing = set()
        fixture.opens = []
        fixture.reads = 0

        class Budget:
            def __init__(self, timeout_ms=10000):
                integer(timeout_ms, 1, 60000)
                self.deadline = time.monotonic() + timeout_ms / 1000
                self.calls, self.network_bytes = 560, 40 * 1024 * 1024
                self.work_steps, self.disk_bytes = 20_000_000, 1024 * 1024 * 1024
            def remaining(self):
                result = self.deadline - time.monotonic()
                require(result > 0, 'deadline'); return result
            def work(self):
                self.remaining(); self.work_steps -= 1
                require(self.work_steps >= 0, 'work exhausted')
            def reserve(self, name, count):
                self.remaining(); require(count <= getattr(self, name), 'allowance')
                setattr(self, name, getattr(self, name) - count)

        class Authority:
            address = ('127.0.0.1', 5000)
            run_token = b'r' * 32
            revoked = False
            def clock_guard(self): require(not self.revoked, 'revoked')
            def guard(self, *_): self.clock_guard()
            @classmethod
            def load(cls): return fixture.authority

        class Batch:
            def __init__(self, path='/private/batch', expected=None, budget=None, *, writable=False):
                fixture.opens.append('batch')
                self.path, self.budget, self.writable = path, budget or Budget(), writable
                v = fixture.value
                self.id = v['batch_id']
                self.address = Authority.address
                self.review = types.SimpleNamespace(digest=v['review_digest'])
                self.plan = types.SimpleNamespace(digest=v['plan_digest'], steps=(1,))
                self.source = context.Source.from_json(v['source'])
                self.closed = False
                self.inventory = v['inventory_digest']
            def audit(self):
                self.check()
                return dict(inventory_verified=True, batch_id=self.id, review_digest=self.review.digest,
                            inventory_digest=self.inventory, status='all_placed', next_step=None,
                            pending_step=None, stopped=False, placed=len(self.plan.steps), steps=[])
            def check(self): require(not self.closed, 'batch closed'); self.budget.work()
            def close(self): self.closed = True

        class Journal:
            def __init__(self, path='/private/completion', budget=None, *, writable=False):
                fixture.opens.append('journal')
                self.path, self.budget, self.writable = path, budget or fixture.batch.budget, writable
                w = fixture.value['completion']
                condition = types.SimpleNamespace(**{name: w[name] for name in (
                    'deadline', 'interval', 'stable_samples', 'stable_span', 'max_gap', 'max_observations')})
                goal = types.SimpleNamespace(digest=w['goal_digest'], condition=condition,
                                             receipts=(b'original-receipt',), batch_id=fixture.batch.id)
                progress = types.SimpleNamespace(goal_digest=w['goal_digest'], terminal=False, reading=False,
                    phase='candidate', **{name: w[name] for name in (
                        'observations', 'streak', 'first_tick', 'counted_tick', 'last_tick', 'last_capture')},
                    assessments=(types.SimpleNamespace(condition=types.SimpleNamespace(status='condition_met')),))
                self.state = types.SimpleNamespace(goal=goal, progress=progress,
                    address=Authority.address, tail=bytes.fromhex(w['journal_head']), frames=w['journal_frames'])
                self.closed = False
            def check(self): require(not self.closed, 'monitor closed'); self.budget.work()
            def close(self): self.closed = True
            def accept(self, *_): raise AssertionError('clock must never fabricate a completion sample')
            def start_read(self): raise AssertionError('clock must never append monitor history')

        def verify_origin(goal, batch):
            require(goal.batch_id == batch.id, 'foreign original goal'); batch.check()

        def final_read(batch, authority):
            authority.clock_guard(); batch.check()
            fixture.events.append('final-read'); fixture.reads += 1
            source = deepcopy(fixture.value['source'])
            source['tick'] = fixture.clock_tick
            state = batch.audit()
            report = dict(batch_id=batch.id, phase='final_construction', status=fixture.status,
                          source=source, evidence_digest=fixture.value['frontier_evidence'])
            state['route_frontier'] = report
            fixture.after_final()
            return state, report

        class Client:
            def __init__(self, authority, budget, guard, *, intent=None, query=False, effect_guard=None):
                self.authority, self.budget, self.guard = authority, budget, guard
                self.intent, self.effect = intent, effect_guard
                self.commit_attempted = False
                self.guard(); authority.clock_guard()
                fixture.events.append('clock-connect')
            def __enter__(self): return self
            def __exit__(self, *_): fixture.events.append('clock-close')
            def call(self, operation, fields=None):
                self.guard(); self.authority.clock_guard(); self.budget.reserve('calls', 1)
                if operation in ('PrepareRun', 'CommitRun'):
                    self.effect(self.intent)
                fixture.events.append(operation)
                if operation == 'ObserveRun':
                    return dict(owner_active=False, retained_records=len(fixture.existing),
                        manifest=deepcopy(fixture.value['clock_manifest']),
                        observation_hex=(b'DFMRO013' + struct.pack('>QQQBBB', 11,
                            fixture.clock_sequence, fixture.clock_tick, 1, 1, 1)).hex())
                if operation == 'PrepareRun': fixture.after_prepare()
                if operation == 'CommitRun':
                    self.commit_attempted = True
                    fixture.clock_sequence += 1
                    fixture.clock_tick += self.intent['game_ticks']
                    fixture.after_commit()
                    if fixture.fail_commit: raise OSError('lost reply')
                self.guard(); self.authority.clock_guard()
                return dict(record=dict(phase='stopped', pause_verified=True,
                    observed_tick=fixture.clock_tick, key=self.intent['idempotency_key']),
                    manifest=deepcopy(fixture.value['clock_manifest']))

        @contextmanager
        def new_run(path):
            require(path not in fixture.existing, 'original record already used')
            fixture.events.append('new-run')
            yield 1
            fixture.events.append('new-run-close')

        class History:
            digest = '9' * 64
            def __init__(self, directory, batch, **_): self.batch = batch
            def __enter__(self): fixture.events.append('history-open'); return self
            def __exit__(self, *_): fixture.events.append('history-close')
            def admit_new(self, scope):
                require(not fixture.missing_history, 'unresolved previous clock')
                require(context._is_scope(scope), 'closed scope required')
                scope.bind(self.batch, current=True)
                fixture.events.append('history-admitted')
            def check(self): require(not fixture.missing_history, 'history changed')

        class Binding:
            def __init__(self, path, parent, batch, expected, *, create=None):
                require(type(create) is model.CompletionScope and create.digest == expected, 'scope')
                self.path, self.scope = path, create
            def __enter__(self):
                fixture.events.append('scope-synced'); fixture.existing.add(self.path); return self
            def __exit__(self, *_): fixture.events.append('scope-close')
            def check(self): require(self.path in fixture.existing, 'scope lost')
            def verify_intent(self, intent):
                require(intent == self.scope.intent(), 'intent changed'); fixture.events.append('intent-checked')

        def start(client, address, path, key, ticks, wall):
            observation = client.call('ObserveRun')
            require(observation['observation_hex'] == client.intent['observation_hex'], 'stale clock')
            fixture.events.append('intent-synced')
            client.call('PrepareRun')
            return client.call('CommitRun')

        class Store:
            def __init__(self, path, **_): self.path = path; self.intent = fixture.last_scope.intent()
            def __enter__(self): return self
            def __exit__(self, *_): self.check()
            def check(self): require(self.path in fixture.existing, 'record absent')
            def retain(self, result): fixture.events.append('outcome-synced'); return result

        old_bind_enter = Binding.__enter__
        def bind_enter(self): fixture.last_scope = self.scope; return old_bind_enter(self)
        Binding.__enter__ = bind_enter
        fixture.authority = Authority()
        fixture.batch = Batch()
        fixture.owner = Journal(budget=fixture.batch.budget)
        fixture.goal = fixture.owner.state.goal.digest
        fixture.opens.clear()
        route = types.ModuleType('route_furniture_batch')
        route.Budget = Budget
        route.packet = lambda operation, state=None, error=False: dict(ok=not error, result=state,
            padding=fixture.render_padding, agent_turn=dict(briefing={}))
        batchmod = types.ModuleType('route_furniture_batch_store'); batchmod.Batch = Batch
        planmod = types.ModuleType('route_furniture_completion'); planmod.verify_origin = verify_origin
        storemod = types.ModuleType('route_furniture_completion_store'); storemod.Journal = Journal
        finalmod = types.ModuleType('route_furniture_final'); finalmod.read_frontier = final_read
        progressmod = types.ModuleType('route_furniture_progress'); progressmod.ProgressAuthority = Authority
        invmod = types.ModuleType('route_furniture_clock_inventory'); invmod.Inventory = History
        storage = sys.modules['bounded_run_outcomes']
        storage.MAX_OUTPUT = 32768
        storage.new_run, storage.OutcomeStore = new_run, Store
        sys.modules['bounded_run_client'].start = start
        with patch.dict(sys.modules, route_furniture_batch=route, route_furniture_batch_store=batchmod,
                        route_furniture_completion=planmod, route_furniture_completion_store=storemod,
                        route_furniture_final=finalmod, route_furniture_progress=progressmod,
                        route_furniture_clock_inventory=invmod):
            # Import the complete production source, without rewriting it.
            spec = importlib.util.spec_from_file_location('completion_clock_workflow_under_test',
                                                          ROOT / 'verify_route_furniture_completion.py')
            runtime = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(runtime)
            runtime.Client, runtime.ScopeStore = Client, Binding
            # runpy's import obtains these same explicit trusted boundary ports.
            with patch.object(context, 'Client', Client), patch.object(context, 'ScopeStore', Binding):
                fixture.runtime, fixture.Journal, fixture.Batch = runtime, Journal, Batch
                yield fixture


def approve(f):
    report = f.runtime.review(f.owner, f.batch, f.authority, f.goal)
    return report['completion_clock_review']['confirm_clock']


class CompletionWorkflowTests(unittest.TestCase):
    def test_quiescent_candidate_gets_goal_bound_consent_then_one_effect_not_a_sample(self):
        with environment() as f:
            original_head = f.owner.state.tail
            token = approve(f)
            self.assertEqual(len(token), 64)
            state = f.runtime.advance(f.owner, f.batch, f.authority, f.goal, token, '/private/clocks')
            report = state['completion_clock']
            self.assertEqual(f.events.count('PrepareRun'), 1)
            self.assertEqual(f.events.count('CommitRun'), 1)
            self.assertEqual(report['clock_key'], 'frv-' + token)
            self.assertEqual(report['target_tick'], 15)
            self.assertEqual(f.owner.state.tail, original_head)
            self.assertEqual(f.owner.state.progress.observations, 1)
            self.assertFalse(report['construction_completion_proven'])
            self.assertFalse(report['completion_sample_appended'])
            self.assertTrue(report['fresh_completion_sample_required'])
            self.assertFalse(state['advance_allowed'])
            self.assertLess(f.events.index('scope-synced'), f.events.index('PrepareRun'))
            self.assertLess(f.events.index('intent-synced'), f.events.index('PrepareRun'))

    def test_stale_goal_head_capture_policy_and_limits_cannot_reuse_consent(self):
        for case in ('head', 'goal', 'inventory', 'wall', 'cap', 'capture'):
            with self.subTest(case=case), environment() as f:
                token = approve(f)
                options = {}
                if case == 'head': f.owner.state.tail = b'x' * 32
                elif case == 'goal': f.owner.state.goal.digest = '8' * 64
                elif case == 'inventory': f.batch.inventory = '8' * 64
                elif case == 'wall': options['wall_ms'] = 999
                elif case == 'cap': options['max_ticks'] = 1199
                else: f.value['source']['inventory_sha256'] = '8' * 64
                with self.assertRaises(ValueError):
                    f.runtime.advance(f.owner, f.batch, f.authority, f.goal, token, '/private/clocks', **options)
                self.assertNotIn('new-run', f.events)
                self.assertNotIn('PrepareRun', f.events)

    def test_pending_blocked_unestablished_and_changed_capture_offer_no_clock(self):
        for status in ('construction_pending', 'construction_blocked', 'construction_unestablished'):
            with self.subTest(status=status), environment() as f:
                f.status = status
                self.assertIsNone(approve(f))
                self.assertNotIn('ObserveRun', f.events)
        with environment() as f:
            f.value['source']['inventory_sha256'] = '0' * 64
            value = f.runtime.review(f.owner, f.batch, f.authority, f.goal)
            self.assertEqual(value['completion_clock_review']['decision']['status'], 'sample_required')
            self.assertNotIn('clock-connect', f.events)

    def test_terminal_interrupted_unsampled_and_incomplete_goals_refuse_before_source(self):
        for field, value in (('terminal', True), ('reading', True), ('phase', 'active'),
                             ('streak', 0), ('assessments', ())):
            with self.subTest(field=field), environment() as f:
                setattr(f.owner.state.progress, field, value)
                with self.assertRaises(ValueError): approve(f)
                self.assertEqual(f.events, [])

    def test_changed_monitor_between_receipt_brackets_refuses_consent(self):
        with environment() as f:
            def change():
                if f.reads == 2: f.owner.state.tail = b'x' * 32
            f.after_final = change
            with self.assertRaises(ValueError): approve(f)
            self.assertNotIn('new-run', f.events)

    def test_newer_frontier_requires_sampling_before_clock(self):
        with environment() as f:
            f.clock_tick = 12
            state = f.runtime.review(f.owner, f.batch, f.authority, f.goal)
            self.assertEqual(state['completion_clock_review']['decision']['status'], 'sample_required')
            self.assertIsNone(state['completion_clock_review']['confirm_clock'])
            self.assertNotIn('ObserveRun', f.events)

    def test_changed_final_evidence_between_brackets_returns_no_consent(self):
        with environment() as f:
            def change():
                if f.reads == 1: f.value['frontier_evidence'] = '7' * 64
            f.after_final = change
            self.assertIsNone(approve(f))
            self.assertNotIn('PrepareRun', f.events)

    def test_history_blocks_new_effect_for_verification_too(self):
        with environment() as f:
            token = approve(f); f.missing_history = True
            with self.assertRaises(ValueError):
                f.runtime.advance(f.owner, f.batch, f.authority, f.goal, token, '/private/clocks')
            self.assertNotIn('scope-synced', f.events)
            self.assertNotIn('PrepareRun', f.events)

    def test_monitor_change_or_revocation_after_prepare_prevents_commit(self):
        for case in ('head', 'authority', 'history'):
            with self.subTest(case=case), environment() as f:
                token = approve(f)
                def change():
                    if case == 'head': f.owner.state.tail = b'x' * 32
                    elif case == 'authority': f.authority.revoked = True
                    else: f.missing_history = True
                f.after_prepare = change
                with self.assertRaises(ValueError):
                    f.runtime.advance(f.owner, f.batch, f.authority, f.goal, token, '/private/clocks')
                self.assertEqual(f.events.count('PrepareRun'), 1)
                self.assertNotIn('CommitRun', f.events)
                self.assertEqual(len(f.existing), 1)

    def test_lost_commit_reply_never_repeats_effect_or_appends_monitor(self):
        with environment() as f:
            token = approve(f); f.fail_commit = True
            with self.assertRaises(OSError):
                f.runtime.advance(f.owner, f.batch, f.authority, f.goal, token, '/private/clocks')
            self.assertEqual(f.events.count('CommitRun'), 1)
            self.assertEqual(f.owner.state.progress.observations, 1)
            self.assertEqual(len(f.existing), 1)
            self.assertNotIn('QueryRun', f.events)

    def test_publication_revocation_preserves_no_claim_of_completion(self):
        with environment() as f:
            token = approve(f)
            f.after_commit = lambda: setattr(f.authority, 'revoked', True)
            with self.assertRaises(ValueError):
                f.runtime.advance(f.owner, f.batch, f.authority, f.goal, token, '/private/clocks')
            self.assertEqual(f.events.count('CommitRun'), 1)
            self.assertEqual(f.owner.state.progress.streak, 1)

    def test_output_and_budget_refusals_precede_new_intent(self):
        for case in ('output', 'budget', 'deadline'):
            with self.subTest(case=case), environment() as f:
                token = approve(f)
                if case == 'output': f.render_padding = 'x' * f.runtime.MAX_OUTPUT
                elif case == 'budget': f.batch.budget.calls = 2
                else: f.batch.budget.deadline = time.monotonic() + 0.5
                with self.assertRaises(ValueError):
                    f.runtime.advance(f.owner, f.batch, f.authority, f.goal, token, '/private/clocks')
                self.assertNotIn('new-run', f.events)

    def test_shared_original_owner_budget_and_readonly_custody_are_required(self):
        for case in ('batch', 'journal', 'budget'):
            with self.subTest(case=case), environment() as f:
                if case == 'batch': f.batch.writable = True
                elif case == 'journal': f.owner.writable = True
                else: f.owner.budget = f.runtime.make_budget(10000)
                with self.assertRaises(ValueError): approve(f)
                self.assertEqual(f.events, [])

    def test_clock_advancement_alone_never_satisfies_the_original_goal(self):
        with environment() as f:
            token = approve(f)
            f.runtime.advance(f.owner, f.batch, f.authority, f.goal, token, '/private/clocks')
            again = f.runtime.review(f.owner, f.batch, f.authority, f.goal)
            self.assertIsNone(again['completion_clock_review']['confirm_clock'])
            self.assertEqual(again['completion_clock_review']['decision']['status'], 'sample_required')
            self.assertEqual(f.owner.state.progress.phase, 'candidate')

    def test_cli_precontact_argument_checks_and_single_error_object(self):
        with environment() as f:
            args = ['advance', '--directory', '/private/batch', '--batch-id', f.batch.id,
                    '--completion-journal', '/private/completion', '--expected-goal', f.goal]
            stream = io.BytesIO()
            with patch.object(f.runtime.sys, 'stdout', types.SimpleNamespace(buffer=stream)):
                self.assertEqual(f.runtime.main(args), 2)
            self.assertEqual(len(stream.getvalue().splitlines()), 1)
            self.assertEqual(f.opens, [])
            self.assertEqual(f.events, [])

    def test_cli_holds_both_original_owners_through_the_single_output_attempt(self):
        with environment() as f:
            args = ['review', '--directory', '/private/batch', '--batch-id', f.batch.id,
                    '--completion-journal', '/private/completion', '--expected-goal', f.goal]
            owners = []
            original_batch, original_journal = f.runtime.Batch, f.runtime.Journal
            # Keep exact Journal type at witness; observe the constructor, not a factory replacement.
            old_binit, old_jinit = original_batch.__init__, original_journal.__init__
            def bi(self, *a, **kw): old_binit(self, *a, **kw); owners.append(self)
            def ji(self, *a, **kw): old_jinit(self, *a, **kw); owners.append(self)
            writes = []
            def output(raw):
                self.assertEqual(len(owners), 2)
                self.assertTrue(all(not o.closed for o in owners))
                writes.append(raw); return 2
            with patch.object(original_batch, '__init__', bi), patch.object(original_journal, '__init__', ji), \
                 patch.object(f.runtime, 'publish', side_effect=output):
                self.assertEqual(f.runtime.main(args), 2)
            self.assertEqual(len(writes), 1)
            self.assertTrue(all(o.closed for o in owners))


    def test_direct_script_review_executes_without_import_identity_drift(self):
        with environment() as f:
            args = [str(ROOT / 'verify_route_furniture_completion.py'), 'review',
                    '--directory', '/private/batch', '--batch-id', f.batch.id,
                    '--completion-journal', '/private/completion', '--expected-goal', f.goal]
            output = io.BytesIO()
            with patch.object(sys, 'argv', args), patch.object(sys, 'stdout', types.SimpleNamespace(buffer=output)):
                with self.assertRaises(SystemExit) as done:
                    runpy.run_path(args[0], run_name='__main__')
            self.assertEqual(done.exception.code, 0)
            packet = __import__('json').loads(output.getvalue())
            self.assertEqual(len(packet['result']['completion_clock_review']['confirm_clock']), 64)
            self.assertEqual(f.events.count('ObserveRun'), 1)
            self.assertNotIn('PrepareRun', f.events)

    def test_clock_stop_is_not_a_stability_sample_and_original_sampler_can_finish_afterward(self):
        # Pure independent monitor oracle: only a new observed condition at an
        # eligible tick can complete the fixed policy. The clock owner leaves it
        # untouched. This is not the production monitor codec/replay qualification.
        with environment() as f:
            token = approve(f)
            out = f.runtime.advance(f.owner, f.batch, f.authority, f.goal, token, '/private/clocks')
            self.assertEqual(f.clock_tick, out['completion_clock']['target_tick'])
            state, condition = f.owner.state.progress, f.owner.state.goal.condition
            self.assertFalse(state.terminal)
            qualifies = (f.clock_tick - state.counted_tick >= condition.interval
                         and state.streak + 1 >= condition.stable_samples
                         and f.clock_tick - state.first_tick >= condition.stable_span
                         and f.clock_tick < condition.deadline)
            self.assertTrue(qualifies)
            self.assertEqual(state.observations, 1)


if __name__ == '__main__': unittest.main()
