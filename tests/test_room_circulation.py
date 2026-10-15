"""Actual bounded circulation engine against an independent closure oracle."""
import itertools
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import room_circulation as c


def line():
    return frozenset((x, 0, 1) for x in range(5))


def evaluate(floors=None, clear=None, uncertain=frozenset(), **kwargs):
    floors = line() if floors is None else floors
    return c.assess(floors, floors if clear is None else clear, uncertain,
                    (('room', (0, 0, 1)),), (('bed', 'room', (4, 0, 1)),),
                    connected=False, **kwargs)


def closure(cells, root):
    # Independent repeated set closure, not the implementation's BFS or parents.
    reached = {root} & cells
    while True:
        following = reached | {p for p in cells if any(
            p[2] == q[2] and abs(p[0]-q[0]) + abs(p[1]-q[1]) == 1 for q in reached)}
        if following == reached:
            return reached
        reached = following


class CirculationTests(unittest.TestCase):
    def test_every_ternary_3x3_observation_matches_independent_closure(self):
        floors = frozenset((x, y, 1) for x in range(3) for y in range(3))
        root, target = (0, 0, 1), (2, 2, 1)
        varying = sorted(floors - {root, target})
        for colors in itertools.product(range(3), repeat=len(varying)):
            clear = frozenset({root, target} | {p for p, n in zip(varying, colors) if n == 0})
            uncertain = frozenset(p for p, n in zip(varying, colors) if n == 1)
            real = c.assess(floors, clear, uncertain, (('room', root),),
                            (('bed', 'room', target),), connected=True)
            approaches = {(1, 2, 1), (2, 1, 1)}
            expected = ('true' if closure(clear - {target}, root) & approaches else
                        'unknown' if closure((clear | uncertain) - {target}, root) & approaches else 'false')
            self.assertEqual(real['truth'], expected, colors)
            self.assertEqual(real['records'][1]['truth'], expected, colors)
            self.assertEqual(sum(real['counts'].values()), 2)

    def test_clear_unknown_and_blocked_cut_have_distinct_truth(self):
        self.assertEqual(evaluate()['truth'], 'true')
        cut = (2, 0, 1)
        clear = line() - {cut}
        self.assertEqual(evaluate(clear=clear)['truth'], 'false')
        possible = evaluate(clear=clear, uncertain=frozenset({cut}))
        self.assertEqual(possible['truth'], 'unknown')
        self.assertFalse(possible['records'][1]['route_is_observed_clear'])
        self.assertIsNotNone(possible['records'][1]['route_digest'])

    def test_unknown_or_blocked_entry_cannot_bootstrap_a_clear_route(self):
        root = (0, 0, 1)
        self.assertEqual(evaluate(clear=line()-{root})['truth'], 'false')
        result = evaluate(clear=line()-{root}, uncertain=frozenset({root}))
        self.assertEqual(result['truth'], 'unknown')
        self.assertEqual(result['counts']['unknown'], 2)

    def test_furniture_cells_never_become_a_corridor(self):
        result = c.assess(line(), line(), frozenset(), (('room', (0, 0, 1)),),
                          (('a', 'room', (2, 0, 1)), ('b', 'room', (4, 0, 1))), connected=True)
        self.assertEqual([r['truth'] for r in result['records']], ['true', 'true', 'false'])
        self.assertEqual(result['truth'], 'false')
        self.assertIsNone(result['records'][-1]['route_digest'])

    def test_connected_project_checks_every_original_entry(self):
        floors = frozenset((x, y, 1) for x in range(6) for y in range(2))
        entries = (('a', (0, 0, 1)), ('b', (5, 0, 1)))
        items = (('one', 'a', (0, 1, 1)), ('two', 'b', (5, 1, 1)))
        clear = floors - {(3, 0, 1), (3, 1, 1)}
        independent = c.assess(floors, clear, frozenset(), entries, items, connected=False)
        connected = c.assess(floors, clear, frozenset(), entries, items, connected=True)
        self.assertEqual(independent['truth'], 'true')
        self.assertEqual(connected['truth'], 'false')
        self.assertEqual(connected['records'][1]['name'], 'b')
        self.assertEqual(connected['records'][1]['truth'], 'false')
        self.assertTrue(connected['records_complete'])
        self.assertEqual(len(connected['records']), 4)

    def test_detour_finds_observed_alternative_instead_of_latching_a_planned_approach(self):
        floors = frozenset((x, y, 1) for x in range(5) for y in range(3))
        result = evaluate(floors=floors, clear=floors-{(2, 0, 1)})
        self.assertEqual(result['truth'], 'true')
        self.assertEqual(result['records'][1]['steps'], 5)

    def test_complete_selection_preserves_false_over_unknown(self):
        floors = frozenset((x, 0, 1) for x in range(8))
        items = (('unknown', 'room', (3, 0, 1)), ('blocked', 'room', (7, 0, 1)))
        result = c.assess(floors, floors-{(2, 0, 1), (5, 0, 1)}, frozenset({(2, 0, 1)}),
                          (('room', (0, 0, 1)),), items, connected=False)
        self.assertEqual(result['truth'], 'false')
        self.assertEqual(result['counts'], {'true': 1, 'false': 1, 'unknown': 1})
        self.assertEqual(len(result['records']), 3)

    def test_ordering_is_independent_of_input_order_and_hash_seed(self):
        floors = frozenset((x, y, 1) for x in range(5) for y in range(3))
        entries = (('a', (0, 0, 1)), ('z', (0, 2, 1)))
        items = (('a.bed', 'a', (4, 0, 1)), ('z.bed', 'z', (4, 2, 1)))
        first = c.assess(floors, floors, frozenset(), entries, items, connected=True)
        self.assertEqual(first, c.assess(floors, floors, frozenset(), entries[::-1], items[::-1], connected=True))
        script = "from test_room_circulation import evaluate; import json; print(json.dumps(evaluate(),sort_keys=True))"
        env = dict(os.environ, PYTHONPATH=os.pathsep.join([str(Path(c.__file__).parent), str(Path(__file__).parent)]))
        outputs = [subprocess.check_output([sys.executable, '-c', script], env=dict(env, PYTHONHASHSEED=seed))
                   for seed in ('1', '97')]
        self.assertEqual(*outputs)

    def test_exact_work_limit_and_every_guard_interruption(self):
        calls = []
        result = evaluate(guard=lambda: calls.append(1))
        self.assertEqual(evaluate(max_work=result['work_units']), result)
        self.assertRaises(ValueError, evaluate, max_work=result['work_units']-1)
        for boundary in range(len(calls)):
            left = [boundary]
            def guard():
                if left[0] == 0:
                    raise InterruptedError('cancelled')
                left[0] -= 1
            self.assertRaises(InterruptedError, evaluate, guard=guard)

    def test_512_tiles_8_areas_32_furnishings_return_every_row(self):
        floors = frozenset((x, y, 1) for x in range(32) for y in range(16))
        entries = tuple((f'a{i}', (0, i, 1)) for i in range(8))
        targets = [(x, y, 1) for x in (2, 4, 6, 8) for y in range(1, 16, 2)]
        items = tuple((f'bed{i:02}', f'a{i%8}', p) for i, p in enumerate(targets))
        result = c.assess(floors, floors, frozenset(), entries, items, connected=False)
        self.assertEqual(result['truth'], 'true')
        self.assertEqual(len(result['records']), 40)
        self.assertLessEqual(result['work_units'], c.MAX_WORK)
        self.assertLessEqual(len(json.dumps(result)), c.MAX_OUTPUT)

    def test_malformed_inputs_and_implicit_stairs_refuse(self):
        good = dict(floors=line(), clear=line(), uncertain=frozenset(),
                    entries=(('room', (0, 0, 1)),), furnishings=(('bed', 'room', (4, 0, 1)),), connected=False)
        cases = [dict(floors=set(line())), dict(clear=frozenset({(9, 9, 9)})),
                 dict(uncertain=frozenset({(1, 0, 1)})), dict(connected=1), dict(max_work=True),
                 dict(max_work=0), dict(entries=()), dict(furnishings=()),
                 dict(entries=(('room', (False, 0, 1)),)), dict(entries=(('r/oom', (0, 0, 1)),)),
                 dict(entries=good['entries']*2), dict(furnishings=good['furnishings']*2),
                 dict(furnishings=(('bed', 'absent', (4, 0, 1)),)),
                 dict(furnishings=(('bed', 'room', (0, 0, 1)),)),
                 dict(floors=line() | {(1, 0, 2)}, connected=True),
                 dict(floors=frozenset((x, 0, 1) for x in range(513)))]
        for change in cases:
            with self.subTest(change=change), self.assertRaises(ValueError):
                c.assess(**(good | change))

    def test_sample_identity_changes_with_each_load_bearing_input(self):
        first = evaluate()
        changed = [evaluate(clear=line()-{(2, 0, 1)}),
                   evaluate(clear=line()-{(2, 0, 1)}, uncertain=frozenset({(2, 0, 1)})),
                   evaluate(floors=line() | {(0, 1, 1)})]
        self.assertEqual(len({v['model_digest'] for v in [first] + changed}), 4)
        self.assertEqual(len({v['evidence_digest'] for v in [first] + changed}), 4)
        for v in changed:
            for key in ('native_pathfinding_proven', 'outside_project_routes_evaluated',
                        'existing_fort_access_proven', 'continuous_access_proven', 'game_effect_performed'):
                self.assertIs(v[key], False)


if __name__ == '__main__':
    unittest.main()
