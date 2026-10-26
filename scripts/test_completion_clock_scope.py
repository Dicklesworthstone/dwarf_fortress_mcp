"""Actual scope/context code with explicit native/source codec ports, no I/O.

These tests qualify new scheduling, domain separation and preserved legacy scope
behavior, not the unchanged native codecs or private custody implementation.
"""
from contextlib import contextmanager
from copy import deepcopy
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import struct
import sys
import types
import unittest
from unittest.mock import patch

from completion_clock_plan import Window
from test_completion_clock_plan import window

ROOT = Path(__file__).resolve().parent


def require(ok, message):
    if not ok: raise ValueError(message)


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'),
                      ensure_ascii=True, allow_nan=False).encode()


def integer(value, low, high):
    require(type(value) is int and low <= value <= high, 'integer')
    return value


def digest_text(value):
    require(type(value) is str and re.fullmatch('[0-9a-f]{64}', value), 'digest')
    return value


class SourcePort:
    def __init__(self, **values): self.__dict__.update(values)
    @classmethod
    def from_json(cls, values): return cls(**values)
    def permits(self, current):
        for name in ('endpoint', 'df_version', 'dfhack_version', 'map_generation',
                     'inventory_generation', 'dimensions', 'origin', 'size'):
            require(getattr(self, name) == getattr(current, name), 'source changed')
        require(current.tick >= self.tick, 'regressed source')


def snapshot(raw):
    require(len(raw) == 35 and raw[:8] == b'DFMRO013', 'snapshot')
    gen, seq, tick, loaded, valid, paused = struct.unpack('>QQQBBB', raw[8:])
    return dict(generation=gen, sequence=seq, tick=tick, loaded=bool(loaded),
                clock_valid=bool(valid), paused=bool(paused))


@contextmanager
def modules():
    native = types.ModuleType('bounded_run_client')
    native.canonical, native.require, native.integer = canonical, require, integer
    native.digest = lambda domain, raw: hashlib.sha256(domain + b'\0' + raw).digest()
    native.exact_hex = lambda value, size: bytes.fromhex(value) if len(value) == 2 * size else None
    native.snapshot = snapshot
    native.endpoint = lambda raw: (raw.split(':')[0], int(raw.split(':')[1]))
    native.plan_for = lambda ticks, wall, raw: hashlib.sha256(struct.pack('>II', ticks, wall) + raw).digest()
    native.token_for = lambda key, plan: hashlib.sha256(key.encode() + plan).digest()[:16]
    native.verify_intent = lambda value: dict(value)
    class NoTransport:
        def __init__(self, *_a, **_k): raise AssertionError('scope tests must not contact native transport')
    native.Client = NoTransport
    storage = types.ModuleType('bounded_run_outcomes')
    storage.MAX_FILE = 8192
    furniture = types.ModuleType('furniture_plan')
    furniture.canonical = canonical
    furniture.decode_json = json.loads
    furniture.integer, furniture.require = integer, require
    furniture.label = lambda value: require(type(value) is str and bool(value), 'label')
    review = types.ModuleType('route_furniture_review')
    review.Source, review.digest_text = SourcePort, digest_text
    with patch.dict(sys.modules, bounded_run_client=native, bounded_run_outcomes=storage,
                    furniture_plan=furniture, route_furniture_review=review):
        loaded = []
        for name in ('route_furniture_clock_context', 'completion_clock_scope'):
            spec = importlib.util.spec_from_file_location(name, ROOT / (name + '.py'))
            module = importlib.util.module_from_spec(spec)
            loaded.append((name, sys.modules.get(name)))
            sys.modules[name] = module
            spec.loader.exec_module(module)
        try:
            yield sys.modules['route_furniture_clock_context'], sys.modules['completion_clock_scope']
        finally:
            for name, old in reversed(loaded):
                if old is None: sys.modules.pop(name, None)
                else: sys.modules[name] = old


