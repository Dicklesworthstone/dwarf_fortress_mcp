"""Actual allocation/ordering code against independent exhaustive forward search."""
from dataclasses import replace
import hashlib
import itertools
import random
import struct
import unittest

from furniture_allocation import Candidate, Request, Slot
from furniture_plan import FurniturePlan, canonical
from route_furniture_joint import allocate_joint_routes
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


def oracle(request, candidates, width, height, hidden, limit):
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
            done, removed = set(), set()
            for name in order:
                slot, candidate = by_name[name]
                steps = length(candidate.position, slot.target, removed)
                if not set(slot.after) <= done or steps is None or steps > limit:
                    break
                removed.add(index(slot.target))
                done.add(name)
            else:
                return True
    return False


class JointDeliveryTests(unittest.TestCase):
    def test_alternative_item_rescues_undeliverable_cheapest_assignment(self):
        req, items, raw = blocked_case()
        self.assertEqual(allocate_sequenced_routes(req, items, raw)[1]['status'], 'delivery_order_blocked')
        effective, result = allocate_joint_routes(req, items, raw)
        self.assertEqual(result['status'], 'allocated')
        self.assertEqual([r['item'] for r in result['assignments']], [1, 3])
        self.assertEqual(effective, req)
        seq = result['delivery_sequence']
        self.assertEqual(seq['order'], ['a', 'b'])
        self.assertEqual([r['route_steps'] for r in seq['routes']], [3, 3])
        self.assertEqual(seq['changed_item_count'], 1)
        self.assertFalse(seq['selected_items_preserved'])
        self.assertFalse(seq['minimum_initial_cost_proven'])
        self.assertFalse(result['game_effect_performed'])
        self.assertFalse(result['items_reserved'])
        self.assertTrue(seq['candidate_model_sequence_proven'])
        self.assertEqual(FurniturePlan.from_json(result['plan']).digest, result['plan_digest'])
        unsigned = {k: v for k, v in result.items() if k != 'decision_digest'}
        self.assertEqual(result['decision_digest'], hashlib.sha256(
            b'dfmcp-route-furniture-decision/1\0' + canonical(unsigned)).hexdigest())

    def test_already_deliverable_and_shortage_results_are_byte_identical(self):
        req, items, raw = blocked_case()
        for candidates in (items[::2], items[:1]):
            self.assertEqual(allocate_joint_routes(req, candidates, raw),
                             allocate_sequenced_routes(req, candidates, raw))
        free = replace(req, slots=(req.slots[0], replace(req.slots[1], after=())))
        self.assertEqual(allocate_joint_routes(free, items, raw), allocate_sequenced_routes(free, items, raw))

    def test_search_limit_never_becomes_exhaustion_or_partial_plan(self):
        req, items, raw = blocked_case()
        for limit in (1, 2):
            effective, result = allocate_joint_routes(req, items, raw, max_search_nodes=limit)
            self.assertEqual(effective, req)
            self.assertEqual(result['status'], 'delivery_assignment_search_limit')
            self.assertIsNone(result['plan'])
            self.assertEqual(result['assignments'], [])
            self.assertEqual(result['delivery_sequence']['routes'], [])
            self.assertFalse(result['delivery_sequence']['search']['search_exhausted'])
            self.assertEqual(result['delivery_sequence']['search']['nodes'], limit)
        self.assertEqual(allocate_joint_routes(req, items, raw, max_search_nodes=3)[1]['status'], 'allocated')

    def test_joint_search_backtracks_instead_of_consuming_the_only_constrained_item(self):
        request = Request('fort', 7, (Slot('a', 'chair', (4, 1, 1), material=(1, -1)),
                                      Slot('b', 'chair', (5, 1, 1), ('a',))))
        candidates = (item(1, 'chair', 8), item(2, 'chair', 9, material=(2, -1)),
                      item(3, 'chair', 3, material=(2, -1)))
        self.assertEqual(allocate_sequenced_routes(request, candidates, capture())[1]['status'],
                         'delivery_order_blocked')
        result = allocate_joint_routes(request, candidates, capture())[1]
        self.assertEqual(result['status'], 'allocated')
        self.assertEqual([row['item'] for row in result['assignments']], [1, 2])
        self.assertGreater(result['delivery_sequence']['search']['backtracks'], 0)

    def test_exhausted_fixed_domain_not_global_absence(self):
        req, items, raw = blocked_case()
        result = allocate_joint_routes(req, items[:2], raw)[1]
        self.assertEqual(result['status'], 'delivery_assignment_blocked')
        self.assertTrue(result['delivery_sequence']['search']['search_exhausted'])
        self.assertFalse(result['global_absence_proven'])
        self.assertIsNone(result['plan'])
        self.assertEqual(result['assignments'], [])

    def test_selectors_exclusions_and_original_limits_cannot_be_relaxed(self):
        req, items, raw = blocked_case()
        variations = [
            replace(req, excluded_items=(3,)),
            replace(req, slots=(req.slots[0], replace(req.slots[1], material=(2, -1)))),
            replace(req, slots=(req.slots[0], replace(req.slots[1], subtype=3))),
            replace(req, slots=(req.slots[0], replace(req.slots[1], max_distance=2))),
        ]
        for changed in variations:
            with self.subTest(request=changed):
                result = allocate_joint_routes(changed, items, raw)[1]
                self.assertNotEqual(result['status'], 'allocated')
                self.assertIsNone(result['plan'])
        # An independent route cap also rules out the fallback, without relaxing
        # the shortest-path cap into an unconstrained reachability question.
        candidates = (item(1, 'bed', 4), *items[1:])
        self.assertEqual(allocate_joint_routes(req, candidates, raw, max_route_steps=2)[1]['status'],
                         'delivery_assignment_blocked')

    def test_unknown_target_and_wrong_source_fail_before_joint_search(self):
        req, items, _ = blocked_case()
        result = allocate_joint_routes(req, items, capture(hidden=(3,)))[1]
        self.assertEqual(result['status'], 'target_unestablished')
        self.assertNotIn('delivery_sequence', result)
        with self.assertRaises(ValueError):
            allocate_joint_routes(replace(req, folder='other'), items, capture())

    def test_full_inventory_is_searched_beyond_cheapest_n(self):
        req, _, raw = blocked_case()
        candidates = (item(0, 'bed', 1),) + tuple(item(i, 'chair', 3) for i in range(1, 65535)) + (item(65535, 'chair', 8),)
        result = allocate_joint_routes(req, candidates, raw)[1]
        self.assertEqual(result['candidate_count'], 65536)
        self.assertEqual([row['item'] for row in result['assignments']], [0, 65535])
        self.assertFalse(result['delivery_sequence']['search']['candidate_lists_truncated'])

    def test_all_32_slots_preserved_through_fallback(self):
        req, items, _ = blocked_case()
        extra = tuple(Slot(f't{i:02}', 'table', (12 + i, 2, 1)) for i in range(30))
        req = replace(req, slots=req.slots + extra)
        items += tuple(item(100 + i, 'table', 12 + i, 2, material=(i + 10, -1)) for i in range(30))
        # Keep only the first row and the table targets; no bypass around x=4.
        hidden = tuple(48 + x for x in range(48) if x not in range(11, 41))
        result = allocate_joint_routes(req, items, capture(48, 2, hidden=hidden))[1]
        self.assertEqual(result['status'], 'allocated')
        self.assertEqual(len(result['plan']['steps']), 32)
        self.assertEqual(len(result['delivery_sequence']['routes']), 32)
        self.assertEqual(len({r['item'] for r in result['assignments']}), 32)
        self.assertEqual(result['delivery_sequence']['search']['nodes'], 33)

    def test_exact_work_boundary_and_every_callback_abort(self):
        req, items, raw = blocked_case()
        calls = []
        result = allocate_joint_routes(req, items, raw, lambda: calls.append(1))[1]
        used = result['work_units']
        self.assertEqual(allocate_joint_routes(req, items, raw, maximum_work=used)[1]['work_units'], used)
        with self.assertRaises(ValueError):
            allocate_joint_routes(req, items, raw, maximum_work=used - 1)
        class Cancelled(Exception):
            pass
        for stop in range(1, len(calls) + 1):
            count = 0
            def guard():
                nonlocal count
                count += 1
                if count == stop:
                    raise Cancelled()
            with self.assertRaises(Cancelled):
                allocate_joint_routes(req, items, raw, guard)

    def test_invalid_search_limits_refuse_before_calling_legacy_solver(self):
        req, items, raw = blocked_case()
        for value in (0, -1, 100001, True, 1.0, None):
            with self.subTest(value=value), self.assertRaises(ValueError):
                allocate_joint_routes(req, items, raw, max_search_nodes=value)

    def test_input_order_cannot_change_solution_counters_or_trace(self):
        req, items, raw = blocked_case()
        expected = allocate_joint_routes(req, items, raw)
        for permuted in itertools.permutations(items):
            self.assertEqual(allocate_joint_routes(req, permuted, raw), expected)

    def test_cross_level_fallback_requires_explicit_opt_in_and_observed_stairs(self):
        req, _, _ = blocked_case()
        # Vertical alternative on (5,1,2) reaches b directly. Cheap chair is
        # blocked by a; only complementary stairs support the alternate source.
        req = replace(req, slots=(req.slots[0], replace(req.slots[1], max_vertical_distance=1)))
        items = (item(1, 'bed', 1), item(2, 'chair', 3), item(3, 'chair', 8, z=2))
        shapes = {7: 6, 18: 7}
        raw = capture(depth=2, shapes=shapes, hidden=tuple(i for i in range(11, 22) if i != 18))
        result = allocate_joint_routes(req, items, raw)[1]
        self.assertEqual(result['status'], 'allocated')
        self.assertEqual(result['assignments'][1]['item'], 3)
        self.assertFalse(result['same_level_endpoints_only'])
        strict = replace(req, slots=(req.slots[0], replace(req.slots[1], max_vertical_distance=0)))
        self.assertEqual(allocate_joint_routes(strict, items, raw)[1]['status'], 'delivery_assignment_blocked')
        self.assertEqual(allocate_joint_routes(req, items, capture(depth=2, hidden=tuple(i for i in range(11,22) if i != 18)))[1]['status'],
                         'delivery_assignment_blocked')

    def test_all_512_obstacle_maps_against_exhaustive_injections_and_forward_orders(self):
        slots = (Slot('a', 'bed', (1, 1, 1)), Slot('b', 'bed', (2, 2, 1), ('a',)),
                 Slot('c', 'bed', (3, 3, 1)))
        req = Request('fort', 7, slots)
        items = (item(1, 'bed', 1, 3), item(2, 'bed', 3, 1), item(3, 'bed', 2, 1), item(4, 'bed', 2, 3))
        for mask in range(512):
            hidden = tuple(i for i in range(9) if mask & (1 << i))
            expected = oracle(req, items, 3, 3, hidden, 8)
            result = allocate_joint_routes(req, items, capture(3, 3, hidden=hidden), max_route_steps=8)[1]
            self.assertEqual(result['status'] == 'allocated', expected, (mask, result['status']))

    def test_random_domains_against_exhaustive_forward_oracle(self):
        rng = random.Random(72041)
        for case in range(150):
            targets = rng.sample(range(9), 3)
            slots = tuple(Slot(chr(97 + i), 'bed', (t % 3 + 1, t // 3 + 1, 1),
                          ('a',) if i and rng.randrange(2) else (),
                          material=(1, -1) if rng.randrange(3) == 0 else None,
                          max_distance=rng.randrange(1, 6)) for i, t in enumerate(targets))
            req = Request('fort', 7, slots, (4,) if case % 5 == 0 else ())
            items = tuple(item(i + 1, 'bed', rng.randrange(1,4), rng.randrange(1,4),
                               material=(rng.randrange(1,3), -1)) for i in range(4))
            hidden = tuple(i for i in range(9) if rng.randrange(4) == 0)
            limit = rng.randrange(0, 9)
            result = allocate_joint_routes(req, items, capture(3,3,hidden=hidden), max_route_steps=limit)[1]
            self.assertEqual(result['status'] == 'allocated', oracle(req, items, 3,3,hidden,limit), case)


if __name__ == '__main__':
    unittest.main()
