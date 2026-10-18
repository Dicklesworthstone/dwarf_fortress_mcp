"""Direct minimum-corridor tests with independent exhaustive cell-set oracles."""
from __future__ import annotations

import itertools
from pathlib import Path
import random
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import room_connection_solver as solver


def connected(cells):
    seen, pending = {min(cells)}, [min(cells)]
    while pending:
        current = pending.pop()
        for candidate in cells - seen:
            if sum(abs(a - b) for a, b in zip(current, candidate)) == 1:
                seen.add(candidate)
                pending.append(candidate)
    return seen == cells


def optimum(floors, admitted):
    choices = sorted(admitted - floors)
    for count in range(len(choices) + 1):
        for selected in itertools.combinations(choices, count):
            if connected(floors | set(selected)):
                return count
    return None


def solve(floors, admitted, allowance, root=None, work=lambda: None):
    return solver.minimum_tiles(floors, admitted, min(floors) if root is None else root,
                                allowance, work)


class SolverCoreTests(unittest.TestCase):
    def check(self, floors, admitted):
        expected = optimum(floors, admitted)
        original = floors.copy(), admitted.copy()
        if expected is None:
            with self.assertRaises(solver.NoConnection):
                solve(floors, admitted, len(admitted - floors))
        else:
            actual = solve(floors, admitted, expected)
            selected = set(actual.added)
            self.assertEqual(actual.cost, expected)
            self.assertEqual(len(selected), expected)
            self.assertFalse(selected & floors)
            self.assertTrue(selected <= admitted)
            self.assertTrue(connected(floors | selected))
            if expected:
                with self.assertRaises(solver.NoConnection):
                    solve(floors, admitted, expected - 1)
        self.assertEqual((floors, admitted), original)

    def test_all_6561_worlds_match_exhaustive_oracle(self):
        cells = set(itertools.product(range(1, 4), range(1, 4), (1,)))
        root = (1, 1, 1)
        variables = sorted(cells - {root})
        for state in itertools.product(range(3), repeat=len(variables)):
            floors = {root} | {p for p, s in zip(variables, state) if s == 1}
            admitted = cells - {p for p, s in zip(variables, state) if s == 2}
            self.check(floors, admitted)

    def test_400_larger_masks_match_exhaustive_oracle(self):
        # A fixed local generator; neither runtime randomness nor wall time is
        # used by the production solver or the oracle's result ordering.
        rng = random.Random(73041)
        points = list(itertools.product(range(1, 5), range(1, 5), (1,)))
        for _ in range(400):
            states = [rng.randrange(3) for _ in points]
            states[0] = 1
            floors = {p for p, s in zip(points, states) if s == 1}
            admitted = {p for p, s in zip(points, states) if s != 2}
            self.check(floors, admitted)

    def test_free_floor_components_and_cycles_do_not_duplicate_cost(self):
        admitted = set(itertools.product(range(1, 5), range(1, 5), (1,)))
        for floors in (
            {(1, 1, 1), (1, 2, 1), (2, 1, 1), (2, 2, 1), (4, 4, 1)},
            {(x, y, 1) for x in (1, 4) for y in range(1, 5)},
            admitted,
        ):
            self.check(floors, admitted)

    def test_root_must_be_an_exact_coordinate_not_a_numeric_alias(self):
        floors = {(1, 1, 1), (3, 1, 1)}
        admitted = floors | {(2, 1, 1)}
        for root in ((True, 1, 1), (1.0, 1, 1), (1, 1, True), None, (), [1, 1, 1]):
            with self.subTest(root=root), self.assertRaises(ValueError):
                solver.minimum_tiles(floors, admitted, root, 1, lambda: None)

    def test_each_original_root_has_same_optimal_cost(self):
        floors = {(1, 1, 1), (2, 2, 1), (3, 1, 1)}
        admitted = set(itertools.product(range(1, 4), range(1, 4), (1,)))
        for root in floors:
            result = solve(floors, admitted, 1, root=root)
            self.assertEqual(result.added, ((2, 1, 1),))

    def test_caller_cancellation_propagates_at_every_checkpoint(self):
        floors = {(1, 1, 1), (2, 2, 1), (3, 1, 1)}
        admitted = set(itertools.product(range(1, 4), range(1, 4), (1,)))
        count = []
        solve(floors, admitted, 1, work=lambda: count.append(1))
        for stop in range(1, len(count) + 1):
            calls = 0
            def work():
                nonlocal calls
                calls += 1
                if calls == stop:
                    raise InterruptedError('cancelled exact corridor computation')
            with self.assertRaisesRegex(InterruptedError, 'cancelled exact'):
                solve(floors, admitted, 1, work=work)
            self.assertEqual(calls, stop)

    def test_nine_components_fit_structural_tables(self):
        floors = {(x, 1, 1) for x in range(1, 18, 2)}
        admitted = {(x, 1, 1) for x in range(1, 18)}
        result = solve(floors, admitted, 8)
        self.assertEqual(result.cost, 8)
        self.assertEqual(result.subset_count, 511)
        self.assertLessEqual(result.table_bytes, solver.MAX_TABLE_BYTES)
        with self.assertRaises(solver.ComponentLimit):
            solve(floors | {(19, 1, 1)}, admitted | {(18, 1, 1), (19, 1, 1)}, 9)

    def test_invalid_bounds_are_refused_before_subset_allocation(self):
        for floors, admitted, allowance in ((set(), set(), 0),
                ({(1, 1, 1)}, set(), 0), ({(1, 1, 1)}, {(1, 1, 1)}, True),
                ({(1, 1, 1)}, {(1, 1, 1)}, 512),
                ({(1, 1, 1)}, {(1, 1, 1), (1, 1, 2)}, 1)):
            with self.assertRaises(ValueError):
                solver.minimum_tiles(floors, admitted, (1, 1, 1), allowance, lambda: None)


if __name__ == '__main__':
    unittest.main()
