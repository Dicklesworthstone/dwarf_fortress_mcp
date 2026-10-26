"""Execute both complete progress owners with explicit infrastructure ports.

These tests isolate family dispatch, CLI, post-stop reporting and consent flow.
They do NOT qualify native transport/codecs, source acquisition or private files.
No production source is rewritten by the loader. The ports intentionally model
both scope families so a wrong-family handoff is observable, not silently fixed.
"""
from __future__ import annotations

from contextlib import contextmanager
from copy import deepcopy
from dataclasses import dataclass, field
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import sys
import types
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent


def require(ok, message):
    if not ok:
        raise ValueError(message)


def integer(n, low, high):
    require(type(n) is int and low <= n <= high, 'integer bound')
    return n


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()


def digest_text(value):
    require(type(value) is str and re.fullmatch('[0-9a-f]{64}', value) is not None, 'digest')
    return value


def endpoint(text):
    host, port = text.split(':')
    require(host == '127.0.0.1', 'loopback only')
    return host, integer(int(port), 1, 65535)


@dataclass(frozen=True)
class Source:
    tick: int
    incarnation: int = 7

    @classmethod
    def from_json(cls, value):
        return cls(**value)

    def permits(self, current):
        require(current.incarnation == self.incarnation and current.tick >= self.tick, 'source regression')


@dataclass(frozen=True)
class Scope:
    raw: bytes
    prefix = 'frp-'

    @classmethod
    def create(cls, value):
        return cls(canonical(value))

    @property
    def digest(self):
        return hashlib.sha256(self.prefix.encode() + self.raw).hexdigest()

    @property
    def clock_key(self):
        return self.prefix + self.digest

    def value(self):
        return json.loads(self.raw)

    def bind(self, batch, *, current=False):
        value = self.value()
        require(value['batch_id'] == batch.id, 'batch substitution')
        if current:
            require(value['inventory_digest'] == batch.inventory and not batch.stopped, 'changed inventory')
            if type(self) is FinalScope:
                require(batch.status == 'all_placed', 'not final')
            else:
                require(batch.status == 'ready', 'no next placement')
        return value

    def intent(self):
        return {'idempotency_key': self.clock_key, 'batch': self.value()['batch_id']}


class FinalScope(Scope):
    prefix = 'frf-'


@dataclass(frozen=True)
class Authority:
    address: tuple[str, int]
    token: bytes = field(repr=False)
    operations_token: bytes = field(repr=False)
    map_token: bytes = field(repr=False)
    allow_place: bool = False


class Budget:
    def __init__(self, timeout_ms=10000):
        integer(timeout_ms, 1, 60000)
        self.calls, self.network_bytes = 560, 40 * 1024 * 1024
        self.disk_bytes, self.work_steps = 1024 * 1024 * 1024, 20_000_000
        self.deadline = 60

    def remaining(self):
        require(self.deadline > 0, 'deadline')
        return self.deadline

    def reserve(self, name, n):
        self.remaining()
        require(0 <= n <= getattr(self, name), 'budget')
        setattr(self, name, getattr(self, name) - n)

    def work(self):
        self.remaining()
        self.work_steps -= 1
        require(self.work_steps >= 0, 'work')


class Batch:
    def __init__(self):
        self.id = 'a' * 64
        self.review = types.SimpleNamespace(digest='b' * 64)
        self.plan = types.SimpleNamespace(digest='c' * 64, steps=('bed', 'chair'))
        self.inventory = 'd' * 64
        self.address = ('127.0.0.1', 5000)
        self.status, self.finding = 'all_placed', 'construction_pending'
        self.writable = self.stopped = self.closed = self.corrupt = False
        self.budget, self.tick = Budget(), 10

    def check(self):
        self.budget.remaining()
        require(not self.closed and not self.corrupt, 'original custody unavailable')

    def audit(self):
        self.check()
        return {'batch_id': self.id, 'review_digest': self.review.digest,
                'inventory_digest': self.inventory, 'inventory_verified': True,
                'status': self.status, 'next_step': None if self.status == 'all_placed' else 'chair',
                'pending_step': None, 'placed': 2 if self.status == 'all_placed' else 1,
                'stopped': self.stopped, 'advance_allowed': self.status == 'ready',
                'steps': [{'name': 'bed', 'phase': 'placed'},
                          {'name': 'chair', 'phase': 'placed' if self.status == 'all_placed' else 'not_started'}]}

    def close(self):
        self.closed = True


