"""Real compilation, search, terrain reduction and allocation; no effect doubles."""
from __future__ import annotations

from copy import deepcopy
from itertools import product
from pathlib import Path
import random
import struct
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import excavation_observer as native
import room_layout as layout
import room_terrain as terrain
from furniture_allocation import Candidate, allocate
from furniture_plan import canonical
from room_layout_costs import TileCosts
from room_provisioning import INTENT_SCHEMA, RoomPlan
from room_residual_capacity import STRATEGY, assess


def request(*, restricted=True, small=False):
    width, height, count = (6, 3, 1) if small else (28, 9, 16)
    value = {'schema': layout.REQUEST_SCHEMA, 'world_folder': 'capacity-fixture', 'site': 7,
             'region': {'origin': [10, 10, 2], 'size': [width, height, 1]},
             'objective': layout.COST_OBJECTIVE,
             'excavation_costs': ''.join('1' if x >= 23 or (x+y) % 2 == 0 else '0'
                                        for y in range(10, 10+height) for x in range(10, 10+width)),
             'areas': [{'name': 'hall', 'template': {'kind': 'dining_hall',
                        'table_count': count, 'columns': 4}}]}
    if restricted:
        value['residual_partition'] = STRATEGY
    return value


def solve(value):
    return layout.solve(canonical(value))


def points(parts):
    result = []
    for part in parts:
        x, y, z = part['region']['origin']
        w, h, d = part['region']['size']
        result.extend(product(range(x, x+w), range(y, y+h), range(z, z+d)))
    return result


def original_mask(plan):
    return set(points(plan.json()['excavation_blueprint']['parts']))


def cost_at(value, p):
    x, y, z = p
    ox, oy, oz = value['region']['origin']
    width, height, _ = value['region']['size']
    if z != oz or not (ox <= x < ox+width and oy <= y < oy+height):
        return 1
    return int(value['excavation_costs'][(y-oy)*width+x-ox])


def capture(value, region):
    """Synthetic map bytes; only declared region cost-zero cells are floors."""
    folder = value['world_folder'].encode()
    raw = bytearray(b'DFMM1500' + struct.pack('>IIBIH', 0, 100, 1, value['site'], len(folder)) + folder)
    raw += struct.pack('>10I', 128, 128, 8, *region.origin, *region.size, region.volume)
    for p in terrain.points(region, lambda: None):
        shape = 2 if cost_at(value, p) else 3
        raw += b'\2' + struct.pack('>IBBBBBBBIHH', 1, shape, 0, 0, 0, 0, 0, 0, 1, 10000, 10000)
    return bytes(raw)


def independent_cover(mask, span):
    """Independent ordered-loop oracle for the unchanged width-first policy."""
    remaining, rectangles = set(mask), []
    while remaining:
        z, y, x = sorted((z, y, x) for x, y, z in remaining)[0]
        width = 0
        for xx in range(x, x+span):
            if (xx, y, z) not in remaining:
                break
            width += 1
        height = 0
        for yy in range(y, y+span):
            row = {(xx, yy, z) for xx in range(x, x+width)}
            if not row.issubset(remaining):
                break
            height += 1
        cells = {(xx, yy, z) for xx in range(x, x+width) for yy in range(y, y+height)}
        remaining -= cells
        rectangles.append(cells)
    return rectangles