def scope_value(module, **changes):
    w = window()
    source = dict(endpoint='127.0.0.1:5000', df_version='df', dfhack_version='dfhack',
                  map_generation=7, inventory_generation=9, map_sha256='0' * 64,
                  inventory_sha256=w.last_capture, tick=w.last_tick,
                  dimensions=[32, 32, 8], origin=[1, 1, 0], size=[4, 4, 1],
                  horizons=[100, 200, 300])
    result = dict(schema=module.SCHEMA, policy=module.POLICY, phase='completion_verification',
                  batch_id='1' * 64, review_digest='2' * 64, plan_digest='3' * 64,
                  inventory_digest='4' * 64, frontier_evidence='5' * 64,
                  source=source, game_ticks=5, wall_ms=1000, max_ticks=1200,
                  completion=w.json(), clock_manifest=dict(generation=11, df_version='df',
                                                          dfhack_version='dfhack'),
                  clock_observation_hex=(b'DFMRO013' + struct.pack('>QQQBBB', 11, 0, 10, 1, 1, 1)).hex())
    result.update(changes)
    return result


class CompletionScopeTests(unittest.TestCase):
    def test_exact_codec_roundtrip_and_distinct_native_key(self):
        with modules() as (context, model):
            value = scope_value(model)
            scope = model.CompletionScope.create(value)
            self.assertEqual(context.decode_scope(scope.raw), scope)
            self.assertEqual(scope.clock_key, 'frv-' + scope.digest)
            self.assertEqual(scope.intent()['idempotency_key'], scope.clock_key)
            self.assertTrue(context._is_scope(scope))
            self.assertEqual(scope.value(), value)

    def test_goal_journal_policy_and_clock_limits_participate_in_consent(self):
        with modules() as (_, model):
            value = scope_value(model)
            original = model.CompletionScope.create(value)
            for field, replacement in (('goal_digest', 'd' * 64), ('journal_head', 'e' * 64),
                                       ('journal_frames', 5), ('max_observations', 9),
                                       ('deadline', 101)):
                new = deepcopy(value)
                new['completion'][field] = replacement
                self.assertNotEqual(model.CompletionScope.create(new).digest, original.digest)
            for field, replacement in (('wall_ms', 999), ('max_ticks', 1199)):
                new = dict(value, **{field: replacement})
                self.assertNotEqual(model.CompletionScope.create(new).digest, original.digest)

    def test_mismatched_capture_tick_interval_and_foreign_scope_refuse(self):
        with modules() as (_, model):
            value = scope_value(model)
            for field, replacement in (('game_ticks', 6), ('max_ticks', 4),
                                       ('schema', 'dfmcp.route-furniture-final-progress/1'),
                                       ('phase', 'final_construction')):
                with self.subTest(field=field), self.assertRaises(ValueError):
                    model.CompletionScope.create(dict(value, **{field: replacement}))
            for field, replacement in (('inventory_sha256', '6' * 64), ('tick', 11)):
                new = deepcopy(value); new['source'][field] = replacement
                with self.assertRaises(ValueError): model.CompletionScope.create(new)
            with self.assertRaises(ValueError): model.CompletionScope.create(dict(value, unexpected=True))

    def test_current_binding_requires_original_all_placed_inventory(self):
        with modules() as (_, model):
            value = scope_value(model); scope = model.CompletionScope.create(value)
            state = dict(inventory_verified=True, inventory_digest='4' * 64, status='all_placed',
                         next_step=None, pending_step=None, stopped=False, placed=2)
            batch = types.SimpleNamespace(id=value['batch_id'], review=types.SimpleNamespace(digest=value['review_digest']),
                plan=types.SimpleNamespace(digest=value['plan_digest'], steps=(1, 2)),
                address=('127.0.0.1', 5000), source=SourcePort(**value['source']), audit=lambda: state)
            scope.bind(batch, current=True)
            for field, replacement in (('stopped', True), ('placed', 1), ('next_step', 'bed'),
                                       ('inventory_digest', '7' * 64), ('status', 'ready')):
                original = state[field]; state[field] = replacement
                with self.assertRaises(ValueError): scope.bind(batch, current=True)
                state[field] = original

    def test_existing_scope_raw_digests_and_keys_are_unchanged(self):
        with modules() as (context, model):
            base = {key: val for key, val in scope_value(model).items() if key in context.FINAL_SCOPE_KEYS}
            base.update(schema=context.FINAL_SCHEMA, policy=context.FINAL_POLICY, phase='final_construction')
            final = context.FinalScope.create(base)
            self.assertEqual(final.raw, canonical(base))
            self.assertEqual(final.digest, hashlib.sha256(
                b'dfmcp-route-furniture-final-progress-confirmation/1\0' + canonical(base)).hexdigest())
            self.assertEqual(final.clock_key, 'frf-' + final.digest)
            legacy = dict(base); legacy.pop('phase')
            legacy.update(schema=context.SCHEMA, policy=context.POLICY, next_step='chair')
            regular = context.Scope.create(legacy)
            self.assertEqual(regular.raw, canonical(legacy))
            self.assertEqual(regular.digest, hashlib.sha256(
                b'dfmcp-route-furniture-progress-confirmation/2\0' + canonical(legacy)).hexdigest())
            self.assertEqual(regular.clock_key, 'frp-' + regular.digest)
            self.assertEqual(context.decode_scope(final.raw), final)
            self.assertEqual(context.decode_scope(regular.raw), regular)


    def test_shared_history_admission_accepts_verified_family_but_not_unresolved_history(self):
        with modules() as (context, model):
            spec = importlib.util.spec_from_file_location('completion_inventory_under_test',
                ROOT / 'route_furniture_clock_inventory.py')
            module = importlib.util.module_from_spec(spec)
            with patch.dict(sys.modules, completion_inventory_under_test=module):
                spec.loader.exec_module(module)
            value = scope_value(model)
            state = dict(inventory_verified=True, inventory_digest='4' * 64, status='all_placed',
                         next_step=None, pending_step=None, stopped=False, placed=1)
            batch = types.SimpleNamespace(id=value['batch_id'], review=types.SimpleNamespace(digest=value['review_digest']),
                plan=types.SimpleNamespace(digest=value['plan_digest'], steps=(1,)),
                address=('127.0.0.1', 5000), source=SourcePort(**value['source']), audit=lambda: state,
                budget=object())
            scope = model.CompletionScope.create(value)
            for pending in (True, False):
                history = module.Inventory(Path('/private/clocks'), batch)
                history.check = lambda: None  # Custody is an explicit port in this test.
                history._rows = ({'resolved': not pending},)
                history._groups = 1
                if pending:
                    with self.assertRaises(ValueError): history.admit_new(scope)
                    self.assertEqual(history._allowed_new, frozenset())
                else:
                    sys.modules['bounded_run_outcomes'].SUFFIX = '.outcome.json'
                    history.admit_new(scope)
                    self.assertEqual(history._allowed_new, frozenset((
                        'progress-' + scope.digest + '.run',
                        'progress-' + scope.digest + '.run.scope.json',
                        'progress-' + scope.digest + '.run.outcome.json')))

    def test_independent_recovery_discovery_preserves_the_verification_phase_and_key(self):
        with modules() as (context, model):
            recovery = types.ModuleType('recover_route_furniture_clock')
            recovery.Budget = object
            storage = sys.modules['bounded_run_outcomes']
            storage.SUFFIX = '.outcome.json'
            spec = importlib.util.spec_from_file_location('completion_discovery_under_test',
                ROOT / 'scoped_clock_inventory.py')
            module = importlib.util.module_from_spec(spec)
            with patch.dict(sys.modules, completion_discovery_under_test=module,
                            recover_route_furniture_clock=recovery):
                spec.loader.exec_module(module)
            value = scope_value(model)
            final = {key: val for key, val in value.items() if key in context.FINAL_SCOPE_KEYS}
            final.update(schema=context.FINAL_SCHEMA, policy=context.FINAL_POLICY, phase='final_construction')
            regular = dict(final); regular.pop('phase')
            regular.update(schema=context.SCHEMA, policy=context.POLICY, next_step='chair')
            for scope, phase in ((model.CompletionScope.create(value), 'completion_verification'),
                                 (context.FinalScope.create(final), 'final_construction'),
                                 (context.Scope.create(regular), 'between_placements')):
                owner = types.SimpleNamespace(scope=scope, confirmation=scope.digest, batch_id=value['batch_id'],
                    path=Path('/private/clock/progress-' + scope.digest + '.run'), name='scope',
                    stamp=(1,), data=scope.raw,
                    store=types.SimpleNamespace(intent=scope.intent(), view=lambda: {}, source_stat=(1,),
                        source_data=b'original', outcome_stat=None, outcome_data=None))
                row = module._entry(owner).row()
                self.assertEqual(row['phase'], phase)
                self.assertEqual(row['clock_key'], scope.clock_key)
                self.assertTrue(row['unresolved'])


if __name__ == '__main__': unittest.main()
