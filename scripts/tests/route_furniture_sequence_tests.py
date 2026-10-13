"""Actual Python planner tests; synthetic map/1.5 bytes, never native evidence."""
from __future__ import annotations

from collections import deque
from dataclasses import replace
import hashlib
import itertools
from pathlib import Path
import random
import struct
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from furniture_allocation import Candidate, Request, Slot
from furniture_plan import FurniturePlan, canonical, progress
import route_furniture_allocation as routes
from route_furniture_sequence import allocate_sequenced_routes


def capture(size: tuple[int, int, int], shapes: list[int], *,
            origin: tuple[int, int, int] = (1, 1, 0),
            dimensions: tuple[int, int, int] = (128, 128, 128),
            excluded: dict[int, tuple[int, int, int, int]] | None = None) -> bytes:
    """Encode the real unchanged native value layout, including absent cells."""
    folder = b'fortress'
    raw = bytearray(b'DFMM1500' + struct.pack('>IIBIH', 1, 0, 1, 1, len(folder)) + folder)
    raw.extend(struct.pack('>10I', *dimensions, *origin, *size, len(shapes)))
    for index, shape in enumerate(shapes):
        if shape == 0:
            raw.append(0)
            continue
        depth, building, unit, walkable = (excluded or {}).get(index, (0, 0, 0, 1))
        raw.append(2)
        raw.extend(struct.pack('>IBBBBBBBIHH', 1, shape, depth, 0, 0, 0,
                               building, unit, walkable, 0, 0))
    return bytes(raw)


def corridor(*, after: tuple[str, ...] = ()) -> tuple[Request, tuple[Candidate, ...], bytes]:
    request = Request('fortress', 1, (
        Slot('a_near', 'bed', (2, 1, 0), material=(0, 1), subtype=1),
        Slot('b_far', 'bed', (4, 1, 0), after, material=(0, 2), subtype=2),
    ), (999,))
    candidates = (Candidate(10, 'bed', (1, 1, 0), (0, 1), 1),
                  Candidate(20, 'bed', (1, 1, 0), (0, 2), 2))
    return request, candidates, capture((4, 1, 1), [3] * 4)


def reference_distance(source, target, floors, blocked, limit):
    """Independent coordinate BFS: does not use any production graph helper."""
    if source in blocked or target in blocked:
        return None
    queue, seen = deque([(source, 0)]), {source}
    while queue:
        current, distance = queue.popleft()
        if current == target:
            return distance
        if distance >= limit:
            continue
        x, y, z = current
        for following in ((x + 1, y, z), (x - 1, y, z), (x, y + 1, z), (x, y - 1, z)):
            if following in floors and following not in blocked and following not in seen:
                seen.add(following)
                queue.append((following, distance + 1))
    return None


def valid_order(order, slots, items, floors, limit):
    done, blocked, distances = set(), set(), []
    for name in order:
        slot, item = slots[name], items[name]
        if not set(slot.after) <= done:
            return None
        distance = reference_distance(item.position, slot.target, floors, blocked, limit)
        if distance is None:
            return None
        distances.append(distance)
        blocked.add(slot.target)
        done.add(name)
    return distances


