"""Actual fixed-order solver against exhaustive assignment / coordinate BFS.

Native-looking maps are independently encoded test data, not game observations.
"""
from collections import deque
from dataclasses import replace
import hashlib
import itertools
import json
import os
from pathlib import Path
import random
import struct
import subprocess
import sys
import unittest
from unittest.mock import patch

from furniture_allocation import Candidate, Request, Slot
from furniture_plan import FurniturePlan, canonical
import route_furniture_allocation as routes
import route_furniture_fixed_order as fixed
import route_furniture_sequence as sequence


def raw_map(size=(3, 3, 1), *, hidden=(), origin=(1, 1, 0), shapes=None):
    n = size[0] * size[1] * size[2]
    out = b'DFMM1500' + struct.pack('>IIBIH', 0, 3, 1, 7, 4) + b'fort'
    out += struct.pack('>10I', 256, 256, 128, *origin, *size, n)
    for i in range(n):
        out += b'\x01' if i in hidden else b'\x02' + struct.pack(
            '>IBBBBBBBIHH', 1, 3 if shapes is None else shapes[i], 0, 0, 0, 0, 0, 0, 1, 10015, 10015)
    return out


def item(identity, point, material=(1, -1), kind='bed', subtype=-1):
    return Candidate(identity, kind, tuple(point), material, subtype)


def solve(request, items, raw, **kwargs):
    returned, result = fixed.allocate_fixed_order_routes(request, items, raw, **kwargs)
    assert returned is request
    return result


def distance(start, goal, cells, blocked, limit):
    """Independent 2D coordinate implementation, never production adjacency."""
    if start not in cells or goal not in cells or start in blocked or goal in blocked:
        return None
    queue, seen = deque([(start, 0)]), {start}
    while queue:
        (x, y, z), n = queue.popleft()
        if (x, y, z) == goal:
            return n
        if n < limit:
            for point in ((x-1, y, z), (x+1, y, z), (x, y-1, z), (x, y+1, z)):
                if point in cells and point not in blocked and point not in seen:
                    seen.add(point)
                    queue.append((point, n+1))
    return None


def oracle(request, items, cells, limit=routes.MAX_ROUTE_STEPS):
    # Independently reconstruct lexical topological order.
    remaining, order = {s.name: s for s in request.slots}, []
    while remaining:
        name = min(n for n, s in remaining.items() if set(s.after) <= set(order))
        order.append(name)
        del remaining[name]
    costs, blocked = {}, set()
    slots = {s.name: s for s in request.slots}
    for name in order:
        s = slots[name]
        row = {}
        for c in items:
            geometric = sum(abs(a-b) for a, b in zip(s.target, c.position))
            if (c.id in request.excluded_items or s.kind != c.kind
                    or (s.material is not None and s.material != c.material)
                    or (s.subtype is not None and s.subtype != c.subtype)
                    or abs(s.target[2]-c.position[2]) > s.max_vertical_distance
                    or geometric > s.max_distance):
                continue
            d = distance(c.position, s.target, cells, blocked, limit)
            if d is not None:
                row[c.id] = d
        costs[name] = row
        blocked.add(s.target)
    best = None
    for perm in itertools.permutations([c.id for c in items], len(request.slots)):
        if all(i in costs[s.name] for s, i in zip(request.slots, perm)):
            value = sum(costs[s.name][i] for s, i in zip(request.slots, perm)), perm
            best = value if best is None else min(best, value)
    return best, costs


