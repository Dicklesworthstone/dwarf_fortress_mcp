"""Execute actual reducer/projection bodies with injected validated producers.

This deliberately isolates source functions from native codecs and file custody.
It is NOT an end-to-end native, journal, or full-module qualification suite.
The separate test_room_readiness_circulation suite uses the production codecs.
Some adversarial child timing inputs deliberately bypass upstream validation;
native goals currently require at least two samples and a positive span.
"""
from __future__ import annotations

import argparse
import ast
from dataclasses import InitVar, dataclass, field, replace
import hashlib
import itertools
import json
from pathlib import Path
import sys
import struct
from types import ModuleType, SimpleNamespace as N
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import room_circulation


def require(ok, reason):
    if not ok:
        raise ValueError(reason)


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def load(path, names, supplied):
    parsed = ast.parse(path.read_text())
    nodes = [n for n in parsed.body if isinstance(n, (ast.FunctionDef, ast.ClassDef)) and n.name in names]
    require({n.name for n in nodes} == set(names), 'isolated source function missing')
    module = ModuleType('_room_isolated_' + path.stem)
    sys.modules[module.__name__] = module
    module.__dict__.update(supplied)
    body = [ast.ImportFrom(module='__future__', names=[ast.alias(name='annotations')], level=0)] + nodes
    exec(compile(ast.fix_missing_locations(ast.Module(body=body, type_ignores=[])), str(path), 'exec'), module.__dict__)
    return module


@dataclass(frozen=True)
class Finding:
    status: str = 'condition_met'
    building_type: int = 50
    stage: int = 3


class Goal:
    def __init__(self, strict=True, **timing):
        child = N(receipt=b'x'*32, record=N(insertion=N(building=1, item=2, job=3), plan=N(key='original')))
        self.condition = N(digest='construction', goals=(child,), deadline=10000,
                           interval=1, stable_samples=1, stable_span=0, max_gap=1200, max_observations=128)
        self.condition.__dict__.update(timing)
        self.require_circulation = strict
        self.digest = 'room-strict' if strict else 'room-legacy'
    def encode(self):
        return self
    @staticmethod
    def decode(raw, _guard):
        return raw


class Sample:
    def __init__(self, tick=300, shape=True, occupancy=True, access=True, status='condition_met',
                 map_source='map', map_change=False, horizons=(9, 9, 9)):
        observed = N(tick=tick, digest=str(tick), horizons=horizons, finding=Finding(status))
        self.furnishings = N(before=N(generation=1), operations=N(generation=2, software=('df', 'dfhack')),
                            validate=lambda goal, guard: observed)
        self.captured = N(tick=tick, witness='map-witness', binding=lambda: {'source': map_source})
        self.diagnosis = {'all_required_shapes_at_sample': shape,
                          'furniture_occupancy': {'all_targets_have_building_occupancy': occupancy},
                          'circulation': {'all_required_routes_clear': access}}
        self.digest = str((tick, shape, occupancy, access, map_change))
    def validate(self, goal, guard):
        guard()
        return self.captured, self.diagnosis


def reducers(path=None):
    common = dict(dataclass=dataclass, replace=replace, require=require, canonical=canonical, json=json,
                  hashlib=hashlib, idle=lambda: None)
    construction = load(ROOT/'scripts/construction_plan.py',
        ('Assessment', 'Progress', 'begin_read', 'cancel', 'advance'),
        common | {'POLICY': 'construction', 'assess': lambda child, observed, guard: observed.finding})
    room = load(path or ROOT/'scripts/room_readiness.py',
        ('Progress', 'begin_read', 'cancel', 'advance'),
        common | dict(construction=construction, Goal=Goal, Sample=Sample,
                      POLICY='legacy', CIRCULATION_POLICY='strict'))
    return room


