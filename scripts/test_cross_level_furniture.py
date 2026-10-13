"""Actual route/assignment/codec regressions with explicit native map byte fixtures.

No native game, transport, custody or placement authority is supplied here.
Beads: df-dfhack-bridge-plane-c-pic.3/.4; df-franken-storage-mvcc-54h.4.
"""
from collections import deque
from dataclasses import replace
import hashlib
import itertools
import random
import struct
import unittest

from furniture_allocation import Candidate, Request, Slot, allocate
from furniture_plan import canonical
import route_furniture_allocation as routes


def terrain(*, size=(6, 5, 3), changes=None, stairs=True, tick=100):
    """Encode every cell independently of the production terrain decoder."""
    origin = (1, 1, 0)
    dimensions = (size[0] + 2, size[1] + 2, size[2])
    cells = {(x, y, z): [2, 3, 0, 0, 0, 1]
             for z in range(size[2]) for y in range(1, size[1] + 1)
             for x in range(1, size[0] + 1)}
    if stairs:
        for z in range(size[2]):
            cells[(2, 2, z)][1] = 6 if z == 0 else 7 if z == size[2] - 1 else 8
    for position, update in (changes or {}).items():
        cells[position] = list(update)
    raw = b'DFMM1500' + struct.pack('>IIBIH', 0, tick, 1, 1, 4) + b'fort'
    raw += struct.pack('>10I', *dimensions, *origin, *size, len(cells))
    for values in cells.values():
        presence, shape, depth, building, units, walkable = values
        raw += bytes([presence])
        if presence == 2:
            raw += struct.pack('>IBBBBBBBIHH', 1, shape, depth, 0, 0, 0,
                               building, units, walkable, 10015, 10015)
    return raw, cells


def item(identity=1, position=(4, 2, 0), kind='bed', material=(419, -1), subtype=-1):
    return Candidate(identity, kind, position, material, subtype)


def request(vertical=0, *, target=(4, 2, 2), max_distance=65532):
    return Request('fort', 1, (Slot('bed', 'bed', target, max_distance=max_distance,
                                    max_vertical_distance=vertical),))


def oracle_distances(cells, root):
    """Coordinate-space BFS, independent of Terrain.adjacency/distances/index."""
    def usable(p):
        v = cells.get(p)
        return v is not None and v[0] == 2 and v[1] in (3, 6, 7, 8) and not any(v[2:5]) and v[5] > 0
    if not usable(root):
        return {}
    found, pending = {root: 0}, deque([root])
    while pending:
        current = pending.popleft()
        for axis in range(3):
            for delta in (-1, 1):
                target = list(current)
                target[axis] += delta
                target = tuple(target)
                if target in found or not usable(target):
                    continue
                if axis == 2:
                    lower, upper = (current, target) if delta > 0 else (target, current)
                    if cells[lower][1] not in (6, 8) or cells[upper][1] not in (7, 8):
                        continue
                found[target] = found[current] + 1
                pending.append(target)
    return found


