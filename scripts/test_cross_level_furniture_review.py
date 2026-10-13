"""Actual immutable-review lifecycle over explicit multi-floor map/inventory data.

No native transport, source authority issuer, private batch store or game is mocked
into a success claim: this suite executes the pure retention/revalidation boundary.
"""
from dataclasses import replace
import hashlib
import unittest

from furniture_allocation import Request, Slot
from furniture_plan import canonical
import route_furniture_allocation as routes
import route_furniture_review as reviews
from test_cross_level_furniture import terrain, item, request


def source(raw, *, inventory='inventory', **changes):
    t = routes.decode_terrain(raw)
    value = reviews.Source('127.0.0.1:5000', 'fixture-df', 'fixture-dfhack', 7, 11,
                           t.digest, hashlib.sha256(inventory.encode()).hexdigest(), t.tick,
                           t.dimensions, t.origin, t.size, (30, 20, 100))
    return replace(value, **changes)


def reviewed(r=None, candidates=None, raw=None, route_limit=16383):
    r = request(2) if r is None else r
    candidates = (item(),) if candidates is None else candidates
    raw = terrain()[0] if raw is None else raw
    allocation = routes.allocate_routes(r, candidates, raw, max_route_steps=route_limit)
    kinds = {'bed': 1, 'chair': 2, 'table': 3}
    types = {i.id: kinds[i.kind] for i in candidates}
    selected = {row['item']: types[row['item']] for row in allocation['assignments']}
    review = reviews.create(r, allocation, source(raw), selected)
    return review, allocation, candidates, types, raw


