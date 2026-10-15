"""Independent geometry/path oracles for the actual room connection engine."""
import unittest

import room_connections as routing


def cells(parts):
    result = []
    for (x, y, z), (w, h, levels) in parts:
        result.extend((xx, yy, zz) for zz in range(z, z + levels)
                      for yy in range(y, y + h) for xx in range(x, x + w))
    return result


def reachable(mask, root):
    pending, seen = [root], {root}
    while pending:
        x, y, z = pending.pop()
        for p in ((x + 1, y, z), (x - 1, y, z), (x, y + 1, z), (x, y - 1, z)):
            if p in mask and p not in seen:
                seen.add(p)
                pending.append(p)
    return seen


class ConnectionsTests(unittest.TestCase):
    def run_route(self, floors, blocked=(), size=(5, 5, 1), **kw):
        entries = tuple((f'area{i}', p) for i, p in enumerate(sorted(floors)[:8]))
        return routing.connect(set(floors), set(blocked), entries, ((1, 1, 1), size),
                               max_new_tiles=kw.pop('max_new_tiles', 512 - len(floors)), **kw)

    def test_connects_every_component_not_only_named_endpoints(self):
        floors = {(1, 1, 1), (5, 1, 1), (5, 5, 1), (1, 5, 1)}
        out = self.run_route(floors)
        self.assertEqual(reachable(floors | set(out.added), (1, 1, 1)), floors | set(out.added))
        self.assertEqual(len(out.links), 3)
        self.assertEqual(out.walkable_tiles, len(floors) + len(out.added))
        self.assertFalse(set(out.added) & floors)

    def test_protected_strip_is_detoured_without_mining_it(self):
        floors = {(1, 3, 1), (5, 3, 1)}
        blocked = {(3, y, 1) for y in range(1, 5)}
        out = self.run_route(floors, blocked)
        self.assertFalse(set(out.added) & blocked)
        self.assertIn((3, 5, 1), out.added)
        self.assertEqual(out.links[0].steps, 8)

    def test_unreachable_and_tile_budget_refuse_without_mutating_inputs(self):
        floors = {(1, 3, 1), (5, 3, 1)}
        blocked = {(3, y, 1) for y in range(1, 6)}
        before = floors.copy(), blocked.copy()
        with self.assertRaises(ValueError):
            self.run_route(floors, blocked)
        with self.assertRaises(ValueError):
            self.run_route(floors, max_new_tiles=2)
        self.assertEqual((floors, blocked), before)

    def test_no_new_tiles_when_complete_floor_is_connected(self):
        floors = {(x, 1, 1) for x in range(1, 6)}
        out = self.run_route(floors, max_new_tiles=0)
        self.assertEqual(out.added, ())
        self.assertEqual(out.parts, ())
        self.assertEqual(out.links, ())

    def test_canonical_inputs_and_equal_cost_paths(self):
        floors = {(1, 1, 1), (5, 5, 1)}
        entries = (('sleep', (1, 1, 1)), ('eat', (5, 5, 1)))
        args = dict(region=((1, 1, 1), (5, 5, 1)), max_new_tiles=510)
        first = routing.connect(floors, set(), entries, **args)
        second = routing.connect(frozenset(reversed(sorted(floors))), frozenset(),
                                 tuple(reversed(entries)), **args)
        self.assertEqual(first, second)
        self.assertEqual(first.links[0].steps, 8)
        self.assertEqual(len(first.added), 7)
        self.assertFalse(first.summary()['native_pathfinding_proven'])

    def test_every_3_by_3_barrier_mask_matches_exhaustive_simple_paths(self):
        root, goal = (1, 1, 1), (3, 3, 1)
        points = {(x, y, 1) for x in range(1, 4) for y in range(1, 4)}
        variable = sorted(points - {root, goal})
        for bits in range(1 << len(variable)):
            blocked = {p for i, p in enumerate(variable) if bits & (1 << i)}
            free = points - blocked
            best = None
            stack = [(root, frozenset({root}), 0)]
            while stack:
                p, seen, steps = stack.pop()
                if p == goal:
                    best = steps if best is None else min(best, steps)
                    continue
                if best is not None and steps >= best:
                    continue
                for q in free - seen:
                    if sum(abs(a - b) for a, b in zip(p, q)) == 1:
                        stack.append((q, seen | {q}, steps + 1))
            with self.subTest(mask=bits):
                if best is None:
                    with self.assertRaises(ValueError):
                        self.run_route({root, goal}, blocked, (3, 3, 1))
                else:
                    out = self.run_route({root, goal}, blocked, (3, 3, 1))
                    self.assertEqual(out.links[0].steps, best)
                    self.assertEqual(len(out.added), best - 1)
                    self.assertEqual(set(cells(out.parts)), set(out.added))

    def test_partitions_every_3_by_3_mask_exactly_without_overlap(self):
        points = [(x, y, 1) for y in range(1, 4) for x in range(1, 4)]
        for bits in range(512):
            mask = {p for i, p in enumerate(points) if bits & (1 << i)}
            parts = routing._partition(mask, routing._Work(routing.idle, routing.MAX_WORK))
            actual = cells(parts)
            self.assertEqual(set(actual), mask)
            self.assertEqual(len(actual), len(mask))

    def test_guard_interruption_at_every_boundary_is_atomic(self):
        floors, blocked = {(1, 1, 1), (5, 5, 1)}, {(3, 3, 1)}
        calls = 0
        def count():
            nonlocal calls
            calls += 1
        expected = self.run_route(floors, blocked, guard=count)
        self.assertEqual(calls, expected.work_units)
        for fail_at in range(1, calls + 1):
            remaining = fail_at
            def stop():
                nonlocal remaining
                remaining -= 1
                if remaining == 0:
                    raise InterruptedError('cancelled')
            with self.assertRaises(InterruptedError):
                self.run_route(floors, blocked, guard=stop)
        self.assertEqual(floors, {(1, 1, 1), (5, 5, 1)})
        self.assertEqual(blocked, {(3, 3, 1)})

    def test_exact_shrinking_work_allowance(self):
        floors = {(1, 1, 1), (5, 5, 1)}
        expected = self.run_route(floors)
        self.assertEqual(self.run_route(floors, max_work=expected.work_units), expected)
        with self.assertRaises(ValueError):
            self.run_route(floors, max_work=expected.work_units - 1)

    def test_closed_coordinate_entry_and_capacity_bounds(self):
        cases = [dict(floors=set()), dict(blocked={(1, 1, 1)}),
                 dict(entries=(('same', (1, 1, 1)), ('same', (3, 3, 1)))),
                 dict(entries=(('bad\nname', (1, 1, 1)),)),
                 dict(entries=(('a', (True, 1, 1)),)),
                 dict(region=((1, 1, 1), (3, 3, 2))),
                 dict(region=((1, 1, 1), (128, 128, 1))),
                 dict(region=((32766, 1, 1), (3, 3, 1))),
                 dict(max_new_tiles=True), dict(max_new_tiles=511), dict(max_work=0)]
        for replacement in cases:
            args = dict(floors={(1, 1, 1), (3, 3, 1)}, blocked=set(),
                        entries=(('a', (1, 1, 1)), ('b', (3, 3, 1))),
                        region=((1, 1, 1), (3, 3, 1)), max_new_tiles=510)
            args.update(replacement)
            with self.subTest(replacement=replacement), self.assertRaises(ValueError):
                routing.connect(**args)

    def test_maximum_capture_keeps_one_complete_connected_result(self):
        floors = {(1, 1, 1), (32, 1, 1), (32, 32, 1), (1, 32, 1)}
        out = self.run_route(floors, size=(32, 32, 1))
        self.assertLessEqual(out.work_units, routing.MAX_WORK)
        self.assertEqual(reachable(floors | set(out.added), (1, 1, 1)), floors | set(out.added))
        self.assertEqual(set(cells(out.parts)), set(out.added))
        self.assertEqual(len(cells(out.parts)), len(out.added))


if __name__ == '__main__':
    unittest.main()
