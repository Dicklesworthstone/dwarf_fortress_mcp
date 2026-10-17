"""Original-entrance route proof against a separate finite graph oracle.

Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5; WP-05/07/10.
"""
from __future__ import annotations

from itertools import product
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import room_circulation as c


def fixture():
    return dict(floors=frozenset((x, 0, 1) for x in range(7)),
                clear=frozenset((x, 0, 1) for x in range(7)), uncertain=frozenset(),
                entries=(('room', (4, 0, 1)),),
                furnishings=(('bed', 'room', (6, 0, 1)),), connected=True)


def run(**changes):
    return c.assess(**(fixture() | changes), access_point=(0, 0, 1))


def closure(cells, root):
    reached = {root} & cells
    while True:
        next_set = reached | {p for p in cells if any(
            p[2] == q[2] and abs(p[0]-q[0]) + abs(p[1]-q[1]) == 1 for q in reached)}
        if next_set == reached:
            return reached
        reached = next_set


class AccessCirculationTests(unittest.TestCase):
    def test_blocked_entrance_corridor_cannot_pass_as_room_local_success(self):
        arguments = fixture()
        arguments['clear'] -= {(2, 0, 1)}
        self.assertEqual(c.assess(**arguments)['truth'], 'true')
        result = c.assess(**arguments, access_point=(0, 0, 1))
        self.assertEqual(result['truth'], 'false')
        self.assertEqual(result['counts'], {'true': 1, 'false': 2, 'unknown': 0})
        self.assertIsNone(result['records'][1]['route_digest'])
        self.assertFalse(result['all_required_routes_clear'])

    def test_unknown_corridor_is_not_clear_or_proven_unreachable(self):
        args = fixture()
        cut = (2, 0, 1)
        result = run(clear=args['clear']-{cut}, uncertain=frozenset({cut}))
        self.assertEqual(result['truth'], 'unknown')
        for row in result['records'][1:]:
            self.assertEqual(row['truth'], 'unknown')
            self.assertFalse(row['route_is_observed_clear'])
            self.assertIsNotNone(row['route_digest'])

    def test_all_routes_start_at_the_exact_original_point(self):
        result = run()
        self.assertEqual(result['policy'], c.ACCESS_POLICY)
        self.assertEqual([r['steps'] for r in result['records']], [0, 4, 5])
        self.assertEqual(result['access_point'], [0, 0, 1])
        self.assertTrue(result['project_access_required'])
        self.assertTrue(all(row['root_area'] is None for row in result['records']))
        self.assertEqual(result['records'][0]['kind'], 'project_access')
        for claim in ('point_is_existing_fortress_entrance_proven', 'native_pathfinding_proven',
                      'existing_fort_access_proven', 'continuous_access_proven',
                      'outside_project_routes_evaluated', 'game_effect_performed'):
            self.assertIs(result[claim], False)

    def test_blocked_and_uncertain_roots_cannot_be_replaced_with_room_entry(self):
        args = fixture()
        point = (0, 0, 1)
        self.assertEqual(run(clear=args['clear']-{point})['counts']['false'], 3)
        self.assertEqual(run(clear=args['clear']-{point}, uncertain=frozenset({point}))['counts']['unknown'], 3)

    def test_root_binding_changes_model_and_every_route_digest(self):
        args = fixture()
        a = c.assess(**args, access_point=(0, 0, 1))
        b = c.assess(**args, access_point=(1, 0, 1))
        self.assertNotEqual(a['model_digest'], b['model_digest'])
        self.assertNotEqual(a['evidence_digest'], b['evidence_digest'])
        self.assertTrue(all(x['route_digest'] != y['route_digest'] for x, y in zip(a['records'], b['records'])))

    def test_no_ninth_area_or_phantom_furnishing_is_needed(self):
        floors = frozenset((x, y, 1) for x in range(32) for y in range(16))
        entries = tuple((f'a{i}', (0, i, 1)) for i in range(8))
        targets = [(x, y, 1) for x in (2, 4, 6, 8) for y in range(1, 16, 2)]
        items = tuple((f'bed{i:02}', f'a{i%8}', p) for i, p in enumerate(targets))
        result = c.assess(floors, floors, frozenset(), entries, items,
                          connected=True, access_point=(31, 15, 1))
        self.assertEqual(result['truth'], 'true')
        self.assertEqual((result['selected_areas'], result['selected_furnishings']), (8, 32))
        self.assertEqual(len(result['records']), 41)
        self.assertLessEqual(len(c._canonical(result)), c.MAX_OUTPUT)
        self.assertLessEqual(result['work_units'], c.MAX_WORK)

    def test_malformed_foreign_obstructed_and_nonconnected_access_is_refused(self):
        for point in ([0, 0, 1], (False, 0, 1), (0, 0), (32768, 0, 1),
                      (0, 0, 2), (9, 0, 1), (6, 0, 1), 'root'):
            with self.subTest(point=point), self.assertRaises(ValueError):
                c.assess(**fixture(), access_point=point)
        with self.assertRaises(ValueError):
            c.assess(**(fixture() | {'connected': False}), access_point=(0, 0, 1))

    def test_furnishings_cannot_be_traversed_even_with_clear_native_occupancy(self):
        result = run(furnishings=(('blocker', 'room', (2, 0, 1)), ('bed', 'room', (6, 0, 1))))
        self.assertEqual(result['truth'], 'false')
        self.assertEqual(result['records'][1]['truth'], 'false')
        self.assertEqual(result['records'][2]['truth'], 'false')

    def test_every_ternary_small_grid_matches_independent_entrance_oracle(self):
        floors = frozenset((x, y, 1) for x in range(3) for y in range(3))
        root, entry, target = (0, 0, 1), (2, 0, 1), (2, 2, 1)
        varying = sorted(floors - {target})  # Includes root AND room entry.
        neighbors = {(1, 2, 1), (2, 1, 1)}
        for colors in product(range(3), repeat=len(varying)):
            clear = frozenset({target} | {p for p, n in zip(varying, colors) if n == 0})
            uncertain = frozenset(p for p, n in zip(varying, colors) if n == 1)
            known = closure(clear-{target}, root)
            possible = closure((clear | uncertain)-{target}, root)
            expected = ['true' if known & wanted else 'unknown' if possible & wanted else 'false'
                        for wanted in ({root}, {entry}, neighbors)]
            result = c.assess(floors, clear, uncertain, (('room', entry),),
                              (('bed', 'room', target),), connected=True, access_point=root)
            self.assertEqual([r['truth'] for r in result['records']], expected, colors)
            overall = 'false' if 'false' in expected else 'unknown' if 'unknown' in expected else 'true'
            self.assertEqual(result['truth'], overall, colors)

    def test_exact_budget_and_all_guard_interruptions(self):
        calls = []
        first = run(guard=lambda: calls.append(1))
        self.assertEqual(run(max_work=first['work_units']), first)
        with self.assertRaises(ValueError):
            run(max_work=first['work_units']-1)
        for boundary in range(len(calls)):
            remaining = boundary
            def guard():
                nonlocal remaining
                if remaining == 0:
                    raise InterruptedError('cancelled')
                remaining -= 1
            with self.subTest(boundary=boundary), self.assertRaises(InterruptedError):
                run(guard=guard)

    def test_input_order_and_process_hash_seed_preserve_evidence_bytes(self):
        floors = frozenset((x, y, 1) for x in range(8) for y in range(3))
        entries = (('z', (4, 0, 1)), ('a', (4, 2, 1)))
        items = (('z.bed', 'z', (7, 0, 1)), ('a.bed', 'a', (7, 2, 1)))
        a = c.assess(floors, floors, frozenset(), entries, items, connected=True, access_point=(0, 1, 1))
        b = c.assess(floors, floors, frozenset(), entries[::-1], items[::-1], connected=True, access_point=(0, 1, 1))
        self.assertEqual(a, b)
        code = 'from test_room_access_circulation import run; import json; print(json.dumps(run(),sort_keys=True))'
        env = dict(os.environ, PYTHONPATH=os.pathsep.join([str(Path(c.__file__).parent), str(Path(__file__).parent)]))
        outputs = [subprocess.check_output([sys.executable, '-c', code], env=dict(env, PYTHONHASHSEED=seed), timeout=10)
                   for seed in ('1', '97', 'random')]
        self.assertEqual(outputs[0], outputs[1])
        self.assertEqual(outputs[0], outputs[2])

    def test_explicit_none_keeps_legacy_shape(self):
        args = fixture()
        self.assertEqual(c.assess(**args), c.assess(**args, access_point=None))
        self.assertNotIn('project_access_required', c.assess(**args))
        self.assertEqual(c.assess(**args)['policy'], c.POLICY)


if __name__ == '__main__':
    unittest.main()