class Backend:
    def __init__(self):
        self.batch = Batch()
        self.records, self.calls, self.reads = {}, [], []
        self.held = self.opens = 0
        self.scope = None
        self.query_result = None
        self.unresolved = False
        self.drop_commit = False
        self.after_observe = lambda: None
        self.after_commit = lambda: None
        self.padding = ''

    def frontier(self, batch, authority, *, final=False):
        authority.guard('QueryPlacement')
        batch.check()
        require(not batch.stopped, 'stopped original batch')
        require((batch.status == 'all_placed') == final, 'wrong frontier family')
        self.reads.append('final' if final else 'step')
        state = batch.audit()
        report = {'batch_id': batch.id, 'phase': 'final_construction' if final else 'next_step',
                  'status': batch.finding, 'source': {'tick': batch.tick, 'incarnation': 7},
                  'construction_prefix': {'status': 'pending' if batch.finding == 'construction_pending' else 'quiescent'},
                  'all_original_furnishings_quiescent_now': batch.finding == 'construction_quiescent'}
        report['evidence_digest'] = hashlib.sha256(canonical(report)).hexdigest()
        state.update(route_frontier=report, expected_plan=None, confirm_review=None,
                     advance_allowed=False, execution_review_required=True)
        if self.padding:
            state['padding'] = self.padding
        return state, report

    def retained(self, scope, phase='stopped'):
        return {'record': {'key': scope.clock_key, 'phase': phase,
                           'pause_verified': phase == 'stopped',
                           'observed_tick': 30 if phase == 'stopped' else None},
                'manifest': {'generation': 11}, 'current_pause_unproved': True}


def module(name, **values):
    result = types.ModuleType(name)
    result.__dict__.update(values)
    return result


