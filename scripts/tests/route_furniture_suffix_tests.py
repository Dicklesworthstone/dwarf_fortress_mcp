"""Actual suffix/checker modules; independent synthetic map bytes and BFS oracle."""
from __future__ import annotations

from collections import deque
from dataclasses import replace
import hashlib
from pathlib import Path
import random
import struct
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from furniture_plan import FurniturePlan, Step, canonical
import route_furniture_allocation as routes
import route_furniture_sequence as sequence
from route_furniture_suffix import assess_remaining


def capture(shapes, *, size=(7, 4, 1), tick=100, hidden=(), paused=True):
    """Encode unchanged DFMM1500 bytes, without using its decoder as an oracle."""
    raw = bytearray(b'DFMM1500' + struct.pack('>IIBI', 0, tick, paused, 1))
    raw += struct.pack('>H', 4) + b'fort'
    raw += struct.pack('>9I', 128, 128, 8, 0, 0, 0, *size)
    raw += struct.pack('>I', size[0] * size[1] * size[2])
    for z in range(size[2]):
        for y in range(size[1]):
            for x in range(size[0]):
                point = (x, y, z)
                if point in hidden:
                    raw += b'\1'
                else:
                    shape = shapes.get(point, 2)
                    raw += b'\2' + struct.pack('>IBBBBBBBIHH',
                        10, shape, 0, 0, 0, 0, 0, 0, int(shape in (3, 6, 7, 8)), 10000, 10000)
    return bytes(raw)


def corridor(detour=False):
    shapes = {(x, 1, 0): 3 for x in range(1, 6)}
    if detour:
        shapes.update({(x, 2, 0): 3 for x in range(1, 6)})
    plan = FurniturePlan((Step('near', 'bed', 11, (2, 1, 0)),
                          Step('far', 'chair', 12, (4, 1, 0), ('near',))))
    return plan, {11: (1, 1, 0), 12: (1, 1, 0)}, shapes


def oracle(shapes, source, target, blocked, limit):
    """Independent coordinate BFS including the explicit vertical stair rule."""
    allowed = {point for point, shape in shapes.items() if shape in (3, 6, 7, 8)} - blocked
    if source not in allowed or target not in allowed:
        return None
    queue, seen = deque([(source, 0)]), {source}
    while queue:
        point, length = queue.popleft()
        if point == target:
            return length
        if length >= limit:
            continue
        for axis in range(3):
            for delta in (-1, 1):
                neighbor = list(point)
                neighbor[axis] += delta
                neighbor = tuple(neighbor)
                if neighbor not in allowed or neighbor in seen:
                    continue
                if axis == 2:
                    low, high = (point, neighbor) if delta > 0 else (neighbor, point)
                    if shapes[low] not in (6, 8) or shapes[high] not in (7, 8):
                        continue
                seen.add(neighbor)
                queue.append((neighbor, length + 1))
    return None


