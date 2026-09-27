"""Execute the actual allocator against independent exhaustive/permutation oracles."""
from dataclasses import replace
import itertools
import random
import unittest

import furniture_allocation as a
from furniture_plan import FurniturePlan, canonical


def slot(name='bed', target=(10, 10, 2), **kw):
    return a.Slot(name, 'bed', target, **kw)


def item(identity=0, position=(10, 10, 2), material=(1, -1), **kw):
    return a.Candidate(identity, 'bed', position, material, kw.get('subtype', -1))


def request(slots=None, excluded=()):
    return a.Request('region1', 2, tuple(slots or [slot()]), excluded)


def brute(edges):
    """Independent exhaustive objective: cardinality, then sum, then ID vector."""
    ids = sorted(set().union(*(set(row) for row in edges)))
    best, cardinality = None, 0
    for size in range(min(len(edges), len(ids)) + 1):
        for rows in itertools.combinations(range(len(edges)), size):
            for chosen in itertools.permutations(ids, size):
                if all(identity in edges[row] for row, identity in zip(rows, chosen)):
                    cardinality = max(cardinality, size)
                    if size == len(edges):
                        value = sum(edges[row][identity] for row, identity in enumerate(chosen)), chosen
                        best = value if best is None else min(best, value)
    return cardinality, best


