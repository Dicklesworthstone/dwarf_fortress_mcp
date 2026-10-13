"""Actual original-selection review implementation; no network or game required."""
from dataclasses import replace
import hashlib
import json
import struct
import unittest
from unittest.mock import patch

from furniture_allocation import Candidate, Request, Slot
from furniture_plan import canonical
import route_furniture_allocation as routes
import route_furniture_review as model


def terrain(*, tick=100, hidden=(), shape_changes=None, size=(5, 3, 1),
            origin=(0, 0, 5), folder='fort', site=2, dimensions=(128, 128, 16), paused=True):
    folder = folder.encode()
    raw = (b'DFMM1500' + struct.pack('>IIBIH', tick // 403200, tick % 403200, paused, site, len(folder))
           + folder + struct.pack('>10I', *dimensions, *origin, *size, size[0] * size[1] * size[2]))
    for i in range(size[0] * size[1] * size[2]):
        if i in hidden:
            raw += b'\1'
        else:
            values = [1, 3, 0, 0, 0, 0, 0, 0, 1, 10015, 10015]
            for field, value in (shape_changes or {}).get(i, {}).items():
                values[field] = value
            raw += b'\2' + struct.pack('>IBBBBBBBIHH', *values)
    return raw


def source(raw, **changes):
    t = routes.decode_terrain(raw)
    return replace(model.Source('127.0.0.1:5000', 'df', 'dfhack', 7, 19,
        t.digest, hashlib.sha256(b'inventory').hexdigest(), t.tick, t.dimensions,
        t.origin, t.size, (100, 200, 65537)), **changes)


def fixture(*, slots=None, items=None, limit=10, raw=None):
    request = Request('fort', 2, slots or (Slot('bed', 'bed', (3, 1, 5), max_distance=8),))
    candidates = items or (Candidate(42, 'bed', (0, 1, 5), (419, -1), -1),)
    raw = raw or terrain()
    allocation = routes.allocate_routes(request, candidates, raw, max_route_steps=limit)
    types = {row['item']: {'bed': 1, 'chair': 2, 'table': 3}[row['kind']] for row in allocation['assignments']}
    review = model.create(request, allocation, source(raw), types)
    return review, request, candidates, raw


def check(review, items, raw, *, current=None, types=None, **kwargs):
    types = types if types is not None else {item.id: {'bed': 1, 'chair': 2, 'table': 3}[item.kind] for item in items}
    return model.revalidate(review, items, types, raw, current or source(raw), **kwargs)


class ReviewTests(unittest.TestCase):
    def test_canonical_round_trip_preserves_whole_original_intent(self):
        slots = (Slot('table', 'table', (3, 2, 5), after=('bed',), material=(419, -1),
                      subtype=2, max_distance=7), Slot('bed', 'bed', (3, 1, 5)))
        items = (Candidate(42, 'bed', (0, 1, 5), (419, -1), -1),
                 Candidate(43, 'table', (0, 2, 5), (419, -1), 2))
        review, request, items, raw = fixture(slots=slots, items=items)
        loaded = model.Review.decode(review.raw, review.digest)
        self.assertEqual(loaded, review)
        value, restored, origin, plan = loaded.unpack()
        self.assertEqual(restored, request)
        self.assertEqual(plan.steps[1].after, ('bed',))
        self.assertEqual(origin.map_generation, 7)
        self.assertEqual(origin.inventory_generation, 19)
        value['request']['slots'][0]['max_distance'] = 0
        self.assertEqual(loaded.raw, review.raw)
        self.assertNotEqual(loaded.json(), value)
        self.assertEqual(check(loaded, items, raw)['plan'], plan.json())

    def test_revalidation_never_reallocates_even_with_better_new_item(self):
        review, request, items, raw = fixture()
        later = items + (Candidate(1, 'bed', (3, 1, 5), (419, -1), -1),)
        with patch.object(routes, 'allocate_routes', side_effect=AssertionError('must not allocate')):
            result = check(review, later, raw)
        self.assertEqual(result['status'], 'constraints_met')
        self.assertEqual(result['rows'][0]['item'], 42)
        self.assertEqual(result['plan'], review.unpack()[3].json())
        self.assertFalse(result['reallocation_performed'])
        self.assertFalse(result['placement_authorized'])

    def test_missing_original_is_unknown_despite_usable_replacement(self):
        review, _, items, raw = fixture()
        result = check(review, (replace(items[0], id=1),), raw)
        self.assertEqual(result['status'], 'unestablished')
        self.assertIsNone(result['plan'])
        self.assertEqual(result['rows'][0]['item'], 42)
        self.assertEqual(result['rows'][0]['reason'], 'selected_item_missing_or_ineligible')
        self.assertFalse(result['global_absence_proven'])
        self.assertEqual(result['original_plan'], review.unpack()[3].json())

    def test_selected_metadata_changes_are_not_substituted_or_normalized(self):
        review, _, items, raw = fixture()
        for changed in (replace(items[0], material=(420, -1)), replace(items[0], subtype=3),
                        replace(items[0], kind='table')):
            with self.subTest(changed=changed):
                result = check(review, (changed,), raw)
                self.assertEqual(result['status'], 'constraints_not_met')
                self.assertEqual(result['rows'][0]['reason'], 'selected_item_identity_changed')
                self.assertIsNone(result['plan'])
        self.assertEqual(check(review, items, raw, types={42: 77})['status'], 'constraints_not_met')

    def test_moving_original_item_rechecks_original_manhattan_level_and_route_limit(self):
        review, _, items, raw = fixture(slots=(Slot('bed', 'bed', (3, 1, 5), max_distance=3),), limit=3)
        valid = check(review, (replace(items[0], position=(2, 1, 5)),), terrain(tick=101))
        self.assertEqual(valid['status'], 'constraints_met')
        self.assertEqual(valid['rows'][0]['route_steps'], 1)
        for position in ((0, 0, 5), (1, 1, 6)):
            result = check(review, (replace(items[0], position=position),), raw)
            self.assertEqual(result['rows'][0]['reason'], 'original_distance_or_level_constraint')
        detour = terrain(hidden=(6, 7))
        result = check(review, items, detour)
        self.assertEqual(result['rows'][0]['reason'], 'original_route_limit_exceeded')
        self.assertEqual(result['rows'][0]['route_steps'], 5)
        self.assertEqual(result['max_route_steps'], 3)

    def test_new_wall_disconnects_and_hidden_endpoints_remain_unknown(self):
        review, _, items, _ = fixture()
        split = terrain(hidden=(2, 7, 12))
        result = check(review, items, split)
        self.assertEqual(result['rows'][0]['reason'], 'no_route_in_observed_model')
        self.assertEqual(result['status'], 'constraints_not_met')
        for hidden, reason in (((8,), 'target_hidden'), ((5,), 'item_hidden')):
            result = check(review, items, terrain(hidden=hidden))
            self.assertEqual(result['rows'][0]['reason'], reason)
            self.assertEqual(result['status'], 'unestablished')
            self.assertIsNone(result['plan'])

    def test_source_changes_refuse_instead_of_rebinding(self):
        review, _, items, raw = fixture()
        for field, changed in (('endpoint', '127.0.0.1:5001'), ('df_version', 'other'),
                               ('dfhack_version', 'other'), ('map_generation', 8),
                               ('inventory_generation', 20), ('dimensions', (127, 128, 16)),
                               ('origin', (1, 0, 5)), ('size', (4, 3, 1)),
                               ('tick', 99), ('horizons', (99, 200, 65537))):
            with self.subTest(field=field):
                with self.assertRaises(ValueError):
                    check(review, items, raw, current=replace(source(raw), **{field: changed}))
        with self.assertRaises(ValueError):
            check(review, items, raw, current=replace(source(raw), map_sha256='0' * 64))
        for bad in (terrain(folder='other'), terrain(site=3), terrain(paused=False)):
            with self.assertRaises(ValueError):
                check(review, items, bad)

    def test_all_original_slots_checked_even_when_first_one_is_blocked(self):
        slots = (Slot('a', 'bed', (3, 1, 5)), Slot('b', 'bed', (3, 2, 5)))
        items = (Candidate(42, 'bed', (0, 1, 5), (419, -1), -1),
                 Candidate(43, 'bed', (0, 2, 5), (419, -1), -1))
        review, _, _, raw = fixture(slots=slots, items=items)
        result = check(review, (replace(items[0], subtype=5), items[1]), raw)
        self.assertEqual(len(result['rows']), 2)
        self.assertEqual(result['counts'], {'constraints_met': 1, 'constraints_not_met': 1, 'unestablished': 0})
        self.assertTrue(result['all_original_selections_checked'])
        self.assertIsNone(result['plan'])

    def test_import_refuses_wrong_external_identity_and_all_corruptions(self):
        review, _, _, _ = fixture()
        for width in range(len(review.raw)):
            with self.assertRaises(ValueError):
                model.Review.decode(review.raw[:width], review.digest)
        for index in range(len(review.raw)):
            raw = bytearray(review.raw)
            raw[index] ^= 1
            with self.assertRaises(ValueError):
                model.Review.decode(bytes(raw), review.digest)
        for digest in ('0' * 64, review.digest.upper(), 'x', None):
            with self.assertRaises(ValueError):
                model.Review.decode(review.raw, digest)

    def test_rechecksummed_invalid_objects_unknown_fields_and_policies_refuse(self):
        review, _, _, _ = fixture()
        mutations = (
            lambda v: v.update(policy='other'),
            lambda v: v.update(route_policy='other'),
            lambda v: v.update(path_policy='other'),
            lambda v: v.update(max_route_steps=0),
            lambda v: v.update(unregistered=True),
            lambda v: v.update(selections=[]),
            lambda v: v['selections'][0].update(slot='other'),
            lambda v: v['selections'][0].update(route_steps=True),
            lambda v: v['selections'][0]['candidate'].update(item=65537),
            lambda v: v['source'].update(endpoint='localhost:5000'),
            lambda v: v['request']['slots'][0].update(after=['missing']),
            lambda v: v.update(plan_digest='0' * 64),
        )
        for index, mutate in enumerate(mutations):
            value = review.json()
            mutate(value)
            raw = canonical(value)
            with self.subTest(index=index), self.assertRaises(ValueError):
                model.Review.decode(raw, model.object_digest(raw))
        for raw in (b'{"schema":0,"schema":0}', b'[' * 13 + b'0' + b']' * 13,
                    review.raw + b'\n', b'{"a":NaN}', b'{"a":1.2}'):
            with self.assertRaises(ValueError):
                model.Review.decode(raw, model.object_digest(raw))

    def test_current_inventory_malformed_including_unselected_items_is_refused(self):
        review, _, items, raw = fixture()
        for candidates, types in ((items * 2, {42: 1}), (items, {}),
                                  (items, {42: True}), (items, {42: 1, 43: 1}),
                                  (items + (replace(items[0], id=65537),), {42: 1, 65537: 1}),
                                  (items + (replace(items[0], id=43),), {42: 1, 43: 2})):
            with self.assertRaises(ValueError):
                check(review, candidates, raw, types=types)

    def test_boolean_native_identity_cannot_alias_an_item_id(self):
        review, _, items, raw = fixture(items=(Candidate(1, 'bed', (0, 1, 5), (419, -1), -1),))
        with self.assertRaises(ValueError):
            check(review, items, raw, types={True: 1})

    def test_exact_work_budget_and_all_cooperative_abort_boundaries(self):
        review, _, items, raw = fixture()
        calls = []
        result = check(review, items, raw, guard=lambda: calls.append(1))
        self.assertEqual(check(review, items, raw, maximum_work=result['work_units'])['rows'], result['rows'])
        with self.assertRaises(ValueError):
            check(review, items, raw, maximum_work=result['work_units'] - 1)
        for fail_at in range(1, len(calls) + 1):
            count = 0
            def fail():
                nonlocal count
                count += 1
                if count == fail_at:
                    raise ValueError('cancelled')
            with self.assertRaises(ValueError):
                check(review, items, raw, guard=fail)
        self.assertEqual(review.raw, canonical(review.json()))

    def test_equivalent_item_order_is_deterministic_and_source_changes_bind_results(self):
        review, _, items, raw = fixture()
        expanded = items + (replace(items[0], id=43),)
        result = check(review, expanded, raw)
        self.assertEqual(result, check(review, expanded[::-1], raw))
        later = terrain(tick=101)
        self.assertNotEqual(result['decision_digest'], check(review, expanded, later)['decision_digest'])
        self.assertEqual(result['review_digest'], review.digest)

    def test_all_obstacle_masks_revalidate_original_not_new_optimum(self):
        raw = terrain(size=(3, 3, 1), origin=(1, 1, 5))
        request = Request('fort', 2, (Slot('a', 'bed', (3, 3, 5)),))
        items = (Candidate(42, 'bed', (1, 1, 5), (419, -1), -1),)
        allocation = routes.allocate_routes(request, items, raw, max_route_steps=8)
        review = model.create(request, allocation, source(raw), {42: 1})
        for mask in range(512):
            hidden = tuple(i for i in range(9) if mask & (1 << i))
            current = terrain(size=(3, 3, 1), origin=(1, 1, 5), hidden=hidden, tick=101)
            result = check(review, items, current)
            if mask & 1 or mask & 256:
                self.assertEqual(result['status'], 'unestablished')
            else:
                # Independent repeated distance relaxation on the 3x3 grid.
                d = [99] * 9
                d[0] = 0
                for _ in range(9):
                    for a in range(9):
                        for b in range(9):
                            if a not in hidden and b not in hidden and abs(a % 3 - b % 3) + abs(a // 3 - b // 3) == 1:
                                d[b] = min(d[b], d[a] + 1)
                self.assertEqual(result['status'] == 'constraints_met', d[8] < 99, mask)
                if d[8] < 99:
                    self.assertEqual(result['rows'][0]['route_steps'], d[8])

    def test_create_cannot_seal_shortages_partial_plans_or_changed_decisions(self):
        review, request, items, raw = fixture()
        allocation = routes.allocate_routes(request, items, raw)
        for change in ('status', 'plan', 'route'):
            v = json.loads(canonical(allocation))
            if change == 'status':
                v['status'] = 'shortage'
            elif change == 'plan':
                v['plan']['steps'][0]['item'] = 7
            else:
                v['assignments'][0]['route_steps'] = 0
            with self.assertRaises(ValueError):
                model.create(request, v, source(raw), {42: 1})

    def test_maximum_items_and_thirty_two_original_targets(self):
        raw = terrain(size=(34, 3, 1))
        request = Request('fort', 2, tuple(Slot(f's{i:02}', 'bed', (i + 1, 1, 5)) for i in range(32)))
        originals = tuple(Candidate(i, 'bed', (i + 1, 1, 5), (419, -1), -1) for i in range(32))
        allocation = routes.allocate_routes(request, originals, raw, max_route_steps=30)
        review = model.create(request, allocation, source(raw), {i: 1 for i in range(32)})
        inventory = originals + tuple(Candidate(i, 'bed', (0, 0, 5), (419, -1), -1) for i in range(32, 65536))
        result = check(review, inventory, raw)
        self.assertEqual(result['counts']['constraints_met'], 32)
        self.assertEqual(result['plan'], allocation['plan'])
        self.assertLessEqual(len(canonical(result)), model.MAX_BYTES)
        self.assertLessEqual(len(review.raw), model.MAX_BYTES)


if __name__ == '__main__':
    unittest.main()