class SequenceTests(unittest.TestCase):
    def test_reorders_corridor_without_changing_items_or_original_selectors(self):
        request, candidates, raw = corridor()
        baseline = routes.allocate_routes(request, candidates, raw)
        effective, result = allocate_sequenced_routes(request, candidates, raw)
        sequence = result['delivery_sequence']
        self.assertEqual(result['status'], 'allocated')
        self.assertEqual(sequence['status'], 'reordered')
        self.assertEqual(sequence['order'], ['b_far', 'a_near'])
        self.assertEqual(sequence['original_order_first_blocked_slot'], 'b_far')
        self.assertEqual(result['assignments'], baseline['assignments'])
        self.assertEqual(sequence['original_request'], request.json())
        self.assertEqual(sequence['original_request_digest'], request.digest)
        self.assertEqual(result['request_digest'], effective.digest)
        self.assertNotEqual(request.digest, effective.digest)
        for old, new in zip(request.slots, effective.slots):
            self.assertEqual(replace(new, after=old.after), old)
            self.assertTrue(set(old.after) <= set(new.after))
        self.assertEqual(effective.excluded_items, request.excluded_items)
        plan = FurniturePlan.from_json(result['plan'])
        self.assertEqual(plan.digest, result['plan_digest'])
        self.assertEqual(progress(plan, {})['next_step'], 'b_far')
        self.assertEqual(progress(plan, {'b_far': 'placed'})['next_step'], 'a_near')
        self.assertEqual([row['route_steps'] for row in sequence['routes']], [3, 1])
        self.assertFalse(result['future_routes_preserved'])
        self.assertFalse(sequence['future_routes_guaranteed'])
        self.assertFalse(sequence['native_pathfinding_proven'])

    def test_keeps_a_safe_original_request_and_plan_byte_identical(self):
        request, candidates, raw = corridor()
        request = replace(request, slots=tuple(replace(s, after=('b_far',) if s.name == 'a_near' else ())
                                              for s in request.slots))
        baseline = routes.allocate_routes(request, candidates, raw)
        effective, result = allocate_sequenced_routes(request, candidates, raw)
        self.assertIs(effective, request)
        self.assertEqual(result['plan'], baseline['plan'])
        self.assertEqual(result['plan_digest'], baseline['plan_digest'])
        self.assertEqual(result['delivery_sequence']['status'], 'preserved')
        self.assertEqual(canonical(routes.allocate_routes(request, candidates, raw)), canonical(baseline))

    def test_unsatisfiable_original_dependencies_produce_no_executable_plan(self):
        request, candidates, raw = corridor(after=('a_near',))
        effective, result = allocate_sequenced_routes(request, candidates, raw)
        self.assertIs(effective, request)
        self.assertEqual(result['maximum_assignable'], 2)
        self.assertEqual(result['status'], 'delivery_order_blocked')
        self.assertIsNone(result['plan'])
        self.assertIsNone(result['plan_digest'])
        self.assertIsNone(result['shortage'])
        self.assertFalse(result['global_absence_proven'])
        report = result['delivery_sequence']
        self.assertEqual(report['order'], [])
        self.assertEqual(report['routes'], [])
        self.assertFalse(report['candidate_model_sequence_proven'])
        self.assertEqual(report['blockage']['infeasible_final_steps'], ['b_far'])
        self.assertFalse(report['alternative_item_assignments_searched'])

    def test_route_limit_is_rechecked_after_placements_not_just_connectivity(self):
        request = Request('fortress', 1, (
            Slot('a', 'bed', (2, 2, 0), subtype=1),
            Slot('b', 'bed', (3, 2, 0), subtype=2)))
        candidates = (Candidate(10, 'bed', (1, 2, 0), (0, 0), 1),
                      Candidate(20, 'bed', (1, 2, 0), (0, 0), 2))
        raw = capture((3, 3, 1), [3] * 9)
        _, enough = allocate_sequenced_routes(request, candidates, raw, max_route_steps=4)
        _, limited = allocate_sequenced_routes(request, candidates, raw, max_route_steps=3)
        self.assertEqual(enough['delivery_sequence']['status'], 'preserved')
        self.assertEqual([r['route_steps'] for r in enough['delivery_sequence']['routes']], [1, 4])
        self.assertEqual(limited['delivery_sequence']['order'], ['b', 'a'])
        self.assertEqual([r['route_steps'] for r in limited['delivery_sequence']['routes']], [2, 1])

    def test_an_unhauled_item_at_another_target_is_not_teleported(self):
        request = Request('fortress', 1, (
            Slot('a', 'bed', (2, 1, 0), subtype=1), Slot('b', 'bed', (3, 1, 0), subtype=2)))
        candidates = (Candidate(1, 'bed', (1, 1, 0), (0, 0), 1),
                      Candidate(2, 'bed', (2, 1, 0), (0, 0), 2))
        _, result = allocate_sequenced_routes(request, candidates, capture((3, 1, 1), [3] * 3))
        self.assertEqual(result['delivery_sequence']['order'], ['b', 'a'])

    def test_cross_level_constraints_and_observed_stair_topology_are_retained(self):
        request = Request('fortress', 1, (
            Slot('a', 'bed', (2, 1, 0), subtype=1),
            Slot('b', 'bed', (2, 1, 1), subtype=2, max_distance=4, max_vertical_distance=1)))
        candidates = (Candidate(1, 'bed', (1, 1, 0), (0, 0), 1),
                      Candidate(2, 'bed', (1, 1, 0), (0, 0), 2))
        raw = capture((3, 1, 2), [3, 3, 6, 0, 3, 7])
        effective, result = allocate_sequenced_routes(request, candidates, raw)
        self.assertEqual(result['delivery_sequence']['order'], ['b', 'a'])
        self.assertEqual(effective.slots[1].max_vertical_distance, 1)
        self.assertEqual(effective.slots[1].max_distance, 4)
        self.assertEqual(result['delivery_sequence']['routes'][0]['route_steps'], 4)
        _, broken = allocate_sequenced_routes(request, candidates,
                                              capture((3, 1, 2), [3, 3, 6, 0, 3, 6]))
        self.assertEqual(broken['status'], 'shortage')
        self.assertNotIn('delivery_sequence', broken)

    def test_shortage_or_unestablished_target_cannot_become_sequence_success(self):
        request, candidates, raw = corridor()
        for selected, terrain in ((candidates[:1], raw),
                                  (candidates, capture((4, 1, 1), [3, 0, 3, 3])),
                                  (candidates, capture((4, 1, 1), [3] * 4,
                                                       excluded={1: (1, 0, 0, 1)}))):
            with self.subTest(selected=len(selected), terrain=terrain.hex()):
                baseline = routes.allocate_routes(request, selected, terrain)
                effective, result = allocate_sequenced_routes(request, selected, terrain)
                self.assertIs(effective, request)
                self.assertEqual(result, baseline)
                self.assertIsNone(result['plan'])

    def test_one_work_allowance_spans_initial_allocation_and_sequencing(self):
        request, candidates, raw = corridor()
        baseline = routes.allocate_routes(request, candidates, raw)
        _, result = allocate_sequenced_routes(request, candidates, raw)
        used = result['work_units']
        self.assertGreater(used, baseline['work_units'])
        _, exact = allocate_sequenced_routes(request, candidates, raw, maximum_work=used)
        self.assertEqual(exact['work_units'], used)
        self.assertEqual(exact['maximum_work'], used)
        for maximum in (1, baseline['work_units'], used - 1):
            with self.subTest(maximum=maximum), self.assertRaisesRegex(ValueError, 'allowance exhausted'):
                allocate_sequenced_routes(request, candidates, raw, maximum_work=maximum)

    def test_guard_cancellation_during_sequence_never_returns_a_partial_plan(self):
        request, candidates, raw = corridor()
        calls = []
        routes.allocate_routes(request, candidates, raw, lambda: calls.append(1))
        threshold = len(calls) + 4
        invoked = 0
        def guard():
            nonlocal invoked
            invoked += 1
            if invoked >= threshold:
                raise TimeoutError('cancelled')
        with self.assertRaises(TimeoutError):
            allocate_sequenced_routes(request, candidates, raw, guard)
        self.assertEqual(invoked, threshold)

    def test_decision_digest_covers_order_constraints_and_prefix_witnesses(self):
        request, candidates, raw = corridor()
        _, result = allocate_sequenced_routes(request, candidates, raw)
        unsigned = {k: v for k, v in result.items() if k != 'decision_digest'}
        expected = hashlib.sha256(b'dfmcp-route-furniture-decision/1\0' + canonical(unsigned)).hexdigest()
        self.assertEqual(result['decision_digest'], expected)
        report = result['delivery_sequence']
        self.assertEqual([row['blocked_target_count'] for row in report['routes']], [0, 1])
        self.assertNotEqual(report['routes'][0]['blocked_targets_sha256'],
                            report['routes'][1]['blocked_targets_sha256'])
        report['routes'][1]['route_steps'] += 1
        self.assertNotEqual(expected, hashlib.sha256(
            b'dfmcp-route-furniture-decision/1\0' + canonical(unsigned)).hexdigest())

    def test_candidate_permutation_does_not_change_any_output_byte(self):
        request, candidates, raw = corridor()
        a, left = allocate_sequenced_routes(request, candidates, raw)
        b, right = allocate_sequenced_routes(request, tuple(reversed(candidates)), raw)
        self.assertEqual(a, b)
        self.assertEqual(canonical(left), canonical(right))

    def test_all_32_steps_compile_and_remain_a_verified_prefix(self):
        request = Request('fortress', 1, tuple(
            Slot(f's{i:02}', 'bed', (i + 2, 1, 0), subtype=i) for i in range(32)))
        items = tuple(Candidate(i + 1, 'bed', (1, 1, 0), (0, 0), i) for i in range(32))
        effective, result = allocate_sequenced_routes(request, items, capture((33, 1, 1), [3] * 33))
        plan = FurniturePlan.from_json(result['plan'])
        self.assertEqual([step.name for step in plan.ordered], [f's{i:02}' for i in reversed(range(32))])
        self.assertEqual(len(effective.slots), 32)
        self.assertEqual(len(result['delivery_sequence']['routes']), 32)
        self.assertLessEqual(len(canonical(result)), routes.MAX_OUTPUT)
        phases = {}
        for step in plan.ordered:
            self.assertEqual(progress(plan, phases)['next_step'], step.name)
            phases[step.name] = 'placed'
        self.assertEqual(progress(plan, phases)['status'], 'all_placed')

    def test_500_seeded_instances_against_all_dependency_valid_permutations(self):
        rng = random.Random(51013)
        coordinates = [(x, y, 0) for y in range(1, 4) for x in range(1, 4)]
        for case in range(500):
            count = rng.randint(1, 5)
            targets = rng.sample(coordinates, count)
            sources = [rng.choice(coordinates) for _ in range(count)]
            floors = {p for p in coordinates if rng.random() > .25} | set(targets) | set(sources)
            names = [f's{i}' for i in range(count)]
            rank = rng.sample(names, count)
            slots = tuple(Slot(name, 'bed', targets[i],
                          tuple(before for before in rank[:rank.index(name)] if rng.random() < .25),
                          subtype=i) for i, name in enumerate(names))
            items = tuple(Candidate(i + 1, 'bed', sources[i], (0, 0), i) for i in range(count))
            request = Request('fortress', 1, slots)
            raw = capture((3, 3, 1), [3 if p in floors else 0 for p in coordinates])
            limit = rng.randint(0, 8)
            by_slot, by_item = dict(zip(names, slots)), dict(zip(names, items))
            valid = {order for order in itertools.permutations(names)
                     if valid_order(order, by_slot, by_item, floors, limit) is not None}
            with self.subTest(case=case, count=count, limit=limit):
                effective, result = allocate_sequenced_routes(request, items, raw, max_route_steps=limit)
                self.assertEqual(result['status'] == 'allocated', bool(valid))
                if valid:
                    report = result['delivery_sequence']
                    order = tuple(report['order'])
                    self.assertIn(order, valid)
                    self.assertEqual(tuple(s.name for s in FurniturePlan.from_json(result['plan']).ordered), order)
                    expected = valid_order(order, by_slot, by_item, floors, limit)
                    self.assertEqual([r['route_steps'] for r in report['routes']], expected)
                    self.assertEqual(result['request_digest'], effective.digest)
                    for old, new in zip(request.slots, effective.slots):
                        self.assertEqual(replace(new, after=old.after), old)
                        self.assertTrue(set(old.after) <= set(new.after))
                else:
                    self.assertIsNone(result['plan'])
                    self.assertIsNone(result['plan_digest'])


if __name__ == '__main__':
    unittest.main()