class ResidualCapacityTests(unittest.TestCase):
    def test_reproduces_and_escapes_cheapest_unusable_32_item_layout(self):
        old = solve(request(restricted=False))
        old_plan = old.room_plan()
        selected = terrain.selection(old_plan)
        report = terrain.survey(old_plan, capture(request(), selected.region), native.Manifest(1, 'fixture', 'fixture'))
        self.assertEqual(report['status'], 'blocked')
        self.assertEqual(report['partition']['blueprint_parts'], 59)
        self.assertEqual(old.json()['optimization']['score']['excavation_cost'], 59)
        new = solve(request())
        plan = new.room_plan()
        self.assertEqual(plan.json()['intent']['areas'][0]['origin'], [17, 10, 2])
        self.assertEqual(plan.json()['furniture_count'], 32)
        self.assertEqual(plan.json()['target_tiles'], 117)
        self.assertEqual(new.json()['residual_partition']['capacity_refusals'], 7)
        self.assertEqual(new.json()['residual_partition']['selected']['blueprint_parts'], 28)
        self.assertEqual(new.json()['optimization']['score']['excavation_cost'], 90)
        self.assertTrue(new.json()['optimization']['minimum_in_compiler_domain_proven'])
        self.assertIn('complete_residual_partition', new.json()['optimization']['scope'])
        self.assertEqual(layout.Layout.decode(new.encode()), new)

    def test_independent_enumeration_matches_selected_feasible_optimum(self):
        value = request()
        expected = []
        for x in range(10, 26):
            mask = {(xx, y, 2) for xx in range(x, x+13) for y in range(10, 19)}
            remaining = {p for p in mask if cost_at(value, p)}
            if len(independent_cover(remaining, 128)) <= 32 and len(independent_cover(remaining, 8)) <= 128:
                expected.append((len(remaining), 117, x-10, ((10, x, 2, 0),)))
        body = solve(value).json()
        score = body['optimization']['score']
        actual = (score['excavation_cost'], score['target_tiles'], score['total_entry_distance'],
                  tuple(map(tuple, score['placements_yxz_rotation'])))
        self.assertEqual(actual, min(expected))

    def test_random_cost_worlds_match_independent_full_origin_enumeration(self):
        rng = random.Random(98121)
        for case in range(24):
            value = request()
            value['excavation_costs'] = ''.join(rng.choice('0011') for _ in range(28*9))
            expected = []
            for x in range(10, 26):
                remaining = {(xx, y, 2) for xx in range(x, x+13) for y in range(10, 19)
                             if cost_at(value, (xx, y, 2))}
                if len(independent_cover(remaining, 128)) <= 32 and len(independent_cover(remaining, 8)) <= 128:
                    expected.append((len(remaining), x-10, x))
            out = solve(value).json()
            with self.subTest(case=case):
                self.assertEqual(out['status'] == 'layout_found', bool(expected))
                if expected:
                    best = min(expected)
                    self.assertEqual(out['optimization']['score']['excavation_cost'], best[0])
                    self.assertEqual(out['room_plan']['intent']['areas'][0]['origin'][0], best[2])
                else:
                    self.assertIsNone(out['room_plan'])

    def test_selected_partition_equals_actual_raw_terrain_reduction(self):
        value = request()
        result = solve(value)
        plan = result.room_plan()
        selected = terrain.selection(plan)
        report = terrain.survey(plan, capture(value, selected.region), native.Manifest(1, 'fixture', 'fixture'))
        capacity = result.json()['residual_partition']['selected']
        self.assertEqual(report['status'], 'excavation_proposed')
        self.assertEqual(report['remaining_mask_digest'], capacity['remaining_mask_digest'])
        self.assertEqual(report['partition']['blueprint_parts'], capacity['blueprint_parts'])
        self.assertEqual(report['partition']['normal_mining_rectangles'], capacity['normal_mining_rectangles'])
        self.assertEqual(report['target_counts']['remaining_wall'], capacity['remaining_tiles'])
        self.assertEqual(set(points(report['remaining_blueprint']['parts'])),
                         {p for p in original_mask(plan) if cost_at(value, p)})

    def test_fixed_bad_origin_cannot_be_moved_or_silently_partially_mined(self):
        value = request()
        value['areas'][0]['origin'] = [10, 10, 2]
        body = solve(value).json()
        self.assertEqual(body['status'], 'no_layout_in_domain')
        self.assertEqual(body['residual_partition']['capacity_refusals'], 1)
        self.assertIsNone(body['room_plan'])
        self.assertIsNone(body['residual_partition']['selected'])
        self.assertTrue(body['search']['domain_exhausted'])
        self.assertFalse(body['partial_plan_returned'])

    def test_node_limit_never_returns_a_capacity_rejected_incumbent(self):
        for limit in (1, 2, 8):
            result = solve({**request(), 'search': {'max_nodes': limit}}).json()
            self.assertEqual(result['status'], 'search_limit_reached')
            self.assertIsNone(result['room_plan'])
        result = solve({**request(), 'search': {'max_nodes': 9}}).json()
        self.assertEqual(result['status'], 'layout_found')
        self.assertTrue(result['residual_partition']['selected']['within_profile_limits'])
        self.assertFalse(result['optimization']['minimum_in_compiler_domain_proven'])
        self.assertFalse(result['search']['domain_exhausted'])

    def test_work_budget_is_shared_through_final_partition_assessment(self):
        value = request(small=True)
        used = solve(value).json()['search']['work_units']
        for maximum in (1, used//2, used-1):
            with self.assertRaises(layout.WorkLimit):
                solve({**value, 'search': {'max_work': maximum}})
        exact = solve({**value, 'search': {'max_work': used}})
        self.assertEqual(exact.json()['search']['work_units'], used)

    def test_every_small_guard_checkpoint_propagates_external_cancellation(self):
        value = request(small=True)
        value['areas'][0]['origin'] = [10, 10, 2]
        raw = canonical(value)
        calls = []
        layout.solve(raw, lambda: calls.append(1))
        for stop in range(1, len(calls)+1):
            seen = 0
            def guard():
                nonlocal seen
                seen += 1
                if seen == stop:
                    raise ValueError('external cancellation')
            with self.subTest(stop=stop), self.assertRaisesRegex(ValueError, 'external cancellation'):
                layout.solve(raw, guard)

    def test_strategy_is_closed_and_requires_binary_cost_semantics(self):
        for strategy in (None, True, 1, [], {}, 'other'):
            value = request(small=True)
            value['residual_partition'] = strategy
            with self.subTest(strategy=strategy), self.assertRaises(ValueError):
                solve(value)
        for objective in (None, layout.MINIMUM_TARGETS_OBJECTIVE):
            value = request(small=True)
            value.pop('excavation_costs')
            if objective is None:
                value.pop('objective')
            else:
                value['objective'] = objective
            with self.assertRaises(ValueError):
                solve(value)

    def test_zero_residual_is_complete_no_work_not_a_replacement_effect(self):
        value = request(small=True)
        value['excavation_costs'] = '0'*18
        result = solve(value)
        capacity = result.json()['residual_partition']['selected']
        self.assertEqual(capacity['remaining_tiles'], 0)
        self.assertEqual(capacity['blueprint_parts'], 0)
        self.assertIsNone(capacity['remaining_mask_digest'])
        self.assertTrue(capacity['empty_residual'])
        self.assertTrue(capacity['within_profile_limits'])

    def test_all_original_furniture_constraints_and_shortage_are_preserved(self):
        value = request()
        value['excluded_items'] = [999]
        value['areas'][0]['item_constraints'] = {'chair': {'material': [7, 8], 'subtype': 2, 'max_distance': 40}}
        before = deepcopy(value)
        plan = solve(value).room_plan()
        wanted = plan.request()
        candidates = tuple(Candidate(i, slot.kind, slot.target, slot.material or (0, 1),
                                     -1 if slot.subtype is None else slot.subtype)
                           for i, slot in enumerate(wanted.slots))
        result = allocate(wanted, candidates)
        self.assertEqual(result['status'], 'allocated')
        self.assertEqual(len(result['plan']['steps']), 32)
        self.assertEqual(value, before)
        self.assertEqual(wanted.excluded_items, (999,))
        shortage = allocate(wanted, candidates[:-1])
        self.assertEqual(shortage['status'], 'shortage')
        self.assertIsNone(shortage['plan'])
        self.assertEqual(shortage['assignments'], [])

    def test_rehashed_changed_strategy_counts_and_costs_are_rejected(self):
        result = solve(request())
        for mode in ('strategy', 'count', 'cost'):
            body = result.json()
            body.pop('layout_digest')
            if mode == 'strategy':
                body['request'].pop('residual_partition')
            elif mode == 'count':
                body['residual_partition']['selected']['blueprint_parts'] = 1
            else:
                body['request']['excavation_costs'] = '0'*252
            with self.subTest(mode=mode), self.assertRaises(ValueError):
                layout.Layout.decode(layout.Layout(canonical(body)).encode())

    def test_shared_cover_is_exact_for_all_512_small_residual_masks(self):
        value = request(small=True)
        plan = solve(value).room_plan()
        body = plan.json()
        domain = sorted(original_mask(plan), key=lambda p: p[::-1])[:9]
        for bits in range(512):
            mask = frozenset(p for index, p in enumerate(domain) if bits & (1 << index))
            parts, native_parts = terrain.residual_parts(body, mask)
            self.assertEqual(set(points(parts)), set(mask))
            self.assertEqual(len(points(parts)), len(mask))
            self.assertEqual(set(points(native_parts)), set(mask))
            self.assertEqual(len(parts), len(independent_cover(mask, 128)))
            self.assertEqual(len(native_parts), len(independent_cover(mask, 8)))

    def test_cost_aware_corridors_are_included_in_full_residual_count(self):
        value = request(small=True)
        value['region']['size'] = [15, 6, 1]
        value['areas'] = [
            {'name': name, 'origin': [x, 11, 2], 'template': {'kind': 'dining_hall', 'table_count': 1}}
            for name, x in (('a', 11), ('b', 21))]
        value['excavation_costs'] = ''.join('0' if y == 10 else '1' for y in range(10, 16) for x in range(10, 25))
        value['connection_strategy'] = layout.COST_CONNECTION_STRATEGY
        result = solve(value)
        plan = result.room_plan()
        counted = sum(cost_at(value, p) for p in original_mask(plan))
        self.assertEqual(result.json()['residual_partition']['selected']['remaining_tiles'], counted)
        self.assertEqual(result.json()['optimization']['score']['excavation_cost'], counted)
        self.assertEqual(len(plan.request().slots), 4)

    def test_capacity_result_does_not_claim_observation_or_permission(self):
        out = solve(request(small=True)).json()
        self.assertFalse(out['residual_partition']['native_excavation_eligibility_proven'])
        self.assertEqual(out['residual_partition']['cost_evidence'], 'caller_declared_not_observed')
        for flag in ('terrain_observed', 'native_excavation_eligibility_proven', 'mutation_authority_granted'):
            self.assertFalse(out['residual_partition']['selected'][flag])


if __name__ == '__main__':
    unittest.main()
