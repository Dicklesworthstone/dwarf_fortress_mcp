"""Real connector composition; independent complete-path oracles and hard limits."""
from copy import deepcopy
import itertools
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import room_connections as legacy
import room_cost_connections as routing


def grid(width=5, height=3, charged=(), origin=(1, 1, 1)):
    cells = ''.join('1' if (x, y, origin[2]) in charged else '0'
                    for y in range(origin[1], origin[1] + height)
                    for x in range(origin[0], origin[0] + width))
    return routing.Costs.parse({'region': {'origin': list(origin), 'size': [width, height, 1]}, 'cells': cells})


def path_oracle(floors, blocked, charged, width, height):
    start, goal = sorted(floors)
    permitted = {(x, y, 1) for x in range(1, width + 1) for y in range(1, height + 1)} - blocked
    best = None
    stack = [(start, {start})]
    while stack:
        point, seen = stack.pop()
        if point == goal:
            additions = seen - floors
            score = len(additions & charged), len(additions)
            best = score if best is None else min(best, score)
            continue
        if best is not None and (len((seen - floors) & charged), len(seen - floors)) >= best:
            continue
        for following in permitted - seen:
            if sum(abs(a - b) for a, b in zip(point, following)) == 1:
                stack.append((following, seen | {following}))
    return best


