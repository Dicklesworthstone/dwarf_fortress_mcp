"""Independent constrained-path oracles; actual connector and budget composition."""
from copy import deepcopy
from dataclasses import replace
import itertools
import json
from pathlib import Path
import random
import subprocess
import sys
import os
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import room_connections as original
import room_cost_connections as costs
import room_bounded_cost_paths as bounded


def fixture():
    return dict(floors={(1, 3, 1), (7, 3, 1)},
                blocked={(2, 1, 1), (3, 3, 1), (4, 3, 1), (5, 3, 1), (6, 5, 1)},
                entries=(('a', (1, 3, 1)), ('b', (7, 3, 1))),
                costs=costs.Costs(((1, 1, 1), (7, 5, 1)),
                    '01101011111100110011001000000001100', costs.BOUNDED_STRATEGY),
                max_new_tiles=7)


def oracle(floors, blocked, charged, size, allowance):
    """Enumerate simple paths directly; no heap, labels or runtime helpers."""
    start, target = sorted(floors)
    free = {(x, y, 1) for x in range(1, size[0] + 1) for y in range(1, size[1] + 1)} - blocked
    pending, best = [(start, {start})], None
    while pending:
        point, seen = pending.pop()
        added = seen - floors
        if len(added) > allowance:
            continue
        if point == target:
            value = len(added & charged), len(added), len(seen) - 1
            best = value if best is None else min(best, value)
            continue
        for following in free - seen:
            if sum(abs(a - b) for a, b in zip(point, following)) == 1:
                pending.append((following, seen | {following}))
    return best


def direct(floors, blocked, charged, size, allowance):
    cells = ''.join('1' if (x, y, 1) in charged else '0'
                    for y in range(1, size[1] + 1) for x in range(1, size[0] + 1))
    model = costs.Costs(((1, 1, 1), (*size, 1)), cells, costs.BOUNDED_STRATEGY)
    path = bounded.link(floors, {min(floors)}, blocked, model, allowance,
                        original._Work(original.idle, original.MAX_WORK))
    if path is None:
        return None
    added = set(path) - floors
    return sum(model.at(p) for p in added), len(added), len(path) - 1


