"""Actual original-route checker and strict native receipt codecs; no game I/O."""
from dataclasses import replace
import hashlib
import struct
import unittest

from furniture_allocation import Candidate, Request, Slot
from furniture_plan import canonical
from build_placement_wire import Capture, Insertion, Item, Plan, Record, Selection, Tile
import route_furniture_allocation as allocation
from route_furniture_review import Source, create
import route_furniture_frontier as frontier

BATCH = 'a' * 64


def terrain(*, blocked=(), tick=10):
    raw = b'DFMM1500' + struct.pack('>IIBIH', 0, tick, 1, 2, 7) + b'region1'
    raw += struct.pack('>IIIIIIIIII', 10, 10, 10, 1, 1, 5, 5, 3, 1, 15)
    for index in range(15):
        raw += bytes([1]) if index in blocked else bytes([2]) + struct.pack('>IBBBBBBBIHH', 1, 3, 0, 0, 0, 0, 0, 0, 1, 10015, 10015)
    return raw


def source(raw, tick=10, horizons=(100, 200, 300)):
    return Source('127.0.0.1:5000', 'df', 'dfhack', 7, 9,
                  hashlib.sha256(raw).hexdigest(), 'b' * 64, tick,
                  (10, 10, 10), (1, 1, 5), (5, 3, 1), horizons)


def fixture():
    request = Request('region1', 2, (Slot('bed', 'bed', (5, 1, 5)),
                                    Slot('chair', 'chair', (5, 3, 5), ('bed',))))
    items = (Candidate(10, 'bed', (1, 1, 5), (419, -1), -1),
             Candidate(20, 'chair', (1, 3, 5), (419, -1), -1))
    raw = terrain()
    report = allocation.allocate_routes(request, items, raw)
    review = create(request, report, source(raw), {10: 30, 20: 31})
    return review, items, raw


def capture(step=0, *, tick=10, sequence=0):
    kind, item_id, target, position, native = ((1, 10, (5, 1, 5), (1, 1, 5), 30) if step == 0 else
                                            (2, 20, (5, 3, 5), (1, 3, 5), 31))
    tile = Tile(2, 1, 3)
    item = Item(presence=2, pos=position, kind=kind, native_type=native,
                subtype=-1, material=419, material_index=-1, on_ground=True, ground=tile)
    return Capture(11, sequence, tick, 2, (10, 10, 10), 200 + step, 100 + step, step,
                   'region1', True, True, True, Selection(kind, item_id, *target), (tile,) * 9, item)


def receipt(before=None, batch=BATCH):
    before = capture() if before is None else before
    step = 'bed' if before.selection.kind == 1 else 'chair'
    plan = Plan(frontier.step_key(batch, step), before)
    inserted = Insertion(before.next_building, before.next_job, before.selection.item,
                         before.selection.kind, before.selection.target, 419, -1,
                         0, 3, True, True, True, False)
    return Record(plan, 'placed', 'none', before.expected_after(), inserted).raw