class CostConnectionsTests(unittest.TestCase):
    def run_route(self, *, floors=None, blocked=(), allowance=7, parts=32, costs=None, **options):
        floors = {(1, 2, 1), (5, 2, 1)} if floors is None else floors
        costs = grid(charged={(2, 2, 1), (3, 2, 1), (4, 2, 1)}) if costs is None else costs
        entries = tuple((f'area{i}', p) for i, p in enumerate(sorted(floors)[:8]))
        return routing.connect(set(floors), set(blocked), entries, costs,
                               max_new_tiles=allowance, max_parts=parts, **options)

    def assert_geometry(self, result, floors, blocked=()):
        parts = []
        for (x, y, z), (w, h, levels) in result.parts:
            parts.extend(itertools.product(range(x, x+w), range(y, y+h), range(z, z+levels)))
        self.assertEqual(set(parts), set(result.added))
        self.assertEqual(len(parts), len(set(parts)))
        self.assertFalse(set(parts) & set(floors))
        self.assertFalse(set(parts) & set(blocked))
        selected = set(parts) | set(floors)
        self.assertEqual(legacy._flood(selected, min(floors), legacy._Work(legacy.idle, legacy.MAX_WORK)), selected)

    def test_open_passage_replaces_three_charged_cells_without_changing_endpoints(self):
        result = self.run_route()
        self.assertEqual((result.baseline_cost, result.selected_cost), (3, 0))
        self.assertEqual(len(result.added), 5)
        self.assertEqual(result.selected_strategy, 'cost_first')
        self.assert_geometry(result, {(1, 2, 1), (5, 2, 1)})
        self.assertTrue(result.summary()['corridors_rerouted_by_cost'])

    def test_long_free_detour_cannot_exceed_original_tile_allowance(self):
        result = self.run_route(allowance=3)
        self.assertEqual(result.selected_strategy, 'original')
        self.assertEqual(result.weighted_status, 'tile_limit')
        self.assertEqual(result.selected_cost, 3)
        self.assertEqual(set(result.added), {(2, 2, 1), (3, 2, 1), (4, 2, 1)})

    def test_protected_free_tiles_never_become_available(self):
        blocked = {(3, 1, 1), (3, 3, 1)}
        result = self.run_route(blocked=blocked)
        self.assertEqual(result.selected_cost, 3)
        self.assert_geometry(result, {(1, 2, 1), (5, 2, 1)}, blocked)

    def test_all_components_and_entries_are_retained(self):
        floors = {(1, 1, 1), (5, 1, 1), (1, 5, 1), (5, 5, 1)}
        charged = {(x, y, 1) for x in range(2, 5) for y in range(1, 6)}
        result = self.run_route(floors=floors, costs=grid(5, 5, charged), allowance=21)
        self.assert_geometry(result, floors)
        self.assertLessEqual(result.selected_cost, result.baseline_cost)
        self.assertTrue(result.summary()['all_areas_connected'])

    def test_existing_connected_project_needs_zero_corridor_parts(self):
        floors = {(x, 2, 1) for x in range(1, 6)}
        result = self.run_route(floors=floors, allowance=0, parts=0)
        self.assertEqual(result.parts, ())
        self.assertEqual(result.added, ())
        self.assertEqual(result.weighted_status, 'not_needed_zero_cost')

    def test_charges_only_added_cells_not_existing_required_floors(self):
        floors = {(1, 2, 1), (5, 2, 1)}
        result = self.run_route(costs=grid(charged=floors))
        self.assertEqual(result.selected_cost, 0)
        self.assertEqual(result.selected_strategy, 'original')

    def test_remaining_part_limit_never_silently_widens(self):
        for bound in (0, 1, 2):
            try:
                result = self.run_route(parts=bound)
            except ValueError as error:
                self.assertEqual(bound, 0)
                self.assertIn('part allowance', str(error))
            else:
                self.assertLessEqual(len(result.parts), bound)

    def test_cheaper_detour_must_fit_remaining_part_allowance(self):
        result = self.run_route(floors={(1, 1, 1), (1, 3, 1)}, allowance=14, parts=1,
                                costs=grid(4, 4, {(1, 2, 1), (2, 2, 1)}))
        self.assertEqual(result.weighted_status, 'part_limit')
        self.assertEqual(result.selected_strategy, 'original')
        self.assertEqual(result.selected_cost, 1)
        self.assertEqual(len(result.parts), 1)

    def test_all_2187_small_cost_and_barrier_worlds_match_path_oracle(self):
        floors = {(1, 1, 1), (3, 3, 1)}
        cells = sorted({(x, y, 1) for x in range(1, 4) for y in range(1, 4)} - floors)
        for states in itertools.product(range(3), repeat=7):
            blocked = {p for p, s in zip(cells, states) if s == 2}
            charged = {p for p, s in zip(cells, states) if s == 1}
            expected = path_oracle(floors, blocked, charged, 3, 3)
            with self.subTest(states=states):
                if expected is None:
                    with self.assertRaises(ValueError):
                        self.run_route(floors=floors, blocked=blocked, costs=grid(3, 3, charged))
                else:
                    result = self.run_route(floors=floors, blocked=blocked, costs=grid(3, 3, charged))
                    self.assertEqual((result.selected_cost, len(result.added)), expected)
                    self.assert_geometry(result, floors, blocked)

    def test_every_guard_checkpoint_propagates_cancellation_even_after_baseline(self):
        result = self.run_route()
        for at in range(1, result.work_units + 1):
            calls = 0
            def guard():
                nonlocal calls
                calls += 1
                if calls == at:
                    raise ValueError('caller cancelled')
            with self.subTest(at=at), self.assertRaisesRegex(ValueError, 'caller cancelled'):
                self.run_route(guard=guard)
            self.assertEqual(calls, at)

    def test_work_budget_spans_baseline_and_alternative_and_publication(self):
        result = self.run_route()
        self.assertEqual(self.run_route(max_work=result.work_units), result)
        for limit in (1, result.work_units // 2, result.work_units - 1):
            with self.assertRaises(legacy.WorkLimit):
                self.run_route(max_work=limit)

    def test_invalid_cost_regions_types_and_nonbinary_cells_refuse(self):
        template = grid().json()
        cases = []
        for cells in ('', '0' * 14, '0' * 16, '2' * 15, 'F' * 15, [0] * 15, True):
            cases.append({**template, 'cells': cells})
        for key, value in (('origin', [True, 1, 1]), ('origin', [1.0, 1, 1]),
                           ('origin', [0, 1, 1]), ('size', [5, 3, 2]), ('size', [129, 1, 1]),
                           ('size', [33, 32, 1]), ('size', [5, False, 1])):
            cases.append({**template, 'region': {**template['region'], key: value}})
        cases.extend(({**template, 'unknown': 1}, {'cells': '0'}, {'region': []}))
        for value in cases:
            with self.subTest(value=value), self.assertRaises(ValueError):
                routing.Costs.parse(value)
        for bound in (-1, 33, True, 1.0):
            with self.assertRaises(ValueError): self.run_route(parts=bound)

    def test_exact_root_and_cost_domain_are_checked_before_routing(self):
        for root in ((True, 2, 1), (1.0, 2, 1), (2, 2, 1)):
            with self.assertRaises(ValueError): self.run_route(root=root)
        with self.assertRaises(ValueError): self.run_route(costs=grid(4, 3))
        with self.assertRaises(ValueError): self.run_route(blocked={(1, 2, 1)})

    def test_cost_lookup_rejects_numeric_coordinate_aliases(self):
        costs = grid()
        for point in ((True, 1, 1), (1.0, 1, 1), (1, 1), [1, 1, 1], (0, 1, 1)):
            with self.assertRaises(ValueError): costs.at(point)

    def test_directly_constructed_costs_are_not_trusted(self):
        costs = routing.Costs(((1, 1, 1), (5, 3, 1)), '0')
        with self.assertRaises(ValueError): self.run_route(costs=costs)

    def test_input_order_and_zero_cost_ties_are_deterministic(self):
        floors, blocked = {(1, 2, 1), (5, 2, 1)}, {(2, 3, 1)}
        original = deepcopy((floors, blocked))
        costs = grid(charged={(3, 2, 1)})
        a = routing.connect(floors, blocked, (('a', (1,2,1)), ('b', (5,2,1))), costs, max_new_tiles=7)
        b = routing.connect(frozenset(reversed(sorted(floors))), frozenset(blocked),
                            (('b', (5,2,1)), ('a', (1,2,1))), costs, max_new_tiles=7)
        self.assertEqual(a, b)
        self.assertEqual((floors, blocked), original)

    def test_witness_binds_original_costs_and_hard_limits(self):
        a = self.run_route()
        b = self.run_route(costs=grid(charged={(3, 2, 1)}))
        c = self.run_route(allowance=6)
        d = self.run_route(parts=20)
        self.assertEqual(len({x.digest for x in (a, b, c, d)}), 4)
        self.assertEqual(routing.Costs.parse(grid().json()), grid())

    def test_no_terrain_effect_or_global_cost_optimality_claim(self):
        value = self.run_route().summary()
        for field in ('terrain_observed', 'global_minimum_excavation_proven',
                      'native_pathfinding_proven', 'mutation_authority_granted'):
            self.assertIs(value[field], False)


if __name__ == '__main__':
    unittest.main()
