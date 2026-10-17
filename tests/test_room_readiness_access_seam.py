"""Execute the real projection function with an injected canonical selection.

This intentionally does not test native decoding, full goal custody or the CLI.
Those integration cases live in test_room_project_access_readiness.py.
"""
from __future__ import annotations

import ast
import hashlib
import json
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest

SCRIPTS = Path(__file__).resolve().parents[1] / 'scripts'
sys.path.insert(0, str(SCRIPTS))
import room_circulation
from room_readiness_policy import ReadinessPolicy


def require(value, message):
    if not value:
        raise ValueError(message)


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True).encode('ascii')


def projection():
    # Compile the actual source function, not a copied or reimplemented reducer.
    module = ast.parse((SCRIPTS / 'room_readiness.py').read_text())
    definition = next(node for node in module.body if isinstance(node, ast.FunctionDef) and node.name == '_circulation')
    source = ast.Module(body=[ast.ImportFrom(module='__future__', names=[ast.alias(name='annotations')], level=0), definition], type_ignores=[])
    namespace = dict(hashlib=hashlib, canonical=canonical, require=require,
                     ReadinessPolicy=ReadinessPolicy, room_circulation=room_circulation)
    exec(compile(ast.fix_missing_locations(source), str(SCRIPTS / 'room_readiness.py'), 'exec'), namespace)
    return namespace


def fixture(*, access=True, blocked=None, units=False):
    region = SimpleNamespace(origin=(10, 10, 2), size=(7, 1, 1))
    floors = frozenset((x, 10, 2) for x in range(10, 17))
    body = {'intent': {'connect_areas': True, 'access': {'point': [10, 10, 2]}},
            'areas': [{'name': 'room', 'entry_point': [14, 10, 2],
                       'units': [{'slots': ['bed']}]}],
            'furniture_request': {'slots': [{'name': 'bed', 'target': [16, 10, 2]}]}}
    plan = SimpleNamespace(json=lambda: body, digest='b' * 64)
    selected = SimpleNamespace(plan=plan, floors=floors, region=region)
    goal = SimpleNamespace(room=SimpleNamespace(room_plan=plan), require_circulation=True,
                           require_project_access=access)
    tiles = []
    for x in range(10, 17):
        building = int(x == 16 or (x == blocked and not units))
        unit = int(x == blocked and units)
        tiles.append(SimpleNamespace(presence=2, attributes=(42, 3, 0, 0, 0, 0, building, unit, 1, 10015, 10015)))
    capture = SimpleNamespace(region=region, tiles=tiles, witness='a' * 64)
    return selected, goal, capture


def assess(selected, goal, capture, guard=lambda: None):
    namespace = projection()
    def select(plan, work):
        require(plan is selected.plan, 'substituted original room selection')
        work()
        return selected
    namespace['terrain'] = SimpleNamespace(terrain=SimpleNamespace(selection=select))
    return namespace['_circulation'](goal, capture, guard)


class SourceProjectionTests(unittest.TestCase):
    def test_observed_building_before_room_blocks_new_policy_not_legacy(self):
        selected, goal, capture = fixture(blocked=12)
        result = assess(selected, goal, capture)
        self.assertEqual(result['truth'], 'false')
        self.assertEqual(result['access_point'], [10, 10, 2])
        goal.require_project_access = False
        self.assertEqual(assess(selected, goal, capture)['truth'], 'true')

    def test_unit_occupancy_only_supplies_potential_path(self):
        selected, goal, capture = fixture(blocked=12, units=True)
        self.assertEqual(assess(selected, goal, capture)['truth'], 'unknown')

    def test_original_point_and_source_witness_bound_together(self):
        selected, goal, capture = fixture()
        result = assess(selected, goal, capture)
        self.assertEqual(result['truth'], 'true')
        self.assertEqual(result['room_plan_digest'], selected.plan.digest)
        self.assertEqual(result['map_observation_witness'], capture.witness)
        expected = hashlib.sha256(b'dfmcp-room-circulation-source/1\0' + canonical({
            'map': capture.witness, 'room': selected.plan.digest,
            'evidence': result['evidence_digest']})).hexdigest()
        self.assertEqual(result['source_bound_digest'], expected)
        capture.witness = 'c' * 64
        self.assertNotEqual(assess(selected, goal, capture)['source_bound_digest'], expected)

    def test_missing_original_point_is_not_replaced_by_an_entry(self):
        selected, goal, capture = fixture()
        selected.plan.json()['intent'].pop('access')
        with self.assertRaises(ValueError): assess(selected, goal, capture)

    def test_capture_substitution_fails_before_assessment(self):
        selected, goal, capture = fixture()
        capture.region = SimpleNamespace(origin=(11, 10, 2), size=(7, 1, 1))
        with self.assertRaisesRegex(ValueError, 'capture differs'): assess(selected, goal, capture)

    def test_authority_cancellation_propagates(self):
        selected, goal, capture = fixture()
        count = []
        assess(selected, goal, capture, lambda: count.append(1))
        for stop in range(len(count)):
            left = stop
            def guard():
                nonlocal left
                if left == 0:
                    raise InterruptedError('cancelled projection')
                left -= 1
            with self.subTest(stop=stop), self.assertRaises(InterruptedError):
                assess(selected, goal, capture, guard)


if __name__ == '__main__':
    unittest.main()
