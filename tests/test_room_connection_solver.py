"""Exhaustive independent masks and whole-router regressions; no native claims."""
from __future__ import annotations

from copy import deepcopy
import hashlib
import itertools
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest

SCRIPTS = Path(__file__).resolve().parents[1] / 'scripts'
sys.path.insert(0, str(SCRIPTS))
import room_connections as connections
import room_connection_solver as exact

REGION = ((1, 1, 1), (3, 3, 1))
JUNCTION = {(1, 1, 1), (2, 2, 1), (3, 1, 1)}


def encode(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def reached(mask, root):
    # Separate cell oracle: no production neighbors, BFS, partition or DP.
    visited, pending = {root}, [root]
    while pending:
        p = pending.pop()
        for q in mask - visited:
            if sum(abs(a - b) for a, b in zip(p, q)) == 1:
                visited.add(q)
                pending.append(q)
    return visited


def oracle(floors, admitted):
    candidates = sorted(admitted - floors)
    for count in range(len(candidates) + 1):
        for chosen in itertools.combinations(candidates, count):
            selected = floors | set(chosen)
            if reached(selected, min(floors)) == selected:
                return count
    return None


def worlds():
    cells = set(itertools.product(range(1, 4), range(1, 4), (1,)))
    variable = sorted(cells - {(1, 1, 1)})
    for states in itertools.product(range(3), repeat=len(variable)):
        floor = {(1, 1, 1)} | {p for p, s in zip(variable, states) if s == 1}
        blocked = {p for p, s in zip(variable, states) if s == 2}
        yield floor, blocked, cells - blocked


def route(floors=JUNCTION, blocked=frozenset(), allowance=1, **kwargs):
    return connections.connect(set(floors), set(blocked), (('room', min(floors)),),
                               REGION, max_new_tiles=allowance, **kwargs)


def partition_cells(parts):
    points = []
    for (x, y, z), (w, h, levels) in parts:
        points.extend(itertools.product(range(x, x + w), range(y, y + h), range(z, z + levels)))
    return points


def compatibility_digest(engine):
    digest, successful, rejected = hashlib.sha256(), 0, 0
    for floors, blocked, admitted in worlds():
        args = (floors, blocked, (('room', min(floors)),), REGION)
        try:
            output = engine.connect(*args, max_new_tiles=len(admitted - floors))
            row = [output.added, output.parts, output.summary()]
            successful += 1
        except ValueError as error:
            row = ['refused', str(error)]
            rejected += 1
        digest.update(encode(row) + b'\n')
    return digest.hexdigest(), successful, rejected


class MinimumConnectionTests(unittest.TestCase):
    def check_complete(self, result, floors=JUNCTION, blocked=frozenset(), allowance=1):
        added = set(result.added)
        self.assertLessEqual(len(added), allowance)
        self.assertFalse(added & (floors | set(blocked)))
        final = set(floors) | added
        self.assertEqual(reached(final, min(floors)), final)
        self.assertEqual(result.walkable_tiles, len(final))
        self.assertEqual(set(partition_cells(result.parts)), added)
        self.assertEqual(len(partition_cells(result.parts)), len(added))
        self.assertEqual(sum(link.new_tiles for link in result.links), len(added))

    def test_shared_junction_rescues_previously_refused_project(self):
        out = route()
        self.check_complete(out)
        self.assertEqual(out.added, ((2, 1, 1),))
        self.assertTrue(out.minimum_excavation)
        self.assertEqual((out.component_count, out.subset_count), (3, 7))
        self.assertEqual(out.digest, 'ec5bda711cea98e07728b74bdf6c22994066d9b249872a7d757f5ce2d2d1ca4b')
        self.assertEqual(out.summary()['policy'], connections.MINIMUM_POLICY)

    def test_feasible_greedy_result_and_its_original_policy_are_unchanged(self):
        out = route(allowance=2)
        self.assertEqual(out.added, ((1, 2, 1), (2, 1, 1)))
        self.assertFalse(out.minimum_excavation)
        self.assertEqual(out.summary()['policy'], connections.POLICY)
        self.assertNotIn('minimum_excavation_in_region_proven', out.summary())

    def test_all_6561_three_state_worlds_match_independent_minimum_oracle(self):
        checked, feasible, rescued = 0, 0, 0
        for floors, blocked, admitted in worlds():
            wanted = oracle(floors, admitted)
            with self.subTest(floors=sorted(floors), blocked=sorted(blocked)):
                if wanted is None:
                    with self.assertRaises(ValueError) as error:
                        route(floors, blocked, len(admitted - floors))
                    self.assertNotIsInstance(error.exception, connections.WorkLimit)
                else:
                    result = route(floors, blocked, wanted)
                    self.check_complete(result, floors, blocked, wanted)
                    self.assertEqual(len(result.added), wanted)
                    feasible += 1
                    rescued += int(result.minimum_excavation)
                    if wanted:
                        with self.assertRaises(ValueError) as error:
                            route(floors, blocked, wanted - 1)
                        self.assertNotIsInstance(error.exception, connections.WorkLimit)
                checked += 1
        self.assertEqual(checked, 6561)
        self.assertGreater(feasible, 1000)
        self.assertGreater(rescued, 0)

    def test_legacy_whole_capture_outputs_keep_the_baseline_digest(self):
        self.assertEqual(compatibility_digest(connections),
                         ('b50a271200367e5feaea3953e8127fbc66ab313e69e1355bb731b57a3694bad7', 4832, 1729))

    def test_every_cancellation_checkpoint_propagates_without_partial_return(self):
        checkpoints = []
        out = route(guard=lambda: checkpoints.append(1))
        self.assertEqual(len(checkpoints), out.work_units)
        original = deepcopy(JUNCTION)
        for stop in range(1, len(checkpoints) + 1):
            seen = 0
            def guard():
                nonlocal seen
                seen += 1
                if seen == stop:
                    # ValueError must not be swallowed as infeasible routing.
                    raise ValueError('caller revoked routing')
            with self.subTest(stop=stop), self.assertRaisesRegex(ValueError, 'caller revoked routing'):
                route(guard=guard)
            self.assertEqual(seen, stop)
        self.assertEqual(JUNCTION, original)

    def test_one_shrinking_work_budget_includes_failed_greedy_attempt(self):
        result = route()
        self.assertEqual(route(max_work=result.work_units), result)
        for limit in (1, result.work_units // 2, result.work_units - 1):
            with self.subTest(limit=limit), self.assertRaises(connections.WorkLimit):
                route(max_work=limit)

    def test_protected_junction_is_not_ignored_or_converted_to_floor(self):
        blocked = {(2, 1, 1)}
        before = deepcopy((JUNCTION, blocked))
        with self.assertRaises(exact.NoConnection):
            route(blocked=blocked)
        out = route(blocked=blocked, allowance=2)
        self.check_complete(out, JUNCTION, blocked, 2)
        self.assertEqual((JUNCTION, blocked), before)

    def test_all_original_components_are_required_not_only_named_rooms(self):
        out = connections.connect(JUNCTION, set(), (('only_named_room', (1, 1, 1)),),
                                  REGION, max_new_tiles=1)
        self.check_complete(out)
        self.assertEqual(out.component_count, 3)

    def test_explicit_entrance_is_not_replaced_with_lexical_first_room(self):
        # Equivalent translated fixture with a separately named room and root.
        floors = {(2, 2, 1), (1, 3, 1), (3, 3, 1)}
        out = connections.connect(floors, set(), (('room', (3, 3, 1)),), REGION,
                                  max_new_tiles=1, root=(1, 3, 1))
        self.check_complete(out, floors, allowance=1)
        self.assertEqual(out.added, ((2, 3, 1),))
        self.assertTrue(out.minimum_excavation)

    def test_maximum_capture_does_not_escape_exclusions_to_find_a_hull(self):
        region = ((1, 1, 1), (32, 32, 1))
        all_cells = set(itertools.product(range(1, 33), range(1, 33), (1,)))
        small = set(itertools.product(range(1, 4), range(1, 4), (1,)))
        blocked = all_cells - small
        out = connections.connect(JUNCTION, blocked, (('room', (1, 1, 1)),), region,
                                  max_new_tiles=1)
        self.check_complete(out, blocked=blocked)
        self.assertEqual(out.added, ((2, 1, 1),))
        self.assertLess(out.work_units, connections.MAX_WORK)

    def test_component_limit_is_inconclusive_not_a_no_layout_claim(self):
        floors = {(x, 1, 1) for x in range(1, 20, 2)}
        with self.assertRaises(connections.WorkLimit):
            connections.connect(floors, set(), (('room', (1, 1, 1)),),
                                ((1, 1, 1), (19, 1, 1)), max_new_tiles=0)

    def test_internal_solver_supports_eight_areas_plus_external_entry(self):
        floor = {(x, 1, 1) for x in range(1, 18, 2)}
        admitted = {(x, 1, 1) for x in range(1, 18)}
        calls = []
        result = exact.minimum_tiles(floor, admitted, (1, 1, 1), 8, lambda: calls.append(1))
        self.assertEqual(len(result.representatives), 9)
        self.assertEqual(result.subset_count, 511)
        self.assertEqual(result.cost, 8)
        self.assertEqual(set(result.added), admitted - floor)
        self.assertLessEqual(result.table_bytes, exact.MAX_TABLE_BYTES)
        self.assertLess(len(calls), 1_000_000)

    def test_same_input_across_hash_seeds_has_identical_result(self):
        code = ('import json,room_connections as c;'
                'r=c.connect({(1,1,1),(2,2,1),(3,1,1)},set(),'
                '(("room",(1,1,1)),),((1,1,1),(3,3,1)),max_new_tiles=1);'
                'print(json.dumps([r.added,r.parts,r.summary()],sort_keys=True,separators=(",",":")))')
        expected = encode([route().added, route().parts, route().summary()]) + b'\n'
        for seed in ('1', '27', 'random'):
            output = subprocess.run([sys.executable, '-W', 'error', '-c', code],
                env={**os.environ, 'PYTHONPATH': str(SCRIPTS), 'PYTHONHASHSEED': seed},
                check=True, capture_output=True, timeout=15)
            self.assertEqual(output.stdout, expected)
            self.assertEqual(output.stderr, b'')

    def test_minimum_proof_does_not_grant_terrain_native_or_global_authority(self):
        out = route().summary()
        self.assertTrue(out['minimum_excavation_in_region_proven'])
        for key in ('global_minimum_excavation_proven', 'native_pathfinding_proven',
                    'terrain_observed', 'rectangle_count_optimal'):
            self.assertIs(out[key], False)

    def test_internal_structural_bounds_precede_exponential_allocation(self):
        admitted = set(itertools.product(range(1, 4), range(1, 4), (1,)))
        for floor, cells, root, limit in ((set(), admitted, (1, 1, 1), 1),
                (JUNCTION, admitted, (2, 1, 1), 1), (JUNCTION, admitted, (1, 1, 1), True),
                (JUNCTION, admitted, (1, 1, 1), 510),
                (JUNCTION, admitted | {(1, 1, 2)}, (1, 1, 1), 1)):
            with self.assertRaises(ValueError):
                exact.minimum_tiles(floor, cells, root, limit, lambda: None)


if __name__ == '__main__':
    unittest.main()
