"""Actual original review, native value codecs, and mandatory frontier suffix gate.

All captures/receipts are synthetic. No native connection or durable batch owner
is replaced and claimed as executed by this suite.
"""
from __future__ import annotations

from dataclasses import replace
import hashlib
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from build_placement_wire import Capture, Insertion, Item, KINDS, Plan, Record, Selection, Tile
from furniture_allocation import Candidate, Request, Slot
from furniture_plan import canonical
import route_furniture_allocation as allocation
import route_furniture_frontier as frontier
from route_furniture_review import Review, Source, create, revalidate
from route_furniture_suffix_tests import capture as terrain_capture, corridor

BATCH = 'a' * 64


def source(raw, *, tick=100, prefix=0):
    return Source('127.0.0.1:5000', 'df', 'dfhack', 7, 9,
        hashlib.sha256(raw).hexdigest(), 'b' * 64, tick,
        (128, 128, 8), (0, 0, 0), (7, 4, 1), (100 + prefix, 200 + prefix, 300))


def fixture(*, max_route_steps=20):
    plan, positions, shapes = corridor(True)
    request = Request('fort', 1, tuple(Slot(s.name, s.kind, s.target, s.after,
        material=(419, -1), subtype=-1, max_distance=20) for s in plan.steps))
    items = tuple(Candidate(s.item, s.kind, positions[s.item], (419, -1), -1) for s in plan.steps)
    types = {11: 30, 12: 31}
    raw = terrain_capture(shapes)
    result = allocation.allocate_routes(request, items, raw, max_route_steps=max_route_steps)
    review = create(request, result, source(raw), types)
    return review, items, types, raw


def native_before(review, items, *, prefix=0, tick=100):
    step = review.unpack()[3].ordered[prefix]
    item = next(item for item in items if item.id == step.item)
    tile = Tile(2, 10, 3)
    native_item = Item(presence=2, pos=item.position, kind=KINDS.index(item.kind),
        native_type=30 if item.kind == 'bed' else 31, subtype=item.subtype,
        material=item.material[0], material_index=item.material[1], on_ground=True, ground=tile)
    return Capture(11, prefix, tick, 1, (128, 128, 8), 200 + prefix, 100 + prefix, prefix,
        'fort', True, True, True, Selection(KINDS.index(step.kind), step.item, *step.target),
        (tile,) * 9, native_item)


def placed(before, name):
    insert = Insertion(before.next_building, before.next_job, before.selection.item,
        before.selection.kind, before.selection.target, before.item.material,
        before.item.material_index, 0, 3, True, True, True, False)
    return Record(Plan(frontier.step_key(BATCH, name), before), 'placed', 'none',
                  before.expected_after(), insert).raw