class AllocationTests(unittest.TestCase):
    def test_all_4096_three_by_four_graphs_against_independent_oracle(self):
        for mask in range(4096):
            edges = tuple({j: (i * 7 + j * 3 + mask) % 11 for j in range(4)
                           if mask & (1 << (i * 4 + j))} for i in range(3))
            maximum, optimal = brute(edges)
            owners, shortage = a._maximum_matching(edges, a.idle)
            self.assertEqual(len(owners), maximum, mask)
            if optimal is not None:
                self.assertIsNone(shortage)
                chosen = a._minimum_cost(edges, a.idle)
                self.assertEqual((sum(edges[i][j] for i, j in enumerate(chosen)), chosen), optimal, mask)
            else:
                self.assertIsNotNone(shortage)
                rows = shortage['rows']
                neighbors = set().union(*(set(edges[i]) for i in rows))
                self.assertEqual(set(shortage['candidate_items']), neighbors, mask)
                self.assertEqual(shortage['missing'], len(rows) - len(neighbors), mask)
                self.assertGreater(shortage['missing'], 0)

    def test_global_assignment_avoids_greedy_material_starvation(self):
        r = request([slot('a', (10, 10, 2)), slot('b', (11, 10, 2), material=(7, 8))])
        result = a.allocate(r, (item(1, material=(7, 8)), item(2, (20, 10, 2))))
        self.assertEqual(result['status'], 'allocated')
        self.assertEqual([s['item'] for s in result['assignments']], [2, 1])
        self.assertEqual(result['total_distance'], 11)
        actual = FurniturePlan.from_json(result['plan'])
        self.assertEqual(actual.digest, result['plan_digest'])

    def test_sum_distance_precedes_id_vector_and_lexical_slots_break_ties(self):
        r = request([slot('z', (12, 10, 2)), slot('a', after=('z',))])
        result = a.allocate(r, (item(0, (30, 10, 2)), item(a.MAX_ID, (10, 10, 2)), item(7, (12, 10, 2))))
        self.assertEqual([s['item'] for s in result['assignments']], [a.MAX_ID, 7])
        self.assertEqual(result['total_distance'], 0)
        equal = a.allocate(r, (item(8, (11, 10, 2)), item(2, (11, 10, 2))))
        self.assertEqual([s['item'] for s in equal['assignments']], [2, 8])
        self.assertEqual([s.name for s in FurniturePlan.from_json(equal['plan']).ordered], ['z', 'a'])

    def test_hall_shortage_not_individual_counts_and_no_partial_plan(self):
        r = request([slot('a'), slot('b', (11, 10, 2)), slot('c', (100, 10, 2), material=(8, 9))])
        result = a.allocate(r, (item(4), item(5, (100, 10, 2), material=(8, 9))))
        self.assertEqual(result['status'], 'shortage')
        self.assertEqual(result['maximum_assignable'], 2)
        self.assertIsNone(result['plan'])
        self.assertIsNone(result['plan_digest'])
        self.assertEqual(result['assignments'], [])
        witness = result['shortage']
        self.assertEqual(witness['missing'], 1)
        self.assertEqual(witness['slots'], ['a', 'b', 'c'])
        self.assertEqual(witness['candidate_items'], [4, 5])
        self.assertTrue(all(row['count'] > 0 for row in result['compatible_counts']))

    def test_150_random_inventories_against_full_unpruned_permutation_oracle(self):
        rng = random.Random(11914)
        for trial in range(150):
            n, m = rng.randrange(1, 5), rng.randrange(0, 8)
            slots = tuple(slot(str(i), (i + 5, 10, 2),
                               material=None if rng.randrange(2) else (rng.randrange(2), -1),
                               max_distance=rng.randrange(1, 16)) for i in range(n))
            r = request(slots, (1,) if m > 1 and trial % 3 == 0 else ())
            items = tuple(item(i, (rng.randrange(25), 10, 2), (rng.randrange(2), -1)) for i in range(m))
            edges = tuple({v.id: d for v in items if v.id not in r.excluded_items
                           and (d := a.distance(s, v)) is not None} for s in r.slots)
            count, best = brute(edges)
            out = a.allocate(r, items)
            self.assertEqual(out['maximum_assignable'], count, trial)
            if best is not None:
                self.assertEqual((out['total_distance'], tuple(v['item'] for v in out['assignments'])), best, trial)
            else:
                witness = out['shortage']
                rows = [i for i, s in enumerate(r.slots) if s.name in witness['slots']]
                neighbors = set().union(*(set(edges[i]) for i in rows))
                self.assertEqual(set(witness['candidate_items']), neighbors)
                self.assertEqual(witness['missing'], len(rows) - len(neighbors))

    def test_same_level_distance_subtype_material_kind_and_exclusions(self):
        r = request([slot(material=(1, -1), subtype=-1, max_distance=2)], excluded=(6,))
        candidates = (item(0, (10, 10, 3)), item(1, (13, 10, 2)), item(2, material=(2, -1)),
                      item(3, subtype=5), replace(item(4), kind='chair'), item(6), item(7, (12, 10, 2)))
        out = a.allocate(r, candidates)
        self.assertEqual(out['compatible_counts'], [{'slot': 'bed', 'count': 1}])
        self.assertEqual(out['assignments'][0]['item'], 7)
        self.assertEqual(out['total_distance'], 2)

    def test_all_65536_items_and_32_slots_exact_bounded_solution(self):
        r = request(tuple(slot(f'{i:02}', (i + 1, 10, 2)) for i in range(32)))
        items = tuple(item(i, (50, 10, 2)) for i in range(65536))
        result = a.allocate(r, items)
        self.assertEqual([v['item'] for v in result['assignments']], list(range(32)))
        self.assertEqual(result['total_distance'], sum(49 - i for i in range(32)))
        self.assertTrue(all(v['count'] == 65536 for v in result['compatible_counts']))
        self.assertLessEqual(len(canonical(result)), 49152)
        with self.assertRaises(ValueError):
            a.allocate(r, items + (item(65536),))

    def test_maximum_1024_reduced_union(self):
        r = request(tuple(slot(f'{i:02}', (1 + i * 100, 10, 2), max_distance=0) for i in range(32)))
        items = tuple(item(i * 32 + j, (1 + i * 100, 10, 2)) for i in range(32) for j in range(32))
        out = a.allocate(r, items)
        self.assertEqual([v['item'] for v in out['assignments']], list(range(0, 1024, 32)))
        self.assertEqual(out['total_distance'], 0)

    def test_permutations_of_input_produce_identical_bytes(self):
        slots = [slot('c', (15, 10, 2)), slot('b', (11, 10, 2)), slot('a')]
        items = (item(9, (11, 10, 2)), item(1, (14, 10, 2)), item(4))
        expected = canonical(a.allocate(request(slots), items))
        for ss in itertools.permutations(slots):
            for ii in itertools.permutations(items):
                self.assertEqual(canonical(a.allocate(request(ss), ii)), expected)

    def test_empty_inventory_has_complete_explicit_shortage(self):
        out = a.allocate(request(), ())
        self.assertEqual(out['shortage'], {'slots': ['bed'], 'candidate_items': [], 'missing': 1,
                                           'scope': 'complete_supplied_same_level_candidate_graph'})
        self.assertEqual(out['maximum_assignable'], 0)
        self.assertIsNone(out['plan'])

    def test_request_normalization_roundtrip_and_actual_plan_compatibility(self):
        raw = b'{"schema":"dfmcp.furniture-request/1","world_folder":"region1","site":2,"slots":[{"name":"bed","kind":"bed","target":[10,10,2]}]}'
        r = a.Request.decode(raw)
        self.assertEqual(a.Request.decode(canonical(r.json())), r)
        out = a.allocate(r, (item(),))
        self.assertEqual(FurniturePlan.decode(canonical(out['plan'])).digest, out['plan_digest'])
        for key in ('placement_eligibility_proven', 'pathfinding_proven', 'items_reserved',
                    'game_effect_performed', 'production_admitted'):
            self.assertIs(out[key], False)

    def test_invalid_requests_rejected_before_candidate_scan(self):
        value = request().json()
        bad = [dict(value, schema='wrong'), dict(value, extra=1), dict(value, slots=[]),
               dict(value, slots=value['slots'] * 33), dict(value, site=True),
               dict(value, world_folder=''), dict(value, world_folder='x\0y'),
               dict(value, excluded_items=[1, 1]), dict(value, excluded_items=[False])]
        for changes in ({'target': [0, 1, 0]}, {'target': [1, 1, True]}, {'kind': 'throne'},
                        {'after': ['missing']}, {'after': ['bed']}, {'material': [1]},
                        {'material': [True, 1]}, {'subtype': False}, {'max_distance': -1},
                        {'max_distance': 65533}, {'name': 'x' * 49}, {'surprise': 1}):
            bad.append(dict(value, slots=[dict(value['slots'][0], **changes)]))
        for v in bad:
            with self.subTest(v=v), self.assertRaises(ValueError):
                a.Request.from_json(v)
        with self.assertRaises(ValueError):
            request([slot('a', after=('b',)), slot('b', (11, 10, 2), after=('a',))])
        with self.assertRaises(ValueError):
            request([slot('a'), slot('b')])
        with self.assertRaises(ValueError):
            a.Request.decode(b'{"schema":1,"schema":2}')
        for raw in (b' ' * 16385, b'[' * 9 + b']' * 9, b'{', b'\xff', b'NaN'):
            with self.assertRaises(ValueError):
                a.Request.decode(raw)

    def test_invalid_candidate_even_when_excluded_or_irrelevant(self):
        with self.assertRaises(ValueError):
            a.allocate(request(excluded=(0,)), (item(), item()))
        with self.assertRaises(ValueError):
            a.allocate(request(), [item()])
        with self.assertRaises(ValueError):
            a.allocate(request(), (object(),))
        for changes in ({'id': True}, {'id': 2147483647}, {'kind': 'other'}, {'material': (-1, 2)},
                        {'position': (0, 0, -1)}, {'subtype': True}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                replace(item(), **changes)

    def test_budget_interruption_never_returns_partial_allocation(self):
        class Exhausted(Exception):
            pass
        r = request([slot('a'), slot('b', (11, 10, 2))])
        for limit in (0, 1, 4, 10, 20):
            calls = 0
            def guard():
                nonlocal calls
                calls += 1
                if calls > limit:
                    raise Exhausted()
            with self.assertRaises(Exhausted):
                a.allocate(r, (item(0), item(1)), guard)


if __name__ == '__main__':
    unittest.main()