@contextmanager
def environment(source_path=None):
    back = Backend()
    route = module('route_furniture_batch', Authority=Authority, Budget=Budget,
        OPT_IN='DFMCP_ALLOW_UNADMITTED_ROUTE_FURNITURE_BATCH', ENDPOINT='DFMCP_ROUTE_FURNITURE_BATCH_ENDPOINT',
        PLACE='DFMCP_ROUTE_FURNITURE_ALLOW_PLACE', MAP_TOKEN='DFMCP_MAP_TOKEN', OPS_TOKEN='DFMCP_OPERATIONS_PAGED_TOKEN',
        native=types.SimpleNamespace(TOKEN='DFMCP_BUILD_TOKEN', endpoint=endpoint))
    route.ENVIRONMENT = (route.OPT_IN, route.ENDPOINT, route.PLACE, route.MAP_TOKEN, route.OPS_TOKEN, route.native.TOKEN)
    route.review_step = lambda b, a: back.frontier(b, a)[0]
    route.packet = lambda op, state, error=False: {'ok': not error, 'result': state,
        'agent_turn': {'operation': op, 'phase': 'inspect', 'briefing': {}, 'coverage': {},
                       'active_work': [], 'uncertainty': []}}

    class Client:
        def __init__(self, authority, budget, guard, *, intent=None, query=False, effect_guard=None):
            self.authority, self.budget, self.guard = authority, budget, guard
            self.intent, self.query, self.effect_guard = intent, query, effect_guard
            self.commit_attempted = False
        def __enter__(self):
            self.authority.clock_guard(); self.guard(); return self
        def __exit__(self, *_): pass
        def call(self, op):
            self.authority.clock_guard(); self.guard()
            if self.query: require(op == 'QueryRun', 'query-only')
            elif self.intent is None: require(op == 'ObserveRun', 'read-only clock')
            if op in ('PrepareRun', 'CommitRun'):
                self.effect_guard(self.intent)
            back.calls.append(op)
            if op == 'ObserveRun':
                out = {'owner_active': False, 'retained_records': 0,
                       'manifest': {'generation': 11}, 'observation_hex': 'clock-fixture'}
                back.after_observe(); return out
            if op == 'CommitRun':
                self.commit_attempted = True
                result = back.retained(back.scope)
                back.after_commit()
                if back.drop_commit: raise OSError('lost acknowledgment')
                return result
            if op == 'QueryRun':
                return back.query_result or back.retained(back.scope)
            return {}

    class Store:
        def __init__(self, path, **kw):
            self.path, self.parent = path, 42
        def __enter__(self):
            back.held += 1
            row = back.records[self.path.name]
            self.intent = row['scope'].intent()
            return self
        def __exit__(self, *_): back.held -= 1
        def load(self): return back.records[self.path.name]['retained']
        def view(self): return deepcopy(self.load())
        def retain(self, result):
            back.records[self.path.name]['retained'] = deepcopy(result)
            return result
        def check(self): back.batch.check()

    class ScopeStore:
        def __init__(self, path, parent, batch, confirmation, *, create=None):
            self.path, self.batch, self.create = path, batch, create
        def __enter__(self):
            if self.create is not None:
                require(self.path.name not in back.records, 'existing original record')
                self.create.bind(self.batch, current=True)
                back.records[self.path.name] = {'scope': self.create, 'retained': None}
            self.scope = back.records[self.path.name]['scope']
            self.scope.bind(self.batch)
            back.scope = self.scope
            return self
        def __exit__(self, *_): pass
        def check(self): self.batch.check()
        def verify_intent(self, intent): require(intent == self.scope.intent(), 'changed intent')

    class Inventory:
        digest = 'e' * 64
        def __init__(self, *args, **kw): pass
        def __enter__(self): return self
        def __exit__(self, *_): pass
        def admit_new(self, scope):
            require(not back.unresolved, 'unresolved prior history')
            scope.bind(back.batch, current=True)
        def check(self): back.batch.check()

    @contextmanager
    def new_run(path):
        back.opens += 1
        require(path.name not in back.records, 'existing original record')
        back.held += 1
        try: yield 42
        finally: back.held -= 1

    def start(client, address, path, key, ticks, wall):
        require(key == back.scope.clock_key, 'wrong native key')
        client.call('PrepareRun')
        return client.call('CommitRun')

    def recover(client, intent, cancel, wait):
        require(cancel is False and client.query, 'not query-only')
        require(intent == back.scope.intent(), 'changed query intent')
        return client.call('QueryRun')

    def open_batch(path, identity, budget, *, writable=False):
        require(identity == back.batch.id and not writable, 'read-only original batch')
        back.batch.budget = budget
        back.batch.closed = False
        return back.batch

    modules = {
        'bounded_run_client': module('bounded_run_client', endpoint=endpoint, start=start, recover=recover),
        'bounded_run_outcomes': module('bounded_run_outcomes', MAX_OUTPUT=32768, MAX_FILE=8192,
                                       OutcomeStore=Store, new_run=new_run),
        'furniture_plan': module('furniture_plan', canonical=canonical, integer=integer, require=require),
        'route_furniture_batch': route,
        'route_furniture_run': module('route_furniture_run', _review_status=lambda *args: None),
        'route_furniture_batch_store': module('route_furniture_batch_store', Batch=open_batch),
        'route_furniture_review': module('route_furniture_review', Source=Source, digest_text=digest_text),
        'route_furniture_clock_context': module('route_furniture_clock_context', Client=Client,
            Scope=Scope, FinalScope=FinalScope, ScopeStore=ScopeStore, POLICY='step/2', SCHEMA='step/2',
            FINAL_POLICY='final/1', FINAL_SCHEMA='final/1'),
        'route_furniture_clock_inventory': module('route_furniture_clock_inventory', Inventory=Inventory),
        'route_furniture_final': module('route_furniture_final', read_frontier=lambda b,a: back.frontier(b,a,final=True)),
    }
    env = {route.OPT_IN: '1', 'DFMCP_ALLOW_UNADMITTED_RUN_V1_13': '1',
           route.ENDPOINT: '127.0.0.1:5000', 'DFMCP_RUN_ENDPOINT': '127.0.0.1:5000',
           route.native.TOKEN: 'b'*32, route.OPS_TOKEN: 'o'*32, route.MAP_TOKEN: 'm'*32,
           'DFMCP_RUN_TOKEN': 'r'*32}
    with patch.dict(sys.modules, modules), patch.dict(os.environ, env, clear=True):
        loaded = []
        for name, path in (('route_furniture_progress', source_path or ROOT/'route_furniture_progress.py'),
                           ('route_furniture_final_progress', ROOT/'route_furniture_final_progress.py')):
            spec = importlib.util.spec_from_file_location(name, path)
            value = importlib.util.module_from_spec(spec)
            # The same module identities are retained during cross-owner calls.
            sys.modules[name] = value
            spec.loader.exec_module(value)
            loaded.append(value)
        yield (*loaded, back)


