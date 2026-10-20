"""Actual allocation/ordering code against independent exhaustive forward search."""
from dataclasses import replace
import hashlib
import itertools
import random
import struct
import unittest

from furniture_allocation import Candidate, Request, Slot
from furniture_plan import FurniturePlan, canonical
from route_furniture_optimal import allocate_optimal_routes
from route_furniture_sequence import allocate_sequenced_routes
import route_furniture_allocation as routes


def capture(width=11, height=1, depth=1, hidden=(), shapes=None, *, paused=True):
    """Independently emit the actual native map/1.5 bytes, not a Terrain double."""
    shapes = {} if shapes is None else shapes
    folder = b'fort'
    out = bytearray(b'DFMM1500' + struct.pack('>IIBIH', 0, 10, paused, 7, len(folder)) + folder)
    out += struct.pack('>9II', 128, 128, 8, 1, 1, 1, width, height, depth, width * height * depth)
    for tile in range(width * height * depth):
        if tile in hidden:
            out += b'\x01'
        else:
            out += b'\x02' + struct.pack('>IBBBBBBBIHH', 1, shapes.get(tile, 3), 0, 0, 0, 0, 0, 0, 1, 10015, 10015)
    return bytes(out)


def item(identity, kind, x, y=1, z=1, material=(1, -1), subtype=-1):
    return Candidate(identity, kind, (x, y, z), material, subtype)


def blocked_case():
    # a must precede b and removes x=4. Cheap chair 2 lies left of that cut;
    # slightly farther chair 3 lies to the right. Fixed-assignment sequencing
    # cannot solve this even though an alternative compatible assignment exists.
    request = Request('fort', 7, (Slot('a', 'bed', (4, 1, 1)),
                                  Slot('b', 'chair', (5, 1, 1), ('a',))))
    return request, (item(1, 'bed', 1), item(2, 'chair', 3), item(3, 'chair', 8)), capture()