class FixedOrderTests(unittest.TestCase):
    def example(self):
        r = Request('fort', 7, (Slot('a', 'bed', (2, 2, 0)), Slot('b', 'bed', (3, 2, 0))))
        cs = (item(1, (2, 2, 0)), item(2, (2, 2, 0)), item(3, (5, 2, 0)))
        return r, cs, raw_map((5, 3, 1))

    def test_avoid_stranding_later_item_without_rewriting_original_dependencies(self):
        r, cs, raw = self.example()
        before = canonical(r.json())
        naive = routes.allocate_routes(r, cs, raw)
        self.assertEqual([row['item'] for row in naive['assignments']], [1, 2])
        out = solve(r, cs, raw)
        self.assertEqual([row['item'] for row in out['assignments']], [1, 3])
        self.assertEqual(out['fixed_order_delivery']['complete_delivery_cost'], 2)
        self.assertEqual(canonical(r.json()), before)
        self.assertEqual(out['request_digest'], r.digest)
        self.assertTrue(out['fixed_order_delivery']['assignment_optimality_proven'])
        self.assertEqual([s.after for s in FurniturePlan.from_json(out['plan']).steps], [(), ()])
        self.assertFalse(out['fixed_order_delivery']['alternative_orders_ruled_out'])

    def test_complete_shortage_is_for_the_fixed_order_not_all_possible_orders(self):
        r, cs, raw = self.example()
        out = solve(r, cs[:2], raw)
        self.assertEqual(out['status'], 'delivery_order_shortage')
        self.assertEqual(out['shortage']['slots'], ['b'])
        self.assertEqual(out['shortage']['candidate_items'], [])
        self.assertIsNone(out['plan'])
        self.assertEqual(out['assignments'], [])
        self.assertEqual(out['delivery_sequence']['routes'], [])
        # Another order exists, but this mode must not silently choose it.
        _, alternate = sequence.allocate_sequenced_routes(r, cs[:2], raw)
        self.assertEqual(alternate['status'], 'allocated')
        self.assertNotEqual(alternate['delivery_sequence']['order'], ['a', 'b'])

    def check_oracle(self, request, items, raw, cells, limit=routes.MAX_ROUTE_STEPS):
        best, costs = oracle(request, items, cells, limit)
        out = solve(request, items, raw, max_route_steps=limit)
        if any(s.target not in cells for s in request.slots):
            self.assertEqual(out['status'], 'target_unestablished')
        elif best is None:
            self.assertIn(out['status'], ('shortage', 'delivery_order_shortage'))
            self.assertIsNone(out['plan'])
            if out['status'] == 'delivery_order_shortage':
                selected = out['shortage']['slots']
                neighbors = set().union(*(set(costs[name]) for name in selected))
                self.assertEqual(sorted(neighbors), out['shortage']['candidate_items'])
                self.assertEqual(len(selected)-len(neighbors), out['shortage']['missing'])
                self.assertGreater(out['shortage']['missing'], 0)
        else:
            self.assertEqual(out['status'], 'allocated')
            self.assertEqual((out['fixed_order_delivery']['complete_delivery_cost'],
                tuple(row['item'] for row in out['assignments'])), best)
            self.assertEqual(out['delivery_sequence']['total_delivery_route_steps'], best[0])
            self.assertEqual(out['request_digest'], request.digest)
        return out

    def test_all_512_obstacle_maps_match_full_assignment_and_coordinate_oracle(self):
        r = Request('fort', 7, (Slot('b', 'bed', (3, 3, 0)), Slot('a', 'bed', (2, 2, 0))))
        cs = (item(11, (1, 1, 0)), item(4, (3, 1, 0)), item(8, (1, 3, 0)))
        for mask in range(512):
            hidden = {i for i in range(9) if mask & (1 << i)}
            cells = {(1+i%3, 1+i//3, 0) for i in range(9) if i not in hidden}
            with self.subTest(mask=mask):
                self.check_oracle(r, cs, raw_map(hidden=hidden), cells)

    def test_250_random_domains_with_materials_exclusions_dependencies_and_limits(self):
        rng = random.Random(6131)
        points = [(x, y, 0) for y in range(1, 5) for x in range(1, 5)]
        for case in range(250):
            targets = rng.sample(points, 3)
            names = ['z', 'a', 'm']
            slots = tuple(Slot(name, 'bed', targets[i], (names[i-1],) if i and rng.randrange(2) else (),
                               (1, -1) if rng.randrange(3)==0 else None,
                               -1 if rng.randrange(4)==0 else None, rng.randrange(1, 9))
                          for i, name in enumerate(names))
            cs = tuple(item(i, rng.choice(points), (rng.randrange(1, 3), -1), subtype=rng.choice((-1, 0)))
                       for i in range(5))
            r = Request('fort', 7, slots, (0,) if case % 3 == 0 else ())
            hidden = {i for i in range(16) if rng.randrange(5)==0}
            cells = set(points) - {points[i] for i in hidden}
            self.check_oracle(r, cs, raw_map((4,4,1), hidden=hidden), cells, rng.randrange(1, 9))

    def test_execution_order_can_differ_from_lexical_item_vector_tie_break(self):
        r = Request('fort', 7, (Slot('a', 'bed', (3, 2, 0), ('z',)), Slot('z', 'bed', (1, 2, 0))))
        cs = (item(8, (2, 1, 0)), item(2, (2, 1, 0)))
        out = solve(r, cs, raw_map())
        self.assertEqual(out['delivery_sequence']['order'], ['z', 'a'])
        self.assertEqual([row['item'] for row in out['assignments']], [2, 8])
        self.assertEqual(out['plan']['steps'][0]['after'], ['z'])

    def test_route_limit_applies_after_earlier_targets_not_only_initial_routes(self):
        r = Request('fort', 7, (Slot('a', 'bed', (2, 2, 0)), Slot('b', 'bed', (3, 2, 0), material=(2,-1))))
        cs = (item(1, (2,2,0)), item(2, (1,2,0), (2,-1)))
        out = solve(r, cs, raw_map(), max_route_steps=2)
        self.assertEqual(out['status'], 'delivery_order_shortage')
        out = solve(r, cs, raw_map(), max_route_steps=4)
        self.assertEqual(out['status'], 'allocated')
        self.assertEqual(out['fixed_order_delivery']['complete_delivery_cost'], 4)
        self.assertEqual(out['total_route_steps'], 2)

    def test_missing_or_unknown_targets_never_prove_shortage(self):
        r, cs, _ = self.example()
        for hidden in ({6}, {7}):
            out = solve(r, cs, raw_map((5,3,1), hidden=hidden))
            self.assertEqual(out['status'], 'target_unestablished')
            self.assertIsNone(out['maximum_assignable'])
            self.assertIsNone(out['shortage'])

    def test_zero_step_self_delivery_and_shared_source_positions(self):
        r, cs, raw = self.example()
        r = replace(r, slots=(r.slots[0],))
        out = solve(r, cs, raw, max_route_steps=0)
        self.assertEqual(out['fixed_order_delivery']['complete_delivery_cost'], 0)
        self.assertEqual(out['assignments'][0]['item'], 1)

    def test_complete_32_target_request_fits_without_dependency_expansion(self):
        r = Request('fort', 7, tuple(Slot(f's{i:02}', 'bed', (i+1,2,0)) for i in range(32)))
        cs = tuple(item(i, (i+1,1,0)) for i in range(32))
        out = solve(r, cs, raw_map((34,3,1), origin=(0,0,0)))
        self.assertEqual(out['fixed_order_delivery']['complete_delivery_cost'], 32)
        self.assertEqual(len(out['delivery_sequence']['routes']), 32)
        self.assertTrue(all(not s['after'] for s in out['plan']['steps']))
        self.assertLessEqual(len(canonical(out)), routes.MAX_OUTPUT)

    def test_full_inventory_only_usable_later_item_is_beyond_initial_reduction(self):
        r, _, raw = self.example()
        cs = tuple(item(i, (2,2,0)) for i in range(65535)) + (item(65535, (5,2,0)),)
        out = solve(r, cs, raw)
        self.assertEqual([row['item'] for row in out['assignments']], [0,65535])
        self.assertEqual(out['fixed_order_delivery']['compatible_counts'][1]['count'], 1)
        self.assertLess(out['work_units'], routes.MAX_WORK)

    def test_explicit_vertical_bound_and_complementary_stairs_preserved(self):
        r = Request('fort', 7, (Slot('a', 'bed', (1,1,1), max_vertical_distance=1),))
        cs = (item(1, (1,1,0)),)
        for shapes, succeeds in [((6,7), True), ((3,3), False), ((7,6), False)]:
            out = solve(r, cs, raw_map((1,1,2), shapes=shapes))
            self.assertEqual(out['status']=='allocated', succeeds)
        no_vertical = replace(r, slots=(replace(r.slots[0], max_vertical_distance=0),))
        self.assertEqual(solve(no_vertical, cs, raw_map((1,1,2), shapes=(6,7)))['status'], 'shortage')

    def test_exact_whole_operation_work_bound_and_all_callback_cancellations(self):
        r, cs, raw = self.example()
        calls = 0
        def count():
            nonlocal calls
            calls += 1
        out = solve(r, cs, raw, guard=count)
        used = out['work_units']
        self.assertEqual(solve(r, cs, raw, maximum_work=used)['work_units'], used)
        with self.assertRaises(ValueError):
            solve(r, cs, raw, maximum_work=used-1)
        for stop in range(1, calls+1):
            current = 0
            def cancel():
                nonlocal current
                current += 1
                if current == stop:
                    raise InterruptedError('caller cancelled')
            with self.assertRaises(InterruptedError):
                solve(r, cs, raw, guard=cancel)

    def test_bad_forward_replay_cannot_publish_matching_as_proof(self):
        r, cs, raw = self.example()
        with patch.object(sequence, '_replay', return_value=([], None)):
            with self.assertRaises(ValueError):
                solve(r, cs, raw)

    def test_invalid_source_duplicate_items_or_parameters_refuse(self):
        r, cs, raw = self.example()
        for values in ((replace(r, folder='other'),cs,raw), (r,cs+(cs[0],),raw), (r,cs,raw[:-1])):
            with self.assertRaises(ValueError):
                solve(*values)
        for bound in (True,-1,routes.MAX_ROUTE_STEPS+1):
            with self.assertRaises(ValueError):
                solve(r,cs,raw,max_route_steps=bound)
        for bound in (0,True,routes.MAX_WORK+1):
            with self.assertRaises(ValueError):
                solve(r,cs,raw,maximum_work=bound)

    def test_candidate_order_and_python_hash_seed_do_not_change_result(self):
        r, cs, raw = self.example()
        out = solve(r, cs, raw)
        self.assertEqual(out, solve(r, tuple(reversed(cs)), raw))
        unsigned = {k:v for k,v in out.items() if k!='decision_digest'}
        self.assertEqual(out['decision_digest'], hashlib.sha256(
            b'dfmcp-route-furniture-decision/1\0'+canonical(unsigned)).hexdigest())
        script = ('from test_route_furniture_fixed_order import *; '
                  'r,cs,raw=FixedOrderTests().example(); print(solve(r,cs,raw)["decision_digest"])')
        for seed in ('1','81','random'):
            env = dict(os.environ, PYTHONHASHSEED=seed, PYTHONDONTWRITEBYTECODE='1',
                       PYTHONPATH=str(Path(__file__).parent))
            completed = subprocess.run([sys.executable,'-W','error','-c',script],env=env,
                                       capture_output=True,text=True,timeout=15,check=True)
            self.assertEqual(completed.stdout.strip(), out['decision_digest'])
            self.assertFalse(completed.stderr)


if __name__ == '__main__':
    unittest.main()