class BoundedCostPathsTests(unittest.TestCase):
    def test_complete_budgeted_route_reduces_six_charged_cells_to_one(self):
        args = fixture()
        legacy = costs.connect(**{**args, 'costs': replace(args['costs'], strategy=costs.STRATEGY)})
        selected = costs.connect(**args)
        self.assertEqual((legacy.selected_cost, len(legacy.added), legacy.weighted_status), (6, 7, 'tile_limit'))
        self.assertEqual((selected.selected_cost, len(selected.added), selected.selected_strategy),
                         (1, 7, 'bounded_cost_first'))
        charged = {(x, y, 1) for x in range(1, 8) for y in range(1, 6) if args['costs'].at((x, y, 1))}
        self.assertEqual(oracle(args['floors'], args['blocked'], charged, (7, 5), 7), (1, 7, 8))
        self.assertFalse(set(selected.added) & (args['floors'] | args['blocked']))

    def test_all_2187_worlds_at_every_allowance_match_exhaustive_simple_paths(self):
        floors = {(1, 1, 1), (3, 3, 1)}
        variable = sorted({(x, y, 1) for x in range(1, 4) for y in range(1, 4)} - floors)
        for states in itertools.product(range(3), repeat=7):
            blocked = {p for p, s in zip(variable, states) if s == 2}
            charged = {p for p, s in zip(variable, states) if s == 1}
            for allowance in range(8):
                self.assertEqual(direct(floors, blocked, charged, (3, 3), allowance),
                                 oracle(floors, blocked, charged, (3, 3), allowance), (states, allowance))

    def test_seeded_larger_worlds_keep_resource_cost_tradeoffs(self):
        rng = random.Random(61019)
        floors = {(1, 1, 1), (4, 4, 1)}
        variable = sorted({(x, y, 1) for x in range(1, 5) for y in range(1, 5)} - floors)
        for index in range(200):
            states = [rng.randrange(3) for _ in variable]
            blocked = {p for p, s in zip(variable, states) if s == 2}
            charged = {p for p, s in zip(variable, states) if s == 1}
            allowance = rng.randrange(15)
            self.assertEqual(direct(floors, blocked, charged, (4, 4), allowance),
                             oracle(floors, blocked, charged, (4, 4), allowance), index)

    def test_larger_allowance_cannot_increase_two_terminal_minimum_cost(self):
        args = fixture(); model = args['costs']; last = None
        for allowance in range(5, 16):
            path = bounded.link(args['floors'], {min(args['floors'])}, args['blocked'], model,
                                allowance, original._Work(original.idle, original.MAX_WORK))
            if path is not None:
                score = sum(model.at(p) for p in set(path) - args['floors'])
                if last is not None:
                    self.assertLessEqual(score, last)
                last = score
        self.assertEqual(last, 0)

    def test_dominance_keeps_more_expensive_shorter_prefixes(self):
        a = bounded._Label((1, 1, 1), 0, 5, 5, None)
        b = bounded._Label((1, 1, 1), 1, 3, 7, None)
        self.assertFalse(bounded._dominates(a, b))
        self.assertFalse(bounded._dominates(b, a))
        self.assertTrue(bounded._dominates(replace(a, tiles=3), b))
        self.assertTrue(bounded._dominates(b, replace(b, steps=8)))
        self.assertFalse(bounded._dominates(replace(b, steps=8), b))

    def test_complete_floor_components_and_exact_partition_are_retained(self):
        floors = {(1, 1, 1), (7, 1, 1), (1, 5, 1), (7, 5, 1), (7, 4, 1)}
        model = replace(fixture()['costs'], cells='0101010' * 5)
        result = costs.connect(floors, set(), tuple((str(i), p) for i, p in enumerate(sorted(floors))),
                               model, max_new_tiles=25)
        expanded = []
        for (x, y, z), (w, h, levels) in result.parts:
            self.assertEqual(levels, 1)
            expanded.extend((xx, yy, z) for yy in range(y, y + h) for xx in range(x, x + w))
        self.assertEqual(len(expanded), len(set(expanded)))
        self.assertEqual(set(expanded), set(result.added))
        complete = floors | set(expanded)
        self.assertEqual(original._flood(complete, min(floors), original._Work(original.idle, original.MAX_WORK)), complete)

    def test_multiple_connected_source_tiles_cost_zero(self):
        floors = {(1, y, 1) for y in range(1, 6)} | {(7, 3, 1)}
        model = replace(fixture()['costs'], cells='1' * 35)
        connected = floors - {(7, 3, 1)}
        path = bounded.link(floors, connected, set(), model, 5, original._Work(original.idle, original.MAX_WORK))
        self.assertEqual(path[0], (1, 3, 1))
        self.assertEqual(len(set(path) - floors), 5)
        self.assertEqual(len(path), 7)

    def test_part_limit_keeps_only_a_complete_eligible_candidate(self):
        args = fixture()
        for limit in range(4):
            try:
                selected = costs.connect(**args, max_parts=limit)
            except ValueError:
                self.assertLess(limit, 3)
            else:
                self.assertLessEqual(len(selected.parts), limit)
                self.assertLessEqual(len(selected.added), args['max_new_tiles'])

    def test_every_checkpoint_propagates_cancellation_after_baseline_too(self):
        args = fixture(); complete = costs.connect(**args)
        for stop in range(1, complete.work_units + 1):
            seen = 0
            def guard():
                nonlocal seen
                seen += 1
                if seen == stop:
                    raise ValueError('external cancellation')
            with self.assertRaisesRegex(ValueError, 'external cancellation'):
                costs.connect(**args, guard=guard)
            self.assertEqual(seen, stop)

    def test_shared_work_boundary_never_returns_the_baseline_after_refusal(self):
        args = fixture(); complete = costs.connect(**args)
        self.assertEqual(costs.connect(**args, max_work=complete.work_units), complete)
        for limit in (1, complete.work_units // 2, complete.work_units - 1):
            with self.assertRaises(original.WorkLimit):
                costs.connect(**args, max_work=limit)

    def test_label_inventory_exhaustion_is_inconclusive_not_a_fallback(self):
        with patch.object(bounded, 'MAX_LABELS', 3), self.assertRaisesRegex(original.WorkLimit, 'label allowance'):
            costs.connect(**fixture())

    def test_strategy_is_closed_exact_and_legacy_default_is_canonical(self):
        value = fixture()['costs'].json()
        for strategy in (True, None, 1, [], 'bounded', 'cost-first-with-original-candidate/2'):
            with self.assertRaises(ValueError):
                costs.Costs.parse({**value, 'strategy': strategy})
        old = costs.Costs.parse({k: v for k, v in value.items() if k != 'strategy'})
        self.assertEqual(old, costs.Costs.parse({**old.json(), 'strategy': costs.STRATEGY}))
        self.assertNotIn('strategy', old.json())
        self.assertNotEqual(old.digest, fixture()['costs'].digest)

    def test_hard_limits_and_original_inputs_are_never_relaxed(self):
        args = fixture(); before = deepcopy(args)
        costs.connect(**args)
        self.assertEqual(args, before)
        for limit in (-1, True, 512, 1.0):
            with self.assertRaises(ValueError):
                costs.connect(**{**args, 'max_new_tiles': limit})
        with self.assertRaises(ValueError):
            costs.connect(**{**args, 'blocked': args['blocked'] | {(1, 3, 1)}})
        with self.assertRaises(ValueError):
            costs.connect(**args, root=(True, 3, 1))

    def test_all_candidates_are_refused_when_original_baseline_is_unestablished(self):
        args = fixture(); args['blocked'] = {(4, y, 1) for y in range(1, 6)}
        with self.assertRaises(ValueError):
            costs.connect(**args)

    def test_hash_seed_and_input_iteration_do_not_change_result(self):
        args = fixture(); expected = costs.connect(**args).summary()
        reverse = {**args, 'floors': frozenset(reversed(sorted(args['floors']))),
                   'blocked': frozenset(reversed(sorted(args['blocked']))), 'entries': tuple(reversed(args['entries']))}
        self.assertEqual(costs.connect(**reverse).summary(), expected)
        code = "from test_room_bounded_cost_paths import fixture; import room_cost_connections,json; print(json.dumps(room_cost_connections.connect(**fixture()).summary(),sort_keys=True))"
        paths = os.pathsep.join((str(Path(__file__).parent), str(Path(__file__).parents[1] / 'scripts')))
        for seed in ('1', '17', 'random'):
            out = subprocess.run([sys.executable, '-c', code], capture_output=True, check=True, timeout=15,
                env={**os.environ, 'PYTHONPATH': paths, 'PYTHONHASHSEED': seed, 'PYTHONDONTWRITEBYTECODE': '1'})
            self.assertEqual(json.loads(out.stdout), expected)

    def test_result_does_not_claim_global_network_or_native_proof(self):
        out = costs.connect(**fixture()).summary()
        self.assertEqual(out['policy'], costs.BOUNDED_POLICY)
        for field in ('global_minimum_excavation_proven', 'terrain_observed',
                      'native_pathfinding_proven', 'mutation_authority_granted'):
            self.assertIs(out[field], False)


if __name__ == '__main__':
    unittest.main()