class FamilyRoutingTests(unittest.TestCase):
    def setUp(self):
        self.p, self.f, self.back = self.enterContext(environment())
        self.batch = self.back.batch
        self.authority = self.p.ProgressAuthority.load()

    def scope(self, cls=FinalScope):
        b = self.batch
        return cls.create({'batch_id': b.id, 'inventory_digest': b.inventory,
                           'source': {'tick': 10, 'incarnation': 7}, 'clock_manifest': {'generation': 11}})

    def saved(self, cls=FinalScope, phase='stopped'):
        scope = self.scope(cls)
        path = self.p._record_path('/private/clock', scope.digest)
        self.back.records[path.name] = {'scope': scope,
            'retained': None if phase is None else self.back.retained(scope, phase)}
        return scope

    def test_final_offline_query_preserves_family_key_and_final_report(self):
        scope = self.saved()
        value = self.p.query_progress(self.batch, None, scope.digest, '/private/clock')
        self.assertIn('final_construction_progress', value)
        result = value['final_construction_progress']
        self.assertEqual(result['clock_key'], scope.clock_key)
        self.assertFalse(result['clock_commit_invoked_this_call'])
        self.assertFalse(result['construction_completion_proven'])
        self.assertFalse(value['advance_allowed'])
        self.assertEqual(self.back.calls, [])
        self.assertEqual(self.back.reads, [])

    def test_final_query_refresh_uses_complete_final_reader(self):
        scope = self.saved()
        self.batch.tick, self.batch.finding = 30, 'construction_quiescent'
        value = self.p.query_progress(self.batch, self.authority, scope.digest,
                                      '/private/clock', refresh_frontier=True)
        self.assertIn('final_construction_progress', value)
        report = value['final_construction_progress']
        self.assertTrue(report['all_original_furnishings_quiescent_now'])
        self.assertFalse(report['construction_completion_proven'])
        self.assertEqual(self.back.reads, ['final'])
        self.assertEqual(self.back.calls, [])
        self.assertEqual(report['post_progress_review']['game_tick'], 30)

    def test_unknown_final_query_is_query_only_and_retains_original_identity(self):
        scope = self.saved(phase=None)
        value = self.p.query_progress(self.batch, self.authority, scope.digest, '/private/clock')
        self.assertIn('final_construction_progress', value)
        report = value['final_construction_progress']
        self.assertTrue(report['native_query_contacted'])
        self.assertEqual(report['clock_key'], scope.clock_key)
        self.assertEqual(self.back.calls, ['QueryRun'])
        self.assertEqual(self.back.reads, [])

    def test_legacy_query_after_final_placement_is_not_reinterpreted(self):
        scope = self.saved(Scope)
        value = self.p.query_progress(self.batch, None, scope.digest, '/private/clock')
        self.assertIn('construction_progress', value)
        self.assertNotIn('final_construction_progress', value)
        self.assertEqual(value['construction_progress']['clock_key'], scope.clock_key)
        self.assertEqual(self.back.calls, [])

    def test_common_review_selects_final_owner_and_exposes_common_confirmation(self):
        value = self.p.review_progress(self.batch, self.authority, 20, 5000)
        self.assertIn('progress_review', value)
        self.assertEqual(value['progress_review'], value['final_progress_review'])
        self.assertIsNotNone(value['progress_review']['confirm_progress'])
        self.assertEqual(value['progress_review']['scope']['phase'], 'final_construction')
        self.assertEqual(self.back.reads, ['final', 'final'])
        self.assertEqual(self.back.calls, ['ObserveRun'])
        self.assertEqual(self.back.records, {})

    def test_final_quiescent_blocked_and_unknown_reviews_do_not_observe_clock(self):
        for finding in ('construction_quiescent', 'construction_blocked', 'construction_unestablished'):
            with self.subTest(finding=finding):
                self.batch.finding = finding
                value = self.p.review_progress(self.batch, self.authority, 20, 5000)
                self.assertIsNone(value['progress_review']['confirm_progress'])
                self.assertFalse(value['advance_allowed'])
        self.assertEqual(self.back.calls, [])

    def test_changed_final_bracket_does_not_issue_consent(self):
        self.back.after_observe = lambda: setattr(self.batch, 'tick', 11)
        value = self.p.review_progress(self.batch, self.authority, 20, 5000)
        self.assertIsNone(value['progress_review']['confirm_progress'])
        self.assertEqual(self.back.records, {})

    def test_common_progress_executes_one_final_effect_and_reobserves(self):
        token = self.p.review_progress(self.batch, self.authority, 20, 5000)['progress_review']['confirm_progress']
        def finish():
            self.batch.tick, self.batch.finding = 30, 'construction_quiescent'
        self.back.after_commit = finish
        out = self.p.progress(self.batch, self.authority, 20, 5000, token, '/private/clock')
        self.assertIn('final_construction_progress', out)
        result = out['final_construction_progress']
        self.assertEqual(result['clock_key'], 'frf-' + token)
        self.assertTrue(result['all_original_furnishings_quiescent_now'])
        self.assertFalse(result['construction_completion_proven'])
        self.assertEqual(self.back.calls.count('PrepareRun'), 1)
        self.assertEqual(self.back.calls.count('CommitRun'), 1)
        self.assertNotIn('step', self.back.reads)

    def test_stale_step_consent_does_not_turn_into_final_dispatch(self):
        with self.assertRaises(ValueError):
            self.p.progress(self.batch, self.authority, 20, 5000, self.scope(Scope).digest, '/private/clock')
        self.assertEqual(self.back.records, {})
        self.assertNotIn('PrepareRun', self.back.calls)
        self.assertNotIn('CommitRun', self.back.calls)

    def test_unresolved_history_still_gates_auto_selected_final_effect(self):
        token = self.p.review_progress(self.batch, self.authority, 20, 5000)['progress_review']['confirm_progress']
        self.back.unresolved = True
        with self.assertRaises(ValueError):
            self.p.progress(self.batch, self.authority, 20, 5000, token, '/private/clock')
        self.assertEqual(self.back.records, {})
        self.assertNotIn('PrepareRun', self.back.calls)

    def test_lost_final_reply_recovery_uses_same_key_without_new_commit(self):
        token = self.p.review_progress(self.batch, self.authority, 20, 5000)['progress_review']['confirm_progress']
        self.back.drop_commit = True
        with self.assertRaises(OSError):
            self.p.progress(self.batch, self.authority, 20, 5000, token, '/private/clock')
        value = self.p.query_progress(self.batch, self.authority, token, '/private/clock')
        self.assertEqual(value['final_construction_progress']['clock_key'], 'frf-' + token)
        self.assertEqual(self.back.calls.count('CommitRun'), 1)
        self.assertEqual(self.back.calls.count('QueryRun'), 1)

    def test_final_source_loss_or_pre_stop_observation_never_proves_current_quiescence(self):
        for phase in ('source_lost', 'stopped'):
            with self.subTest(phase=phase):
                scope = self.saved(phase=phase)
                value = self.p.query_progress(self.batch, self.authority, scope.digest,
                                              '/private/clock', refresh_frontier=True)
                result = value['final_construction_progress']
                self.assertFalse(result['all_original_furnishings_quiescent_now'])
                self.assertIsNone(result['post_progress_review'])
                self.assertFalse(result['construction_completion_proven'])

    def test_next_step_review_keeps_legacy_shape(self):
        self.batch.status = 'ready'
        value = self.p.review_progress(self.batch, self.authority, 20, 5000)
        self.assertNotIn('final_progress_review', value)
        self.assertEqual(value['progress_review']['policy'], 'step/2')
        self.assertEqual(self.back.reads, ['step', 'step'])

    def test_final_result_is_visible_in_common_agent_briefing(self):
        scope = self.saved()
        out = self.p.query_progress(self.batch, None, scope.digest, '/private/clock')
        briefing = self.p.packet('query', out)['agent_turn']['briefing']
        self.assertIn('all_original_furnishings_quiescent_now', briefing)
        self.assertFalse(briefing['construction_completion_proven'])
        self.assertFalse(briefing['clock_commit_invoked_this_call'])

    def test_cli_query_discovers_family_without_credentials_or_extra_option(self):
        scope = self.saved()
        output = []
        with patch.dict(os.environ, {}, clear=True), patch.object(self.p, 'publish', side_effect=lambda b: output.append(b) or 0):
            code = self.p.main(['query', '--directory', '/private/batch', '--batch-id', self.batch.id,
                '--clock-directory', '/private/clock', '--confirm-progress', scope.digest])
        self.assertEqual(code, 0)
        self.assertEqual(len(output), 1)
        self.assertIn('final_construction_progress', json.loads(output[0])['result'])
        self.assertTrue(self.batch.closed)
        self.assertEqual(self.back.calls, [])

    def test_review_revocation_refuses_before_any_read(self):
        with patch.dict(os.environ, {'DFMCP_RUN_TOKEN': 'changed'}), self.assertRaises(ValueError):
            self.p.review_progress(self.batch, self.authority, 20, 5000)
        self.assertEqual(self.back.reads, [])

    def test_wrong_family_never_falls_back_after_final_observation_failure(self):
        self.batch.stopped = True
        with self.assertRaises(ValueError):
            self.p.review_progress(self.batch, self.authority, 20, 5000)
        self.assertEqual(self.back.reads, [])
        self.assertEqual(self.back.calls, [])


if __name__ == '__main__':
    unittest.main()
