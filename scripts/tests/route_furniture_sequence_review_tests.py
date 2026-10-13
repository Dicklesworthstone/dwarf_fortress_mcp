"""Sequenced requests through actual retained-review and prefix reducers.

All captures, enum identities, source manifests and progress receipts are
synthetic. No native acquisition, private custody or placement is exercised.
"""
from __future__ import annotations

from dataclasses import replace
import hashlib
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from furniture_allocation import Candidate, Request, Slot
from furniture_plan import FurniturePlan, canonical, progress
import route_furniture_allocation as routes
from route_furniture_review import Review, Source, create, object_digest, revalidate
from route_furniture_sequence import allocate_sequenced_routes
from route_furniture_sequence_tests import capture, corridor


def source_for(raw: bytes) -> Source:
    terrain = routes.decode_terrain(raw)
    return Source('127.0.0.1:5000', 'synthetic-df', 'synthetic-dfhack', 11, 22,
                  terrain.digest, 'ab' * 32, terrain.tick, terrain.dimensions,
                  terrain.origin, terrain.size, (100, 100, 1000))


class SequenceReviewTests(unittest.TestCase):
    def setUp(self):
        self.original, self.items, self.raw = corridor()
        self.effective, self.allocation = allocate_sequenced_routes(self.original, self.items, self.raw)
        self.source = source_for(self.raw)
        self.types = {item.id: 1 for item in self.items}
        self.review = create(self.effective, self.allocation, self.source, self.types)

    def test_effective_request_and_exact_order_survive_review_roundtrip(self):
        decoded = Review.decode(self.review.raw, self.review.digest)
        value, request, source, plan = decoded.unpack()
        self.assertEqual(request, self.effective)
        self.assertEqual(source, self.source)
        self.assertEqual(plan.json(), self.allocation['plan'])
        self.assertEqual([step.name for step in plan.ordered], ['b_far', 'a_near'])
        self.assertEqual(value['allocation_digest'], self.allocation['decision_digest'])
        self.assertEqual(value['plan_digest'], self.allocation['plan_digest'])
        self.assertEqual([row['candidate']['item'] for row in value['selections']], [10, 20])

    def test_stale_original_request_cannot_be_paired_with_compiled_allocation(self):
        with self.assertRaisesRegex(ValueError, 'original route allocation'):
            create(self.original, self.allocation, self.source, self.types)

    def test_revalidation_preserves_order_without_granting_placement_authority(self):
        result = revalidate(self.review, self.items, self.types, self.raw, self.source)
        self.assertEqual(result['status'], 'constraints_met')
        self.assertEqual(result['plan'], self.allocation['plan'])
        self.assertEqual(result['original_request_digest'], self.effective.digest)
        self.assertEqual(result['original_allocation_digest'], self.allocation['decision_digest'])
        self.assertTrue(result['all_original_selections_checked'])
        for field in ('reallocation_performed', 'constraints_relaxed', 'placement_authorized',
                      'native_pathfinding_proven', 'future_routes_preserved', 'items_reserved',
                      'game_effect_performed', 'production_admitted'):
            self.assertFalse(result[field], field)
        plan = FurniturePlan.from_json(result['plan'])
        self.assertEqual(progress(plan, {})['next_step'], 'b_far')
        self.assertEqual(progress(plan, {'b_far': 'placed'})['next_step'], 'a_near')
        with self.assertRaises(ValueError):
            progress(plan, {'a_near': 'placed'})
        pending = progress(plan, {'b_far': 'indeterminate'})
        self.assertEqual(pending['status'], 'pending_recovery')
        self.assertIsNone(pending['next_step'])
        self.assertFalse(pending['retry_permitted'])

    def test_compiled_review_never_substitutes_a_missing_original_item(self):
        changed = (self.items[0], replace(self.items[1], id=21))
        result = revalidate(self.review, changed, {10: 1, 21: 1}, self.raw, self.source)
        self.assertEqual(result['status'], 'unestablished')
        self.assertIsNone(result['plan'])
        self.assertEqual(result['rows'][1]['item'], 20)
        self.assertEqual(result['rows'][1]['reason'], 'selected_item_missing_or_ineligible')
        self.assertFalse(result['reallocation_performed'])

    def test_changed_material_subtype_and_native_identity_remain_refusals(self):
        for material, subtype, native in (((0, 9), 2, 1), ((0, 2), 9, 1), ((0, 2), 2, 9)):
            with self.subTest(material=material, subtype=subtype, native=native):
                changed = (self.items[0], replace(self.items[1], material=material, subtype=subtype))
                # Both candidates are beds; a changed enum must be consistent
                # within the new projection before original identity is checked.
                types = {10: native, 20: native}
                result = revalidate(self.review, changed, types, self.raw, self.source)
                self.assertEqual(result['status'], 'constraints_not_met')
                self.assertIsNone(result['plan'])
                self.assertEqual(result['rows'][1]['reason'], 'selected_item_identity_changed')

    def test_a_fresh_obstruction_cannot_reuse_historical_sequence_success(self):
        current = capture((4, 1, 1), [3, 3, 3, 3], excluded={2: (0, 1, 0, 1)})
        result = revalidate(self.review, self.items, self.types, current, source_for(current))
        self.assertEqual(result['status'], 'constraints_not_met')
        self.assertIsNone(result['plan'])
        self.assertEqual(result['rows'][1]['reason'], 'no_route_in_observed_model')
        self.assertFalse(result['future_routes_preserved'])

    def test_source_incarnation_region_or_clock_regression_is_not_rebound(self):
        for source in (replace(self.source, map_generation=12),
                       replace(self.source, inventory_generation=23),
                       replace(self.source, endpoint='127.0.0.1:5001'),
                       replace(self.source, df_version='changed'),
                       replace(self.source, origin=(2, 1, 0)),
                       replace(self.source, tick=self.source.tick - 1)):
            with self.subTest(source=source), self.assertRaises(ValueError):
                revalidate(self.review, self.items, self.types, self.raw, source)

    def test_deleting_compiled_dependency_is_rejected_even_with_rehashed_review(self):
        value = self.review.json()
        value['request']['slots'][0]['after'] = []
        changed = canonical(value)
        with self.assertRaises(ValueError):
            Review.decode(changed, self.review.digest)
        with self.assertRaisesRegex(ValueError, 'changed original plan'):
            Review.decode(changed, object_digest(changed))

    def test_tampered_sequence_witness_cannot_enter_the_retained_review(self):
        self.allocation['delivery_sequence']['routes'][0]['route_steps'] += 1
        with self.assertRaisesRegex(ValueError, 'decision digest differs'):
            create(self.effective, self.allocation, self.source, self.types)

    def test_blocked_and_shortage_allocations_cannot_create_executable_reviews(self):
        original, items, raw = corridor(after=('a_near',))
        effective, blocked = allocate_sequenced_routes(original, items, raw)
        with self.assertRaisesRegex(ValueError, 'original route allocation'):
            create(effective, blocked, source_for(raw), self.types)
        effective, shortage = allocate_sequenced_routes(self.original, self.items[:1], self.raw)
        with self.assertRaises(ValueError):
            create(effective, shortage, self.source, self.types)

    def test_cross_level_reordered_plan_retains_selectors_through_review(self):
        request = Request('fortress', 1, (
            Slot('a', 'bed', (2, 1, 0), subtype=1),
            Slot('b', 'bed', (2, 1, 1), subtype=2, max_distance=4, max_vertical_distance=1)))
        items = (Candidate(10, 'bed', (1, 1, 0), (0, 0), 1),
                 Candidate(20, 'bed', (1, 1, 0), (0, 0), 2))
        raw = capture((3, 1, 2), [3, 3, 6, 0, 3, 7])
        effective, result = allocate_sequenced_routes(request, items, raw)
        review = create(effective, result, source_for(raw), self.types)
        saved = review.unpack()[1]
        self.assertEqual(saved.slots[1].max_vertical_distance, 1)
        self.assertEqual(saved.slots[1].max_distance, 4)
        checked = revalidate(review, items, self.types, raw, source_for(raw))
        self.assertEqual(checked['status'], 'constraints_met')
        self.assertEqual([s.name for s in FurniturePlan.from_json(checked['plan']).ordered], ['b', 'a'])

    def test_largest_32_step_sequence_fits_the_existing_closed_review(self):
        request = Request('fortress', 1, tuple(
            Slot(f's{i:02}', 'bed', (i + 2, 1, 0), subtype=i) for i in range(32)))
        items = tuple(Candidate(i + 1, 'bed', (1, 1, 0), (0, 0), i) for i in range(32))
        raw = capture((33, 1, 1), [3] * 33)
        effective, result = allocate_sequenced_routes(request, items, raw)
        types = {item.id: 1 for item in items}
        review = create(effective, result, source_for(raw), types)
        self.assertEqual(len(review.unpack()[3].steps), 32)
        checked = revalidate(review, items, types, raw, source_for(raw))
        self.assertEqual(checked['status'], 'constraints_met')
        self.assertEqual(checked['counts']['constraints_met'], 32)
        unsigned = {k: v for k, v in checked.items() if k != 'decision_digest'}
        self.assertEqual(checked['decision_digest'], hashlib.sha256(
            b'dfmcp-route-furniture-revalidation/1\0' + canonical(unsigned)).hexdigest())


if __name__ == '__main__':
    unittest.main()