def oracle_cost(request, candidates, width, height, hidden, limit):
    """Enumerate injections and forward topological permutations independently."""
    hidden = set(hidden)
    def index(point):
        x, y, z = point
        return (y - 1) * width + x - 1 if z == 1 and 1 <= x <= width and 1 <= y <= height else None
    def length(source, target, removed):
        source, target = index(source), index(target)
        if source is None or target is None or source in hidden | removed or target in hidden | removed:
            return None
        front, seen = [(source, 0)], {source}
        for tile, steps in front:
            if tile == target:
                return steps
            x, y = tile % width, tile // width
            for other in range(width * height):
                if (abs(other % width - x) + abs(other // width - y) == 1
                        and other not in seen | hidden | removed):
                    seen.add(other)
                    front.append((other, steps + 1))
        return None
    best = None
    for chosen in itertools.permutations(candidates, len(request.slots)):
        if any(c.id in request.excluded_items or s.kind != c.kind
               or s.material is not None and s.material != c.material
               or s.subtype is not None and s.subtype != c.subtype
               or abs(s.target[2] - c.position[2]) > s.max_vertical_distance
               or sum(abs(a - b) for a, b in zip(s.target, c.position)) > s.max_distance
               for s, c in zip(request.slots, chosen)):
            continue
        by_name = {s.name: (s, c) for s, c in zip(request.slots, chosen)}
        for order in itertools.permutations(by_name):
            done, removed, cost = set(), set(), 0
            for name in order:
                slot, candidate = by_name[name]
                steps = length(candidate.position, slot.target, removed)
                if not set(slot.after) <= done or steps is None or steps > limit:
                    break
                cost += steps
                removed.add(index(slot.target))
                done.add(name)
            else:
                best = cost if best is None else min(best, cost)
    return best


def detour_case():
    req = Request('fort', 7, tuple(Slot(chr(97 + i), 'bed', (t % 4 + 1, t // 4 + 1, 1))
                                  for i, t in enumerate((10, 9, 11))))
    candidates = (item(1, 'bed', 1, 2), item(2, 'bed', 1, 4),
                  item(3, 'bed', 1, 3), item(4, 'bed', 1, 3))
    return req, candidates, capture(4, 4, hidden=(14,))


class OptimalDeliveryTests(unittest.TestCase):
    def test_minimizes_complete_delivery_cost_not_just_initial_distances(self):
        req, items, raw = detour_case()
        baseline = allocate_sequenced_routes(req, items, raw)[1]
        self.assertEqual(baseline['delivery_sequence']['total_delivery_route_steps'], 9)
        effective, result = allocate_optimal_routes(req, items, raw, require_optimal=True)
        self.assertEqual(result['status'], 'allocated')
        proof = result['delivery_optimization']
        self.assertEqual(proof['best_complete_cost'], 7)
        self.assertEqual(proof['cost_lower_bound'], 7)
        self.assertEqual(proof['absolute_cost_gap'], 0)
        self.assertTrue(proof['cost_optimality_proven'])
        self.assertEqual(oracle_cost(req, items, 4, 4, (14,), 16), 7)
        self.assertEqual(sum(r['route_steps'] for r in result['delivery_sequence']['routes']), 7)
        self.assertEqual(FurniturePlan.from_json(result['plan']).digest, result['plan_digest'])
        self.assertEqual(effective.digest, result['request_digest'])
        self.assertFalse(result['game_effect_performed'])
        self.assertFalse(proof['retained_reviews_modified'])
        self.assertFalse(proof['native_hauling_time_minimized'])

    def test_search_continues_past_first_improvement_until_true_optimum(self):
        req = Request('fort', 7, tuple(Slot(chr(97+i), 'bed', (t%4+1,t//4+1,1))
                                      for i,t in enumerate((13,11,15))))
        items = (item(1,'bed',4,1),item(2,'bed',1,3),item(3,'bed',3,1),item(4,'bed',2,1))
        raw = capture(4,4,hidden=(5,6,12))
        first = allocate_optimal_routes(req,items,raw,max_search_nodes=4)[1]['delivery_optimization']
        self.assertEqual(first['initial_complete_cost'],12)
        self.assertEqual(first['best_complete_cost'],11)
        self.assertFalse(first['cost_optimality_proven'])
        best = allocate_optimal_routes(req,items,raw,require_optimal=True)[1]['delivery_optimization']
        self.assertEqual(best['best_complete_cost'],8)
        self.assertGreater(best['improvements'],1)
        self.assertEqual(oracle_cost(req,items,4,4,(5,6,12),16),8)
        self.assertTrue(best['cost_optimality_proven'])

    def test_recovers_a_feasible_alternative_and_proves_its_higher_minimum(self):
        req, items, raw = blocked_case()
        self.assertEqual(allocate_sequenced_routes(req, items, raw)[1]['status'], 'delivery_order_blocked')
        result = allocate_optimal_routes(req, items, raw, require_optimal=True)[1]
        self.assertEqual([row['item'] for row in result['assignments']], [1, 3])
        self.assertEqual(result['delivery_optimization']['best_complete_cost'], 6)
        self.assertTrue(result['delivery_optimization']['cost_optimality_proven'])

    def test_node_limit_returns_full_feasible_incumbent_with_explicit_gap(self):
        req, items, raw = detour_case()
        result = allocate_optimal_routes(req, items, raw, max_search_nodes=1)[1]
        proof = result['delivery_optimization']
        self.assertEqual(result['status'], 'allocated')
        self.assertEqual(len(result['assignments']), 3)
        self.assertEqual(len(result['delivery_sequence']['routes']), 3)
        self.assertFalse(proof['cost_optimality_proven'])
        self.assertFalse(proof['search_complete'])
        self.assertEqual((proof['best_complete_cost'], proof['cost_lower_bound'], proof['absolute_cost_gap']), (9, 7, 2))

    def test_strict_optimum_refuses_unproven_incumbent_without_partial_export(self):
        req, items, raw = detour_case()
        effective, result = allocate_optimal_routes(req, items, raw, max_search_nodes=1, require_optimal=True)
        self.assertEqual(effective, req)
        self.assertEqual(result['status'], 'delivery_optimization_incomplete')
        self.assertIsNone(result['plan'])
        self.assertEqual(result['assignments'], [])
        self.assertEqual(result['delivery_sequence']['routes'], [])
        self.assertFalse(result['delivery_sequence']['candidate_model_sequence_proven'])
        self.assertEqual(result['delivery_optimization']['best_complete_cost'], 9)

    def test_no_incumbent_limit_is_distinct_from_exhaustive_infeasibility(self):
        req, items, raw = blocked_case()
        incomplete = allocate_optimal_routes(req, items, raw, max_search_nodes=1)[1]
        self.assertEqual(incomplete['status'], 'delivery_optimization_incomplete')
        self.assertFalse(incomplete['delivery_optimization']['infeasibility_proven_in_supplied_model'])
        absent = allocate_optimal_routes(req, items[:2], raw)[1]
        self.assertEqual(absent['status'], 'delivery_assignment_blocked')
        self.assertTrue(absent['delivery_optimization']['infeasibility_proven_in_supplied_model'])
        self.assertFalse(absent['global_absence_proven'])
        self.assertEqual(absent['assignments'], [])

    def test_matching_lower_bound_can_prove_optimum_without_branching(self):
        req, items, raw = blocked_case()
        result = allocate_optimal_routes(req, items[::2], raw, max_search_nodes=1, require_optimal=True)[1]
        self.assertEqual(result['status'], 'allocated')
        self.assertEqual(result['delivery_optimization']['visited_nodes'], 0)
        self.assertTrue(result['delivery_optimization']['cost_optimality_proven'])

    def test_material_subtype_exclusion_distance_and_route_limit_are_preserved(self):
        req, items, raw = blocked_case()
        for changed in (replace(req, excluded_items=(3,)),
                        replace(req, slots=(req.slots[0], replace(req.slots[1], max_distance=2))),
                        replace(req, slots=(req.slots[0], replace(req.slots[1], material=(2,-1)))),
                        replace(req, slots=(req.slots[0], replace(req.slots[1], subtype=3)))):
            result = allocate_optimal_routes(changed, items, raw)[1]
            self.assertNotEqual(result['status'], 'allocated')
            self.assertIsNone(result['plan'])
        changed_items = (item(1,'bed',4), *items[1:])
        self.assertEqual(allocate_optimal_routes(req, changed_items, raw, max_route_steps=2)[1]['status'],
                         'delivery_assignment_blocked')

    def test_full_inventory_and_32_targets_remain_bounded(self):
        req, _, raw = blocked_case()
        candidates = (item(0, 'bed', 1),) + tuple(item(i, 'chair', 3) for i in range(1, 65535)) + (item(65535, 'chair', 8),)
        result = allocate_optimal_routes(req, candidates, raw)[1]
        self.assertEqual([row['item'] for row in result['assignments']], [0, 65535])
        req, items, _ = blocked_case()
        extra = tuple(Slot(f't{i:02}', 'table', (12+i,2,1)) for i in range(30))
        req = replace(req, slots=req.slots + extra)
        items += tuple(item(100+i,'table',12+i,2) for i in range(30))
        hidden = tuple(48+x for x in range(48) if x not in range(11,41))
        result = allocate_optimal_routes(req, items, capture(48,2,hidden=hidden))[1]
        self.assertEqual(result['status'], 'allocated')
        self.assertEqual(len(result['plan']['steps']), 32)
        self.assertEqual(len({row['item'] for row in result['assignments']}), 32)
        self.assertTrue(result['delivery_optimization']['cost_optimality_proven'])

    def test_every_guard_abort_and_exact_work_limit_publish_nothing(self):
        req, items, raw = detour_case()
        calls = []
        result = allocate_optimal_routes(req, items, raw, lambda: calls.append(1))[1]
        used = result['work_units']
        self.assertEqual(allocate_optimal_routes(req, items, raw, maximum_work=used)[1]['work_units'], used)
        with self.assertRaises(ValueError):
            allocate_optimal_routes(req, items, raw, maximum_work=used-1)
        class Cancelled(Exception):
            pass
        for stop in range(1,len(calls)+1):
            count = 0
            def guard():
                nonlocal count
                count += 1
                if count == stop:
                    raise Cancelled()
            with self.assertRaises(Cancelled):
                allocate_optimal_routes(req,items,raw,guard)

    def test_unknown_targets_shortage_and_bad_source_do_not_become_cost_proofs(self):
        req, items, _ = blocked_case()
        for candidates, raw in ((items,capture(hidden=(3,))), (items[:1],capture())):
            result = allocate_optimal_routes(req,candidates,raw)[1]
            self.assertNotIn('delivery_optimization',result)
            self.assertIsNone(result['plan'])
        with self.assertRaises(ValueError):
            allocate_optimal_routes(replace(req,folder='other'),items,capture())

    def test_invalid_limits_and_boolean_refuse_before_solver(self):
        req,items,raw=blocked_case()
        for value in (0,-1,4097,True,1.0,None):
            with self.assertRaises(ValueError):
                allocate_optimal_routes(req,items,raw,max_search_nodes=value)
        for value in (0,1,None,'yes'):
            with self.assertRaises(ValueError):
                allocate_optimal_routes(req,items,raw,require_optimal=value)

    def test_candidate_permutations_and_digest_are_deterministic(self):
        req,items,raw=detour_case()
        expected=allocate_optimal_routes(req,items,raw)
        for permuted in itertools.permutations(items):
            self.assertEqual(allocate_optimal_routes(req,permuted,raw),expected)
        result=expected[1]
        unsigned={k:v for k,v in result.items() if k!='decision_digest'}
        self.assertEqual(result['decision_digest'],hashlib.sha256(
            b'dfmcp-route-furniture-decision/1\0'+canonical(unsigned)).hexdigest())

    def test_all_512_obstacle_maps_match_exhaustive_numeric_optimum(self):
        req=Request('fort',7,(Slot('a','bed',(1,1,1)), Slot('b','bed',(2,2,1),('a',)),
                              Slot('c','bed',(3,3,1))))
        items=(item(1,'bed',1,3),item(2,'bed',3,1),item(3,'bed',2,1),item(4,'bed',2,3))
        for mask in range(512):
            hidden=tuple(i for i in range(9) if mask & (1<<i))
            cost=oracle_cost(req,items,3,3,hidden,8)
            result=allocate_optimal_routes(req,items,capture(3,3,hidden=hidden),max_route_steps=8,require_optimal=True)[1]
            self.assertEqual(result['status']=='allocated',cost is not None,mask)
            if cost is not None:
                self.assertEqual(result['delivery_optimization']['best_complete_cost'],cost,mask)
                self.assertTrue(result['delivery_optimization']['cost_optimality_proven'])

    def test_200_random_domains_match_independent_numeric_optimum(self):
        rng=random.Random(872)
        for case in range(200):
            targets=rng.sample(range(9),3)
            slots=tuple(Slot(chr(97+i),'bed',(t%3+1,t//3+1,1),('a',) if i and rng.randrange(2) else (),
                            material=(1,-1) if rng.randrange(3)==0 else None,
                            max_distance=rng.randrange(1,6)) for i,t in enumerate(targets))
            req=Request('fort',7,slots,(4,) if case%5==0 else ())
            items=tuple(item(i+1,'bed',rng.randrange(1,4),rng.randrange(1,4),
                             material=(rng.randrange(1,3),-1)) for i in range(4))
            hidden=tuple(i for i in range(9) if rng.randrange(6)==0)
            limit=rng.randrange(0,9)
            result=allocate_optimal_routes(req,items,capture(3,3,hidden=hidden),max_route_steps=limit,require_optimal=True)[1]
            cost=oracle_cost(req,items,3,3,hidden,limit)
            self.assertEqual(result['status']=='allocated',cost is not None,case)
            if cost is not None:
                self.assertEqual(result['delivery_optimization']['best_complete_cost'],cost,case)

    def test_vertical_delivery_uses_only_explicit_stair_policy(self):
        req, _, _=blocked_case()
        req=replace(req,slots=(req.slots[0],replace(req.slots[1],max_vertical_distance=1)))
        items=(item(1,'bed',1),item(2,'chair',3),item(3,'chair',8,z=2))
        hidden=tuple(i for i in range(11,22) if i!=18)
        result=allocate_optimal_routes(req,items,capture(depth=2,hidden=hidden,shapes={7:6,18:7}))[1]
        self.assertEqual(result['status'],'allocated')
        self.assertTrue(result['delivery_optimization']['cost_optimality_proven'])
        self.assertFalse(result['same_level_endpoints_only'])
        self.assertEqual(allocate_optimal_routes(req,items,capture(depth=2,hidden=hidden))[1]['status'],
                         'delivery_assignment_blocked')


if __name__=='__main__':
    unittest.main()