class FrontierTests(unittest.TestCase):
    def setUp(self):
        self.review, self.items, self.raw = fixture()

    def check(self, *, placed=(), before=None, items=None, raw=None, current=None, **kw):
        raw = self.raw if raw is None else raw
        return frontier.assess(self.review, BATCH, placed, capture() if before is None else before,
            self.items if items is None else items,
            {item.id: 30 if item.kind == 'bed' else 31 for item in (self.items if items is None else items)},
            raw, source(raw) if current is None else current, **kw)

    def test_initial_frontier_preserves_every_original_selection(self):
        out = self.check()
        self.assertEqual(out['status'], 'ready')
        self.assertEqual(out['next_step'], 'bed')
        self.assertEqual(len(out['rows']), 2)
        self.assertEqual(out['original_plan_digest'], self.review.unpack()[3].digest)
        self.assertFalse(out['placement_authorized'])
        self.assertFalse(out['atomic_route_placement_proven'])
        self.assertEqual(self.check(), out)

    def test_verified_prefix_not_required_to_remain_loose_inventory(self):
        out = self.check(placed=(receipt(),), before=capture(1, sequence=1), items=(self.items[1],),
                         current=source(self.raw, horizons=(101, 201, 300)))
        self.assertEqual(out['status'], 'ready')
        self.assertEqual(out['next_step'], 'chair')
        self.assertEqual(out['rows'][0]['status'], 'historical_placed')
        self.assertFalse(out['rows'][0]['current_condition_proven'])
        self.assertEqual(out['remaining_steps'], 1)

    def test_blocked_remaining_step_prevents_first_dispatch_too(self):
        # Keep first target accessible but make the second endpoint unknown.
        out = self.check(raw=terrain(blocked=(14,)))
        self.assertEqual(out['status'], 'route_unestablished')
        self.assertEqual(out['rows'][0]['status'], 'constraints_met')
        self.assertEqual(out['rows'][1]['status'], 'unestablished')

    def test_earlier_placement_obstructing_later_corridor_is_not_skipped(self):
        raw = terrain(blocked=(2, 7, 12))
        out = self.check(placed=(receipt(),), before=capture(1, sequence=1), items=(self.items[1],), raw=raw,
                         current=source(raw, horizons=(101, 201, 300)))
        self.assertEqual(out['status'], 'route_blocked')
        self.assertEqual(out['rows'][1]['reason'], 'no_route_in_observed_model')

    def test_receipt_from_another_batch_or_step_never_counts(self):
        for raw in (receipt(batch='c' * 64), receipt(capture(1, sequence=1))):
            with self.assertRaises(ValueError):
                self.check(placed=(raw,), before=capture(1, sequence=2),
                           current=source(self.raw, horizons=(101, 201, 300)))

    def test_unknown_prepared_refused_and_cancelled_do_not_unlock_next(self):
        p = Plan(frontier.step_key(BATCH, 'bed'), capture())
        for phase, reason in (('prepared', 'none'), ('indeterminate', 'native_failure'),
                              ('refused', 'stale'), ('cancelled', 'cancelled')):
            with self.subTest(phase=phase), self.assertRaises(ValueError):
                self.check(placed=(Record(p, phase, reason).raw,), before=capture(1, sequence=1),
                           current=source(self.raw, horizons=(101, 201, 300)))

    def test_cannot_skip_first_step_or_reopen_completed_frontier(self):
        with self.assertRaises(ValueError):
            self.check(before=capture(1))
        with self.assertRaises(ValueError):
            self.check(placed=(receipt(), receipt(capture(1, sequence=1))))

    def test_fresh_native_capture_must_match_route_clock_horizons_and_item(self):
        for before in (replace(capture(), tick=11), replace(capture(), next_job=101),
                       replace(capture(), next_building=201),
                       replace(capture(), item=replace(capture().item, pos=(2, 1, 5))),
                       replace(capture(), item=replace(capture().item, material=420))):
            with self.subTest(before=before), self.assertRaises(ValueError):
                self.check(before=before)

    def test_prefix_native_reset_sequence_and_clock_regression_refused(self):
        for before in (capture(1, sequence=0), replace(capture(1, sequence=1), generation=12)):
            with self.assertRaises(ValueError):
                self.check(placed=(receipt(),), before=before,
                           current=source(self.raw, horizons=(101, 201, 300)))

    def test_new_stock_cannot_replace_an_original_missing_item(self):
        new = (self.items[0], replace(self.items[1], id=21, position=(5, 3, 5)))
        out = self.check(items=new)
        self.assertEqual(out['status'], 'route_unestablished')
        self.assertEqual(out['rows'][1]['item'], 20)
        self.assertFalse(out['reallocation_performed'])

    def test_map_and_inventory_generations_stay_separate_but_fixed(self):
        self.assertEqual(self.check()['status'], 'ready')
        for field in ('map_generation', 'inventory_generation'):
            with self.assertRaises(ValueError):
                self.check(current=replace(source(self.raw), **{field: 22}))

    def test_every_receipt_truncation_and_byte_corruption_is_refused(self):
        raw = receipt()
        for bad in [raw[:i] for i in range(len(raw))] + [raw[:i] + bytes([raw[i] ^ 1]) + raw[i+1:] for i in range(len(raw))]:
            with self.assertRaises(ValueError):
                self.check(placed=(bad,), before=capture(1, sequence=1),
                           current=source(self.raw, horizons=(101, 201, 300)))

    def test_exact_shared_work_limit_and_stable_evidence_identity(self):
        out = self.check()
        bounded = self.check(maximum_work=out['work_units'])
        self.assertEqual(bounded['evidence_digest'], out['evidence_digest'])
        with self.assertRaises(ValueError):
            self.check(maximum_work=out['work_units'] - 1)

    def test_every_cooperative_abort_including_publication_returns_no_result(self):
        calls = []
        self.check(guard=lambda: calls.append(True))
        for abort in range(len(calls)):
            seen = []
            def guard():
                seen.append(True)
                if len(seen) == abort + 1:
                    raise ValueError('cancelled')
            with self.assertRaises(ValueError):
                self.check(guard=guard)


if __name__ == '__main__':
    unittest.main()