class CrossLevelTests(unittest.TestCase):
    def test_legacy_canonical_request_and_result_unchanged(self):
        r = request(target=(4, 2, 0))
        old = {'schema': 'dfmcp.furniture-request/1', 'world_folder': 'fort', 'site': 1,
               'excluded_items': [], 'slots': [{'name': 'bed', 'kind': 'bed', 'target': [4, 2, 0],
                    'after': [], 'material': None, 'subtype': None, 'max_distance': 65532}]}
        self.assertEqual(r.json(), old)
        explicit = r.json()
        explicit['slots'][0]['max_vertical_distance'] = 0
        self.assertEqual(Request.from_json(explicit).digest, r.digest)
        raw, _ = terrain()
        result = routes.allocate_routes(r, (item(4, (3, 2, 0)), item(3, (5, 2, 0))), raw)
        self.assertEqual(result['assignments'][0]['item'], 3)
        self.assertTrue(result['same_level_endpoints_only'])
        self.assertNotIn('vertical_endpoint_policy', result)
        self.assertEqual(hashlib.sha256(canonical(result)).hexdigest(), LEGACY_RESULT_SHA256)

    def test_default_refuses_other_floors_even_with_complete_stairs(self):
        raw, _ = terrain()
        result = routes.allocate_routes(request(), (item(),), raw)
        self.assertEqual(result['status'], 'shortage')
        self.assertIsNone(result['plan'])
        self.assertEqual(result['shortage']['missing'], 1)

    def test_explicit_cross_level_routes_up_and_down_with_full_witness(self):
        raw, cells = terrain()
        for origin, target in [((4, 2, 0), (4, 2, 2)), ((4, 2, 2), (4, 2, 0))]:
            result = routes.allocate_routes(request(2, target=target), (item(position=origin),), raw)
            row = result['assignments'][0]
            self.assertEqual(result['status'], 'allocated')
            self.assertEqual(row['route_steps'], oracle_distances(cells, target)[origin])
            self.assertEqual(row['route_steps'], 6)
            self.assertEqual(row['manhattan_distance'], 2)
            self.assertFalse(result['same_level_endpoints_only'])
            self.assertEqual(result['vertical_limits'], [{'slot': 'bed', 'max_vertical_distance': 2}])
            # Independently reconstruct the stable index parent path and commitment.
            size = (6, 5, 3)
            index = lambda p: (p[2] * size[1] + p[1] - 1) * size[0] + p[0] - 1
            lengths = oracle_distances(cells, target)
            parent = {target: target}
            queue = deque([target])
            while queue:
                p = queue.popleft()
                successors = []
                for q in lengths:
                    if sum(abs(a-b) for a, b in zip(p, q)) == 1 and lengths[q] == lengths[p] + 1:
                        if p[2] != q[2]:
                            low, high = (p, q) if p[2] < q[2] else (q, p)
                            if cells[low][1] not in (6, 8) or cells[high][1] not in (7, 8):
                                continue
                        successors.append(q)
                for q in sorted(successors, key=index):
                    if q not in parent:
                        parent[q] = p
                        queue.append(q)
            digest = hashlib.sha256(b'dfmcp-furniture-candidate-route/1\0')
            digest.update(routes.PATH_POLICY.encode() + b'\0' + hashlib.sha256(raw).digest())
            digest.update(struct.pack('>III', index(origin), index(target), row['route_steps'] + 1))
            p = origin
            for _ in range(row['route_steps'] + 1):
                digest.update(struct.pack('>I', index(p)))
                p = parent[p]
            self.assertEqual(row['route_sha256'], digest.hexdigest())

    def test_all_stair_pair_orientations_require_a_connected_observed_path(self):
        for lower, upper in itertools.product(range(9), repeat=2):
            raw, _ = terrain(size=(3, 3, 2), stairs=False, changes={
                (2, 2, 0): (2, lower, 0, 0, 0, 1), (2, 2, 1): (2, upper, 0, 0, 0, 1)})
            out = routes.allocate_routes(request(1, target=(3, 2, 1)), (item(position=(1, 2, 0)),), raw)
            self.assertEqual(out['status'] == 'allocated', lower in (6, 8) and upper in (7, 8),
                             (lower, upper))

    def test_hidden_unallocated_liquid_occupied_or_unknown_stairs_never_connect(self):
        for values in [(0, 8, 0, 0, 0, 1), (1, 8, 0, 0, 0, 1), (2, 8, 1, 0, 0, 1),
                       (2, 8, 0, 1, 0, 1), (2, 8, 0, 0, 1, 1), (2, 8, 0, 0, 0, 0)]:
            raw, _ = terrain(changes={(2, 2, 1): values})
            out = routes.allocate_routes(request(2), (item(),), raw)
            self.assertEqual(out['status'], 'shortage')
            self.assertFalse(out['global_absence_proven'])
            self.assertIsNone(out['plan'])
        raw, _ = terrain(size=(6, 5, 2))
        out = routes.allocate_routes(request(2), (item(),), raw)
        self.assertEqual(out['status'], 'target_unestablished')
        self.assertIsNone(out['maximum_assignable'])

    def test_geometric_level_and_route_limits_are_independent_and_inclusive(self):
        raw, _ = terrain()
        for vertical, lower_bound, route_limit, success in [(2, 2, 6, True), (1, 2, 6, False),
                                                          (2, 1, 6, False), (2, 2, 5, False)]:
            out = routes.allocate_routes(request(vertical, max_distance=lower_bound), (item(),), raw,
                                         max_route_steps=route_limit)
            self.assertEqual(out['status'] == 'allocated', success)
        # Same-floor endpoints may use stairs for a detour; the cap is not a path excursion limit.
        self.assertEqual(routes.distance(request().slots[0], item(position=(4, 2, 2))), 0)

    def test_nonroute_allocator_rejects_vertical_opt_in(self):
        with self.assertRaisesRegex(ValueError, 'observed route allocator'):
            allocate(request(2), (item(),))
        with self.assertRaisesRegex(ValueError, 'observed route allocator'):
            allocate(request(2, target=(4, 2, 0)), (item(),))

    def test_material_subtype_kind_and_exclusions_are_never_relaxed(self):
        raw, _ = terrain()
        slot = Slot('bed', 'bed', (4, 2, 2), material=(419, -1), subtype=-1, max_vertical_distance=2)
        good = item(7)
        candidates = (item(1, material=(0, 1)), item(2, subtype=0), item(3, kind='chair'), good)
        r = Request('fort', 1, (slot,))
        self.assertEqual(routes.allocate_routes(r, candidates, raw)['assignments'][0]['item'], 7)
        out = routes.allocate_routes(replace(r, excluded_items=(7,)), candidates, raw)
        self.assertIsNone(out['plan'])
        self.assertEqual(out['maximum_assignable'], 0)

    def test_global_matching_mixed_limits_and_hall_shortage(self):
        raw, _ = terrain()
        r = Request('fort', 1, (Slot('a', 'bed', (4, 2, 2), max_vertical_distance=2),
                                Slot('b', 'bed', (5, 2, 0))))
        candidates = (item(1, (4, 2, 0)), item(2, (5, 2, 2)))
        out = routes.allocate_routes(r, candidates, raw)
        self.assertEqual([row['item'] for row in out['assignments']], [2, 1])
        self.assertEqual([v['max_vertical_distance'] for v in out['vertical_limits']], [2, 0])
        out = routes.allocate_routes(r, candidates[:1], raw)
        self.assertEqual(out['shortage']['slots'], ['a', 'b'])
        self.assertEqual(out['shortage']['candidate_items'], [1])
        self.assertEqual(out['shortage']['missing'], 1)
        self.assertIsNone(out['plan'])
        self.assertEqual(out['assignments'], [])

    def test_random_unpruned_global_optimum_and_order_independence(self):
        rng = random.Random(20261013)
        raw, cells = terrain()
        positions = [(x, y, z) for z in range(3) for y in (2, 3) for x in (3, 4, 5)]
        for case in range(180):
            slots = tuple(Slot(chr(97+i), 'bed', p, max_distance=rng.randrange(2, 10),
                               max_vertical_distance=rng.randrange(3))
                          for i, p in enumerate(rng.sample(positions, 3)))
            candidates = tuple(item(i+1, p, material=(419, rng.randrange(-1, 2)))
                               for i, p in enumerate(rng.sample(positions, 5)))
            r = Request('fort', 1, slots)
            choices = []
            maps = [oracle_distances(cells, s.target) for s in r.slots]
            for selection in itertools.permutations(candidates, 3):
                costs = []
                for s, c, lengths in zip(r.slots, selection, maps):
                    lower_bound = sum(abs(a-b) for a, b in zip(s.target, c.position))
                    if abs(s.target[2]-c.position[2]) > s.max_vertical_distance or lower_bound > s.max_distance:
                        break
                    if c.position not in lengths:
                        break
                    costs.append(lengths[c.position])
                if len(costs) == 3:
                    choices.append((sum(costs), tuple(c.id for c in selection)))
            out = routes.allocate_routes(r, candidates, raw)
            if choices:
                self.assertEqual((out['total_route_steps'], tuple(v['item'] for v in out['assignments'])),
                                 min(choices), case)
            else:
                self.assertEqual(out['status'], 'shortage', case)
            self.assertEqual(out, routes.allocate_routes(replace(r, slots=slots[::-1]), candidates[::-1], raw))

    def test_127_level_endpoints_and_32_slot_bound(self):
        raw, _ = terrain(size=(3, 3, 128))
        out = routes.allocate_routes(request(127, target=(3, 2, 127)), (item(position=(1, 2, 0)),), raw)
        self.assertEqual(out['assignments'][0]['route_steps'], 129)
        raw, _ = terrain(size=(8, 8, 2))
        slots = tuple(Slot(f'bed-{i:02}', 'bed', (3+i%6, 1+i//6, 1), max_vertical_distance=1)
                      for i in range(32))
        candidates = tuple(item(i, (3+i%6, 1+i//6, 0)) for i in range(32))
        out = routes.allocate_routes(Request('fort', 1, slots), candidates, raw)
        self.assertEqual(out['maximum_assignable'], 32)
        self.assertEqual(len({v['item'] for v in out['assignments']}), 32)
        self.assertLessEqual(len(canonical(out)), routes.MAX_OUTPUT)

    def test_closed_request_bounds_and_digest_bind_every_vertical_limit(self):
        r = request(2)
        self.assertEqual(Request.decode(canonical(r.json())), r)
        for limit in [None, True, False, -1, 128, 2.0, '2', [], {}]:
            value = r.json()
            value['slots'][0]['max_vertical_distance'] = limit
            with self.subTest(limit=limit), self.assertRaises((ValueError, TypeError)):
                Request.from_json(value)
        self.assertEqual(len({request(i).digest for i in range(4)}), 4)
        value = r.json()
        value['slots'][0]['allow_unobserved_stairs'] = True
        with self.assertRaises(ValueError):
            Request.from_json(value)

    def test_shared_work_and_cancellation_refuse_without_partial_plan(self):
        raw, _ = terrain()
        result = routes.allocate_routes(request(2), (item(),), raw)
        count = result['work_units']
        for limit in (0, 1, count-1):
            with self.assertRaises(ValueError):
                routes.allocate_routes(request(2), (item(),), raw, maximum_work=limit)
        self.assertEqual(routes.allocate_routes(request(2), (item(),), raw,
                         maximum_work=count)['status'], 'allocated')
        calls = 0
        def cancelled():
            nonlocal calls
            calls += 1
            if calls == 3:
                raise ValueError('cancelled')
        with self.assertRaisesRegex(ValueError, 'cancelled'):
            routes.allocate_routes(request(2), (item(),), raw, guard=cancelled)


LEGACY_RESULT_SHA256 = '99ccfc2fd6d3cbd20770f417f08e9f7fc11611a5fee0cdc54191b692e4080a76'

if __name__ == '__main__':
    unittest.main(verbosity=2)