class SuffixTests(unittest.TestCase):
    def test_individually_reachable_is_not_a_deliverable_fixed_order(self):
        plan, positions, shapes = corridor()
        self.assertTrue(all(oracle(shapes, positions[s.item], s.target, set(), 20) is not None
                            for s in plan.ordered))
        result = assess_remaining(plan, (), positions, capture(shapes))
        self.assertEqual(result['status'], 'blocked')
        self.assertEqual(result['first_blocked_slot'], 'far')
        self.assertFalse(result['candidate_model_sequence_proven'])
        self.assertEqual(result['routes'], [])
        self.assertIsNone(result['total_delivery_route_steps'])
        self.assertEqual(result['remaining_order'], ['near', 'far'])

    def test_safe_original_order_preserved_without_optimizer_or_input_mutation(self):
        plan, positions, shapes = corridor()
        plan = FurniturePlan(tuple(replace(s, after=()) for s in plan.steps))
        original = (canonical(plan.json()), dict(positions))
        with patch.object(sequence, '_reverse_order', side_effect=AssertionError('must not reorder')), \
             patch.object(routes, 'allocate_routes', side_effect=AssertionError('must not reallocate')):
            result = assess_remaining(plan, (), positions, capture(shapes))
        self.assertEqual(result['status'], 'ready')
        self.assertEqual(result['remaining_order'], ['far', 'near'])
        self.assertEqual([r['route_steps'] for r in result['routes']], [3, 1])
        self.assertEqual([r['blocked_target_count'] for r in result['routes']], [0, 1])
        self.assertEqual(original, (canonical(plan.json()), positions))
        for flag in ('reordering_performed', 'reallocation_performed', 'prefix_receipts_authenticated',
                     'item_selectors_verified', 'placement_authorized', 'game_effect_performed',
                     'native_pathfinding_proven', 'future_routes_guaranteed'):
            self.assertIs(result[flag], False, flag)

    def test_placed_prefix_removes_targets_even_when_fresh_capture_is_open(self):
        plan, positions, shapes = corridor()
        result = assess_remaining(plan, ('near',), {12: positions[12]}, capture(shapes))
        self.assertEqual(result['status'], 'blocked')
        self.assertEqual(result['first_blocked_slot'], 'far')
        self.assertEqual(result['placed_prefix'], ['near'])
        self.assertEqual(result['remaining_order'], ['far'])

    def test_current_sources_and_post_placement_detour_obey_exact_original_limit(self):
        plan, positions, shapes = corridor(True)
        self.assertEqual(oracle(shapes, positions[12], (4, 1, 0), {(2, 1, 0)}, 20), 5)
        blocked = assess_remaining(plan, (), positions, capture(shapes), max_route_steps=4)
        self.assertEqual(blocked['status'], 'blocked')
        success = assess_remaining(plan, (), positions, capture(shapes), max_route_steps=5)
        self.assertEqual([r['route_steps'] for r in success['routes']], [1, 5])
        # Never reuse the item position in an earlier allocation/review.
        moved = {**positions, 12: (3, 1, 0)}
        self.assertEqual(assess_remaining(plan, (), moved, capture(shapes), max_route_steps=1)['status'], 'ready')

    def test_unhauled_source_covered_by_earlier_target_is_blocked(self):
        plan, positions, shapes = corridor(True)
        positions[12] = (2, 1, 0)
        result = assess_remaining(plan, (), positions, capture(shapes))
        self.assertEqual((result['status'], result['first_blocked_slot']), ('blocked', 'far'))
        self.assertEqual(result['routes'], [])

    def test_unknown_endpoints_never_become_absence_or_partial_success(self):
        plan, positions, shapes = corridor(True)
        for point in ((1, 1, 0), (4, 1, 0)):
            with self.subTest(point=point):
                result = assess_remaining(plan, (), positions, capture(shapes, hidden=(point,)))
                self.assertEqual(result['status'], 'unestablished')
                self.assertTrue(result['unestablished_endpoints'])
                self.assertIsNone(result['first_blocked_slot'])
                self.assertEqual(result['routes'], [])
        result = assess_remaining(plan, (), {11: positions[11], 12: (30, 30, 0)}, capture(shapes))
        self.assertEqual(result['unestablished_endpoints'][0]['reason'], 'outside_observed_region')

    def test_no_skipped_foreign_reordered_or_complete_prefix(self):
        plan, positions, shapes = corridor(True)
        for prefix in (['near'], ('far',), ('near', 'near'), ('near', 'far'), ('foreign',), (True,)):
            with self.subTest(prefix=prefix), self.assertRaises(ValueError):
                assess_remaining(plan, prefix, positions, capture(shapes))
        for invalid in ({11: positions[11]}, {**positions, 13: (1, 1, 0)}, {11: positions[11], 13: positions[12]}):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                assess_remaining(plan, (), invalid, capture(shapes))
        with self.assertRaises(ValueError):
            assess_remaining(plan, ('near',), positions, capture(shapes))

    def test_strict_types_and_forged_plan_cache_are_refused(self):
        plan, positions, shapes = corridor(True)
        for bad in ((True, 1, 0), [1, 1, 0], (1, 1), (-1, 1, 0), (32768, 1, 0)):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                assess_remaining(plan, (), {**positions, 12: bad}, capture(shapes))
        for bad in (True, -1, routes.MAX_ROUTE_STEPS + 1):
            with self.assertRaises(ValueError):
                assess_remaining(plan, (), positions, capture(shapes), max_route_steps=bad)
        object.__setattr__(plan, 'ordered', tuple(reversed(plan.ordered)))
        with self.assertRaises(ValueError):
            assess_remaining(plan, (), positions, capture(shapes))

    def test_cross_level_compatible_stairs_only(self):
        shapes = {(1, 1, 0): 3, (2, 1, 0): 6, (2, 1, 1): 7, (3, 1, 1): 3}
        plan = FurniturePlan((Step('up', 'bed', 11, (3, 1, 1)),))
        result = assess_remaining(plan, (), {11: (1, 1, 0)}, capture(shapes, size=(5, 3, 2)))
        self.assertEqual(result['routes'][0]['route_steps'], 3)
        for lower, upper in ((7, 6), (3, 7), (6, 3)):
            bad = {**shapes, (2, 1, 0): lower, (2, 1, 1): upper}
            self.assertEqual(assess_remaining(plan, (), {11: (1, 1, 0)},
                capture(bad, size=(5, 3, 2)))['status'], 'blocked')

    def test_maximum_plan_and_every_valid_suffix(self):
        steps = tuple(Step(f's{i:02}', 'bed', i, (2, i + 1, 0)) for i in range(32))
        plan = FurniturePlan(steps)
        shapes = {(x, y, 0): 3 for x in (1, 2) for y in range(1, 33)}
        raw = capture(shapes, size=(4, 34, 1))
        for count in range(32):
            prefix = tuple(s.name for s in plan.ordered[:count])
            positions = {s.item: (1, s.target[1], 0) for s in plan.ordered[count:]}
            result = assess_remaining(plan, prefix, positions, raw, max_route_steps=1)
            self.assertEqual(result['status'], 'ready')
            self.assertEqual(len(result['routes']), 32 - count)
            self.assertEqual(result['routes'][0]['blocked_target_count'], count)
            self.assertLessEqual(len(canonical(result)), routes.MAX_OUTPUT)

    def test_budget_cancellation_and_final_output_check(self):
        plan, positions, shapes = corridor(True)
        raw = capture(shapes)
        calls = []
        good = assess_remaining(plan, (), positions, raw, lambda: calls.append(None))
        exact = assess_remaining(plan, (), positions, raw, maximum_work=good['work_units'])
        self.assertEqual(exact['work_units'], good['work_units'])
        self.assertEqual(exact['evidence_digest'], good['evidence_digest'])
        for maximum in (0, 1, good['work_units'] - 1):
            with self.assertRaises(ValueError):
                assess_remaining(plan, (), positions, raw, maximum_work=maximum)
        for fail_at in range(1, len(calls) + 1):
            seen = 0
            def guard():
                nonlocal seen
                seen += 1
                if seen == fail_at:
                    raise ValueError('cancelled')
            with self.subTest(fail_at=fail_at), self.assertRaisesRegex(ValueError, 'cancelled'):
                assess_remaining(plan, (), positions, raw, guard)
        with patch.object(routes, 'MAX_OUTPUT', 1), self.assertRaises(ValueError):
            assess_remaining(plan, (), positions, raw)

    def test_evidence_binds_inputs_and_ignores_only_work_allowance(self):
        plan, positions, shapes = corridor(True)
        base = assess_remaining(plan, (), positions, capture(shapes))
        unsigned = {k: v for k, v in base.items() if k not in ('work_units', 'maximum_work', 'evidence_digest')}
        self.assertEqual(base['evidence_digest'], hashlib.sha256(
            b'dfmcp-furniture-delivery-suffix/1\0' + canonical(unsigned)).hexdigest())
        variants = [assess_remaining(plan, (), positions, capture(shapes, tick=101)),
                    assess_remaining(plan, (), {**positions, 12: (3, 1, 0)}, capture(shapes)),
                    assess_remaining(plan, ('near',), {12: positions[12]}, capture(shapes)),
                    assess_remaining(plan, (), positions, capture(shapes), max_route_steps=20),
                    assess_remaining(FurniturePlan(tuple(replace(s, after=()) for s in plan.steps)),
                                     (), positions, capture(shapes))]
        self.assertTrue(all(v['evidence_digest'] != base['evidence_digest'] for v in variants))
        shuffled = dict(reversed(list(positions.items())))
        self.assertEqual(assess_remaining(plan, (), shuffled, capture(shapes)), base)

    def test_truncated_and_unpaused_captures_refuse_without_diagnostics(self):
        plan, positions, shapes = corridor()
        raw = capture(shapes)
        for count in range(len(raw)):
            with self.assertRaises(ValueError):
                assess_remaining(plan, (), positions, raw[:count])
        with self.assertRaises(ValueError):
            assess_remaining(plan, (), positions, raw + b'\0')
        with self.assertRaises(ValueError):
            assess_remaining(plan, (), positions, capture(shapes, paused=False))

    def test_seeded_independent_fixed_order_oracle(self):
        rng = random.Random(176239)
        cells = [(x, y, 0) for x in range(1, 5) for y in range(1, 5)]
        for case in range(600):
            count = rng.randint(1, 5)
            targets = rng.sample(cells, count)
            steps = tuple(Step(f's{i}', 'bed', 100 + i, target,
                tuple(f's{j}' for j in range(i) if rng.random() < .3)) for i, target in enumerate(targets))
            plan = FurniturePlan(steps)
            prefix_length = rng.randrange(count)
            prefix = tuple(s.name for s in plan.ordered[:prefix_length])
            active = plan.ordered[prefix_length:]
            positions = {s.item: rng.choice(cells) for s in active}
            shapes = {p: 3 for p in cells if rng.random() < .6}
            shapes.update({p: 3 for p in targets + list(positions.values())})
            limit = rng.randrange(12)
            blocked, lengths, failed = set(targets[:prefix_length]), [], None
            for step in active:
                length = oracle(shapes, positions[step.item], step.target, blocked, limit)
                if length is None:
                    failed = step.name
                    break
                lengths.append(length)
                blocked.add(step.target)
            result = assess_remaining(plan, prefix, positions,
                capture(shapes, size=(6, 6, 1)), max_route_steps=limit)
            with self.subTest(case=case):
                self.assertEqual(result['status'], 'ready' if failed is None else 'blocked')
                self.assertEqual(result['first_blocked_slot'], failed)
                self.assertEqual([r['route_steps'] for r in result['routes']], lengths if failed is None else [])


if __name__ == '__main__':
    unittest.main()