class ReducerTests(unittest.TestCase):
    def setUp(self):
        self.r = reducers()
    def advance(self, goal, sample, state=None):
        p = self.r.Progress(goal.digest) if state is None else state
        return self.r.advance(self.r.begin_read(p), goal, sample)

    def test_joint_gate_cannot_inherit_hypothetical_child_only_terminal_state(self):
        for strict, shape, occupancy, access in itertools.product((False, True), repeat=4):
            goal = Goal(strict)
            p = self.advance(goal, Sample(shape=shape, occupancy=occupancy, access=access))
            expected = shape and occupancy and (not strict or access)
            self.assertEqual(p.phase == 'satisfied', expected, (strict, shape, occupancy, access))
            if not expected:
                self.assertEqual((p.phase, p.streak, p.first_tick), ('active', 0, None))
            self.assertEqual(p.observations, 1)

    def test_rejected_last_sample_expires_instead_of_manufacturing_success(self):
        for shape, occupancy, access in ((False, True, True), (True, False, True), (True, True, False)):
            p = self.advance(Goal(max_observations=1), Sample(shape=shape, occupancy=occupancy, access=access))
            self.assertEqual((p.phase, p.reason, p.streak), ('expired', 'sample_budget_exhausted', 0))

    def test_child_failure_identity_and_deadline_precedence_is_retained(self):
        for status, phase in (('removal_pending', 'failed'), ('building_missing', 'invalidated'),
                              ('source_regressed', 'invalidated')):
            p = self.advance(Goal(), Sample(access=False, status=status))
            self.assertEqual((p.phase, p.reason), (phase, status))
        p = self.advance(Goal(deadline=300), Sample(access=False))
        self.assertEqual((p.phase, p.reason), ('expired', 'game_deadline_reached'))

    def test_access_and_furnishings_must_share_the_whole_advancing_window(self):
        goal = Goal(stable_samples=2, stable_span=10)
        p = self.advance(goal, Sample())
        self.assertEqual(p.streak, 1)
        p = self.advance(goal, Sample(310, access=False), p)
        self.assertEqual((p.streak, p.reason), (0, 'original_room_circulation_not_ready'))
        p = self.advance(goal, Sample(320, status='pending'), p)
        self.assertEqual(p.streak, 0)
        p = self.advance(goal, Sample(330), p)
        self.assertEqual((p.streak, p.first_tick), (1, 330))
        p = self.advance(goal, Sample(340), p)
        self.assertEqual(p.phase, 'satisfied')

    def test_false_sample_retains_horizons_and_later_regression_invalidates(self):
        goal = Goal()
        p = self.advance(goal, Sample(access=False, horizons=(10, 10, 10)))
        self.assertEqual(p.horizons, (10, 10, 10))
        p = self.advance(goal, Sample(310, horizons=(9, 10, 10)), p)
        self.assertEqual((p.phase, p.reason), ('invalidated', 'native_horizon_regressed'))
        p = self.advance(goal, Sample(access=False))
        p = self.advance(goal, Sample(310, map_source='changed'), p)
        self.assertEqual((p.phase, p.reason), ('invalidated', 'room_map_source_changed'))

    def test_same_tick_change_and_interruption_cannot_carry_credit(self):
        goal = Goal(stable_samples=2, stable_span=10)
        p = self.advance(goal, Sample())
        p = self.advance(goal, Sample(map_change=True), p)
        self.assertEqual((p.streak, p.reason), (0, 'room_sample_changed_at_same_tick'))
        p = self.advance(goal, Sample(310), p)
        interrupted = self.r.begin_read(self.r.begin_read(p))
        self.assertEqual((interrupted.streak, interrupted.interruptions), (0, 1))
        self.assertEqual(self.r.cancel(interrupted).phase, 'cancelled')

    def test_guard_refusal_does_not_publish_partial_progress(self):
        goal, sample = Goal(), Sample()
        state = self.r.begin_read(self.r.Progress(goal.digest))
        calls = []
        self.r.advance(state, goal, sample, lambda: calls.append(1))
        for boundary in range(len(calls)):
            before = state.view()
            remaining = [boundary]
            def guard():
                if not remaining[0]:
                    raise InterruptedError('revoked')
                remaining[0] -= 1
            self.assertRaises(InterruptedError, self.r.advance, state, goal, sample, guard)
            self.assertEqual(state.view(), before)

    def test_projection_distinguishes_units_hidden_buildings_liquid_and_clear_floor(self):
        floors = frozenset((x, 0, 1) for x in range(5))
        region = N(origin=(0, 0, 1), size=(5, 1, 1))
        body = {'areas': [{'name': 'room', 'entry_point': [0, 0, 1],
                          'units': [{'slots': ['bed']}]}],
                'furniture_request': {'slots': [{'name': 'bed', 'target': [4, 0, 1]}]},
                'intent': {'connect_areas': True}}
        plan = N(json=lambda: body, digest='plan')
        selected = N(floors=floors, region=region, plan=plan)
        module = load(ROOT/'scripts/room_readiness.py', ('_circulation',),
            dict(terrain=N(terrain=N(selection=lambda plan, guard: selected)), require=require,
                 canonical=canonical, hashlib=hashlib, room_circulation=room_circulation))
        goal = N(room=N(room_plan=plan))
        def tile(presence=2, shape=3, depth=0, magma=0, dig=0, building=0, units=0):
            return N(presence=presence, attributes=(42, shape, depth, magma, 0, dig, building, units))
        for obstacle, truth in ((tile(), 'true'), (tile(presence=0), 'unknown'),
                                (tile(presence=1), 'unknown'), (tile(units=1), 'unknown'),
                                (tile(building=1), 'false'), (tile(shape=2), 'false'),
                                (tile(depth=1), 'false'), (tile(magma=1), 'false'), (tile(dig=1), 'false')):
            capture = N(region=region, witness='capture', tiles=[tile(), tile(), obstacle, tile(), tile(building=1)])
            report = module._circulation(goal, capture, lambda: None)
            self.assertEqual(report['truth'], truth)
            self.assertEqual(report['map_observation_witness'], 'capture')
            self.assertEqual(len(report['records']), 2)
            self.assertEqual(len(report['source_bound_digest']), 64)

    def test_goal_framing_preserves_legacy_bytes_and_binds_strict_policy(self):
        # Only framing/policy code is under test here. The opaque child payloads
        # stand in for already validated original custody and placement codecs.
        class Reader:
            def __init__(self, raw, maximum):
                require(type(raw) is bytes and len(raw) <= maximum, 'extent')
                self.raw, self.offset = raw, 0
            def take(self, n):
                require(self.offset + n <= len(self.raw), 'truncated')
                result = self.raw[self.offset:self.offset+n]
                self.offset += n
                return result
            def u32(self, maximum):
                n = int.from_bytes(self.take(4), 'big')
                require(n <= maximum, 'extent')
                return n
            def finish(self):
                require(self.offset == len(self.raw), 'trailing')
        def integer(n, low, high):
            require(type(n) is int and low <= n <= high, 'integer')
            return n
        class Condition:
            def __init__(self, *args):
                self.goals, self.receipts = (), ()
                self.deadline = self.interval = self.stable_samples = self.stable_span = 1
                self.max_gap = self.max_observations = 1
            def encode(self):
                return b'condition'
            @classmethod
            def decode(cls, raw):
                require(raw == b'condition', 'child payload')
                return cls()
        class Room:
            def __init__(self):
                self.allocation = N(plan=lambda: N(steps=()))
                self.room_plan = None
            def encode(self):
                return b'room'
            @classmethod
            def decode(cls, raw, guard):
                require(raw == b'room', 'child payload')
                return cls()
        supplied = dict(dataclass=dataclass, InitVar=InitVar, derived=field, struct=struct,
                        hashlib=hashlib, require=require, integer=integer, Cursor=Reader,
                        idle=lambda: None, MAX_HANDOFF=100, MAX_ORIGIN=100, MAX_GOAL=1024,
                        construction=N(Goal=Condition, MAX_GOAL=100),
                        map_wire=N(integer=integer), RoomFurnitureHandoff=Room,
                        terrain=N(RoomTerrainGoal=lambda *a, **k: None))
        room = load(ROOT/'scripts/room_readiness.py', ('Goal', '_blob', '_read_blob'), supplied)
        legacy = room.Goal(Room(), Condition())
        strict = room.Goal(Room(), Condition(), require_circulation=True)
        body = struct.pack('>I', 4) + b'room' + struct.pack('>I', 9) + b'condition'
        self.assertEqual(legacy.encode(), b'DFMRDG01' + body)
        self.assertEqual(strict.encode(), b'DFMRDG02' + body)
        self.assertNotEqual(legacy.digest, strict.digest)
        class Origin:
            receipts = ()
            room_handoff = Room()
            def encode(self):
                return b'origin'
            @classmethod
            def decode(cls, raw, guard):
                require(raw == b'origin', 'origin payload')
                return cls()
        store = load(ROOT/'scripts/room_readiness_store.py', ('Goal', '_blob', '_read_blob', '_idle'),
            supplied | dict(Origin=Origin, readiness=room, GOAL_MAGIC=b'DFMRDC01',
                            CIRCULATION_GOAL_MAGIC=b'DFMRDC02', GOAL_DOMAIN=b'domain'))
        old = store.Goal(Origin(), Condition())
        new = store.Goal(Origin(), Condition(), require_circulation=True)
        self.assertEqual(old.encode()[8:], new.encode()[8:])
        self.assertEqual(old.encode()[:8], b'DFMRDC01')
        self.assertEqual(new.encode()[:8], b'DFMRDC02')
        self.assertTrue(new.readiness_goal.require_circulation)
        for value, owner in ((legacy, room), (strict, room), (old, store), (new, store)):
            decoded = owner.Goal.decode(value.encode())
            self.assertEqual(decoded.encode(), value.encode())
            self.assertEqual(decoded.require_circulation, value.require_circulation)
            for raw in (value.encode()[:-1], value.encode()+b'x', b'INVALID!'+value.encode()[8:]):
                self.assertRaises(ValueError, owner.Goal.decode, raw)
        self.assertRaises(ValueError, room.Goal, Room(), Condition(), require_circulation=1)
        self.assertRaises(ValueError, store.Goal, Origin(), Condition(), require_circulation=1)
        object.__setattr__(new, 'require_circulation', False)
        self.assertRaises(ValueError, new.encode)

    def test_cli_reopening_cannot_replace_original_circulation_policy(self):
        contacted = []
        def forbidden(*args, **kwargs):
            contacted.append(True)
            raise AssertionError('resource opened before policy refusal')
        module = load(ROOT/'scripts/track_room_readiness.py', ('Parser', 'main'),
            dict(argparse=argparse, Rejected=ValueError, require=require,
                 foreground=N(add_arguments=lambda p: None, parse_limits=lambda a: None),
                 Budget=lambda timeout: None, Journal=forbidden, Batch=forbidden,
                 packet=lambda *a, **k: {}, output=lambda v: b'{}', publish=lambda raw: 0))
        for operation in ('sample', 'wait', 'inspect', 'cancel'):
            self.assertEqual(module.main([operation, '--journal', 'unused', '--require-circulation']), 2)
        self.assertEqual(contacted, [])


if __name__ == '__main__':
    unittest.main()