class CrossLevelReviewTests(unittest.TestCase):
    def test_roundtrip_retains_every_original_vertical_constraint(self):
        r = Request('fort', 1, (Slot('a', 'bed', (4, 2, 2), max_vertical_distance=2),
                                Slot('b', 'chair', (5, 2, 1), after=('a',), max_vertical_distance=1)))
        review, out, candidates, types, raw = reviewed(r, (item(1), item(2, kind='chair')))
        old = review.raw
        reopened = reviews.Review.decode(review.raw, review.digest)
        value, request_back, _, plan = reopened.unpack()
        self.assertEqual(request_back, r)
        self.assertEqual([s.max_vertical_distance for s in request_back.slots], [2, 1])
        self.assertEqual(plan.json(), out['plan'])
        self.assertEqual([s.name for s in plan.ordered], ['a', 'b'])
        result = reviews.revalidate(reopened, candidates, types, raw, source(raw))
        self.assertEqual(result['status'], 'constraints_met')
        self.assertEqual(result['original_plan'], plan.json())
        self.assertTrue(result['all_original_selections_checked'])
        self.assertFalse(result['reallocation_performed'])
        self.assertFalse(result['placement_authorized'])
        self.assertEqual(review.raw, old)
        value['request']['slots'][0]['max_vertical_distance'] = 127
        self.assertEqual(review.raw, old)  # No shared mutable request escapes custody.

    def test_retained_request_cannot_silently_drop_or_widen_opt_in(self):
        review, _, _, _, _ = reviewed()
        for change in ('omit', 'zero', 'widen', 'null'):
            value = review.json()
            if change == 'omit':
                value['request']['slots'][0].pop('max_vertical_distance')
            else:
                value['request']['slots'][0]['max_vertical_distance'] = {'zero': 0, 'widen': 3, 'null': None}[change]
            raw = canonical(value)
            with self.subTest(change=change), self.assertRaises(ValueError):
                reviews.Review.decode(raw, review.digest)
            if change != 'widen':
                with self.assertRaises(ValueError):
                    reviews.Review.decode(raw, reviews.object_digest(raw))
        widened = review.json()
        widened['request']['slots'][0]['max_vertical_distance'] = 3
        new = reviews.Review.decode(canonical(widened), reviews.object_digest(canonical(widened)))
        self.assertNotEqual(new.digest, review.digest)
        self.assertEqual(new.unpack()[3].digest, review.unpack()[3].digest)

    def test_moved_original_item_can_use_another_allowed_level_but_never_exceed_bound(self):
        raw, _ = terrain()
        r = request(1)
        original = item(position=(4, 2, 1))
        review, _, _, types, _ = reviewed(r, (original,), raw)
        for z, expected in [(2, 'constraints_met'), (1, 'constraints_met'), (0, 'constraints_not_met')]:
            moved = replace(original, position=(4, 2, z))
            fresh, _ = terrain(tick=101)
            result = reviews.revalidate(review, (moved,), types, fresh, source(fresh, inventory=str(z)))
            self.assertEqual(result['status'], expected)
            self.assertEqual(result['rows'][0]['item'], original.id)
            if z == 0:
                self.assertEqual(result['rows'][0]['reason'], 'original_distance_or_level_constraint')
                self.assertIsNone(result['plan'])

    def test_broken_stairs_block_original_item_even_with_easy_substitute(self):
        review, _, candidates, types, _ = reviewed()
        broken, _ = terrain(changes={(2, 2, 1): (2, 3, 0, 0, 0, 1)}, tick=101)
        replacement = item(2, position=(4, 2, 2))
        result = reviews.revalidate(review, candidates+(replacement,), {**types, 2: 1}, broken, source(broken))
        self.assertEqual(result['status'], 'constraints_not_met')
        self.assertEqual(result['rows'][0]['reason'], 'no_route_in_observed_model')
        self.assertEqual(result['rows'][0]['item'], 1)
        self.assertIsNone(result['plan'])
        self.assertFalse(result['reallocation_performed'])

    def test_original_route_and_3d_distance_limits_survive_reopen(self):
        r = request(2, max_distance=2)
        review, _, candidates, types, raw = reviewed(r, route_limit=6)
        same = reviews.Review.decode(review.raw, review.digest)
        moved = replace(candidates[0], position=(5, 2, 0))
        result = reviews.revalidate(same, (moved,), types, raw, source(raw, inventory='moved'))
        self.assertEqual(result['rows'][0]['reason'], 'original_distance_or_level_constraint')
        # Retain geometry freedom but a stricter native-model path length cap.
        review, _, candidates, types, raw = reviewed(request(2), route_limit=6)
        result = reviews.revalidate(review, (moved,), types, raw, source(raw, inventory='moved'))
        self.assertEqual(result['rows'][0]['reason'], 'original_route_limit_exceeded')
        self.assertEqual(result['rows'][0]['route_steps'], 7)

    def test_missing_item_and_uncaptured_floor_do_not_become_absence_proof(self):
        review, _, candidates, types, raw = reviewed()
        result = reviews.revalidate(review, (), {}, raw, source(raw, inventory='missing'))
        self.assertEqual(result['status'], 'unestablished')
        self.assertFalse(result['global_absence_proven'])
        self.assertIsNone(result['plan'])
        # Reopening with a shallower map is not permitted to silently narrow the domain.
        smaller, _ = terrain(size=(6, 5, 2))
        with self.assertRaisesRegex(ValueError, 'source changed'):
            reviews.revalidate(review, candidates, types, smaller, source(smaller))

    def test_native_type_material_source_and_whole_review_budget_still_apply(self):
        review, _, candidates, types, raw = reviewed()
        result = reviews.revalidate(review, (replace(candidates[0], material=(0, 0)),), types, raw, source(raw))
        self.assertEqual(result['rows'][0]['reason'], 'selected_item_identity_changed')
        result = reviews.revalidate(review, candidates, {1: 2}, raw, source(raw))
        self.assertEqual(result['rows'][0]['reason'], 'selected_item_identity_changed')
        for field, value in [('map_generation', 8), ('inventory_generation', 12), ('df_version', 'other'),
                             ('endpoint', '127.0.0.1:5001')]:
            with self.assertRaises(ValueError):
                reviews.revalidate(review, candidates, types, raw, source(raw, **{field: value}))
        with self.assertRaises(ValueError):
            reviews.revalidate(review, candidates, types, raw, source(raw), maximum_work=1)

    def test_legacy_same_floor_review_bytes_and_revalidation_unchanged(self):
        review, _, candidates, types, raw = reviewed(request(target=(4, 2, 0)), (item(position=(3, 2, 0)),))
        self.assertEqual(hashlib.sha256(review.raw).hexdigest(), LEGACY_REVIEW_SHA256)
        result = reviews.revalidate(review, candidates, types, raw, source(raw))
        self.assertEqual(hashlib.sha256(canonical(result)).hexdigest(), LEGACY_REVALIDATION_SHA256)


LEGACY_REVIEW_SHA256 = 'bfc0ba8b10031a046e7ff3a5fcef4e5246b71988ec8a780ceefd98224e342b0e'
LEGACY_REVALIDATION_SHA256 = '84330e86e3c6732f97425b313a798508ec13cd5adf4d0ccd958e611542e507a6'

if __name__ == '__main__':
    unittest.main(verbosity=2)