class SuffixFrontierTests(unittest.TestCase):
    def setUp(self):
        self.review, self.items, self.types, self.raw = fixture()
        self.before = native_before(self.review, self.items)

    def assess(self, *, raw=None, items=None, prefix=(), before=None, current=None, **kw):
        raw = self.raw if raw is None else raw
        items = self.items if items is None else items
        return frontier.assess(self.review, BATCH, prefix, before or self.before,
            items, {i.id: self.types.get(i.id, 32) for i in items}, raw,
            current or source(raw), **kw)

    def test_current_individual_routes_cannot_authorize_a_blocked_suffix(self):
        bad = terrain_capture(corridor()[2])
        individual = revalidate(self.review, self.items, self.types, bad, source(bad))
        self.assertEqual(individual['status'], 'constraints_met')
        result = self.assess(raw=bad)
        self.assertEqual(result['status'], 'route_blocked')
        self.assertTrue(all(r['status'] == 'constraints_met' for r in result['rows']))
        self.assertEqual(result['delivery_suffix']['first_blocked_slot'], 'far')
        self.assertEqual(result['delivery_suffix']['routes'], [])
        self.assertEqual(result['next_step'], 'near')
        self.assertFalse(result['placement_authorized'])

    def test_before_commit_recheck_can_block_after_same_native_before_remains_eligible(self):
        first = self.assess()
        bad = terrain_capture(corridor()[2])
        final = self.assess(raw=bad)
        self.assertTrue(self.before.eligible)
        self.assertEqual(first['native_before_digest'], final['native_before_digest'])
        self.assertEqual(first['status'], 'ready')
        self.assertEqual(final['status'], 'route_blocked')
        self.assertNotEqual(first['evidence_digest'], final['evidence_digest'])
        self.assertEqual(first['review_digest'], final['review_digest'])
        self.assertEqual(first['original_plan_digest'], final['original_plan_digest'])

    def test_ready_includes_full_fresh_suffix_without_resealing_original_review(self):
        raw, digest = self.review.raw, self.review.digest
        out = self.assess()
        suffix = out['delivery_suffix']
        self.assertEqual(out['policy'], 'original-route-constraints-and-fixed-delivery-suffix/2')
        self.assertEqual(suffix['remaining_order'], ['near', 'far'])
        self.assertEqual([r['route_steps'] for r in suffix['routes']], [1, 5])
        self.assertEqual(suffix['original_plan_digest'], out['original_plan_digest'])
        self.assertEqual(suffix['terrain_sha256'], out['source']['map_sha256'])
        self.assertNotIn('work_units', suffix)
        self.assertNotIn('maximum_work', suffix)
        self.assertEqual((raw, digest), (self.review.raw, self.review.digest))
        self.assertFalse(out['future_routes_preserved'])
        self.assertFalse(out['construction_completion_proven'])

    def test_original_placed_prefix_reopens_without_loose_items_or_new_order(self):
        receipt = placed(self.before, 'near')
        # Re-decode exactly as a newly opened owner would; not a custody claim.
        self.review = Review.decode(self.review.raw, self.review.digest)
        receipt = Record.decode(receipt).raw
        items = tuple(i for i in self.items if i.id == 12)
        before = native_before(self.review, items, prefix=1, tick=101)
        good = terrain_capture(corridor(True)[2], tick=101)
        out = self.assess(raw=good, items=items, prefix=(receipt,), before=before,
                          current=source(good, tick=101, prefix=1))
        self.assertEqual(out['status'], 'ready')
        self.assertEqual(out['delivery_suffix']['placed_prefix'], ['near'])
        self.assertEqual(out['delivery_suffix']['remaining_order'], ['far'])
        self.assertEqual(out['delivery_suffix']['routes'][0]['blocked_target_count'], 1)
        self.assertEqual(out['delivery_suffix']['routes'][0]['route_steps'], 5)
        prior = next(r for r in out['rows'] if r['slot'] == 'near')
        self.assertEqual(prior['status'], 'historical_placed')
        self.assertFalse(prior['current_condition_proven'])
        # An open-looking old target cannot erase the original prefix blocker.
        bad = terrain_capture(corridor()[2], tick=101)
        out = self.assess(raw=bad, items=items, prefix=(receipt,), before=before,
                          current=source(bad, tick=101, prefix=1))
        self.assertEqual(out['status'], 'route_blocked')
        self.assertEqual(out['delivery_suffix']['first_blocked_slot'], 'far')

    def test_current_item_positions_not_historical_allocations_drive_suffix(self):
        moved = tuple(replace(i, position=(3, 1, 0)) if i.id == 12 else i for i in self.items)
        bad = terrain_capture(corridor()[2])
        out = self.assess(raw=bad, items=moved)
        self.assertEqual(out['status'], 'ready')
        self.assertEqual(out['delivery_suffix']['routes'][1]['route_steps'], 1)
        original = next(r for r in self.review.json()['selections'] if r['slot'] == 'far')
        self.assertEqual(original['candidate']['position'], [1, 1, 0])

    def test_original_selectors_and_missing_inventory_refuse_before_suffix_analysis(self):
        variants = [tuple(replace(i, material=(420, -1)) if i.id == 12 else i for i in self.items),
                    tuple(i for i in self.items if i.id != 12),
                    tuple(replace(i, id=13) if i.id == 12 else i for i in self.items)]
        for items in variants:
            with self.subTest(items=items), patch.object(frontier, 'assess_remaining',
                side_effect=AssertionError('unqualified selections reached suffix')):
                out = self.assess(items=items)
                self.assertNotEqual(out['status'], 'ready')
                self.assertIsNone(out['delivery_suffix'])

    def test_unknown_receipt_never_becomes_prefix_authority_for_suffix(self):
        unknown = Record(Plan(frontier.step_key(BATCH, 'near'), self.before),
                         'indeterminate', 'native_failure').raw
        before = native_before(self.review, self.items, prefix=1)
        with patch.object(frontier, 'assess_remaining', side_effect=AssertionError('unknown skipped')):
            with self.assertRaises(ValueError):
                self.assess(prefix=(unknown,), before=before, current=source(self.raw, prefix=1))

    def test_original_route_limit_is_not_replaced_by_global_default(self):
        self.review, self.items, self.types, self.raw = fixture(max_route_steps=4)
        self.before = native_before(self.review, self.items)
        result = self.assess()
        self.assertTrue(all(r['status'] == 'constraints_met' for r in result['rows']))
        self.assertEqual(result['status'], 'route_blocked')
        self.assertEqual(result['delivery_suffix']['max_route_steps'], 4)

    def test_suffix_consumes_remaining_budget_and_nested_seal_is_allowance_independent(self):
        first = self.assess()
        exact = self.assess(maximum_work=first['work_units'])
        self.assertEqual(first['evidence_digest'], exact['evidence_digest'])
        self.assertEqual(first['delivery_suffix'], exact['delivery_suffix'])
        with self.assertRaises(ValueError):
            self.assess(maximum_work=first['work_units'] - 1)
        called = []
        original = frontier.assess_remaining
        def record(*args, **kwargs):
            called.append(kwargs['maximum_work'])
            return original(*args, **kwargs)
        with patch.object(frontier, 'assess_remaining', side_effect=record):
            self.assess(maximum_work=first['work_units'])
        self.assertEqual(len(called), 1)
        self.assertLess(called[0], first['work_units'])

    def test_old_individual_route_confirmation_is_not_a_current_suffix_confirmation(self):
        out = self.assess()
        new = {k: v for k, v in out.items() if k not in ('work_units', 'maximum_work', 'evidence_digest')}
        old = {k: v for k, v in new.items() if k != 'delivery_suffix'}
        old['policy'] = 'original-route-constraints-after-verified-placement-prefix/1'
        old_digest = hashlib.sha256(b'dfmcp-route-furniture-frontier/1\0' + canonical(old)).hexdigest()
        self.assertNotEqual(out['evidence_digest'], old_digest)
        self.assertEqual(out['evidence_digest'], hashlib.sha256(
            b'dfmcp-route-furniture-frontier/1\0' + canonical(new)).hexdigest())

    def test_suffix_failure_and_oversized_publication_do_not_return_prior_ready_result(self):
        with patch.object(frontier, 'assess_remaining', side_effect=ValueError('cancelled in suffix')):
            with self.assertRaisesRegex(ValueError, 'cancelled in suffix'):
                self.assess()
        with patch.object(frontier, 'MAX_OUTPUT', 1), self.assertRaises(ValueError):
            self.assess()


if __name__ == '__main__':
    unittest.main()
