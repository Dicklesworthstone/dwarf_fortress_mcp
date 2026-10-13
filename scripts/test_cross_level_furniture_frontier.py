"""Actual codec/frontier tests, with explicit captures and no placement writer.

This exercises receipt-verified prefix reasoning, not native acquisition or the
private batch coordinator. Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from dataclasses import replace
import unittest

import build_placement_wire as wire
from furniture_allocation import Request, Slot
from furniture_plan import canonical
import route_furniture_frontier as frontier
from test_cross_level_furniture import terrain, item, request
from test_cross_level_furniture_review import reviewed, source

BATCH = 'a' * 64


def capture(candidate, target, *, sequence=0, tick=100, next_building=20, next_job=30):
    floor = wire.Tile(presence=2, tiletype=1, shape=3)
    kind = wire.KINDS.index(candidate.kind)
    loose = wire.Item(presence=2, pos=candidate.position, kind=kind, native_type=kind,
                      subtype=candidate.subtype, material=candidate.material[0],
                      material_index=candidate.material[1], on_ground=True, ground=floor)
    return wire.Capture(41, sequence, tick, 1, (8, 7, 3), next_building, next_job,
                        sequence, 'fort', True, True, True,
                        wire.Selection(kind, candidate.id, *target), (floor,) * 9, loose)


def placed(before, name):
    p = wire.Plan(frontier.step_key(BATCH, name), before)
    proof = wire.Insertion(before.next_building, before.next_job, before.selection.item,
                           before.selection.kind, before.selection.target, before.item.material,
                           before.item.material_index, 0, 3, True, True, True, False)
    return wire.Record(p, 'placed', 'none', before.expected_after(), proof)


def prefix():
    r = Request('fort', 1, (
        Slot('a', 'bed', (4, 2, 2), max_vertical_distance=2),
        Slot('b', 'chair', (5, 3, 2), after=('a',), max_vertical_distance=1)))
    candidates = (item(1), item(2, (5, 3, 1), kind='chair'))
    review, _, _, _, _ = reviewed(r, candidates)
    a = capture(candidates[0], r.slots[0].target)
    receipt = placed(a, 'a')
    before = capture(candidates[1], r.slots[1].target, sequence=1, next_building=21, next_job=31)
    raw, _ = terrain(changes={(4, 2, 2): (2, 3, 0, 1, 0, 1)})
    current = source(raw, inventory='after-a', horizons=(31, 21, 100))
    return review, receipt, before, candidates[1], raw, current


class CrossLevelFrontierTests(unittest.TestCase):
    def test_exact_native_selection_reaches_full_route_frontier(self):
        review, _, candidates, types, raw = reviewed()
        before = capture(candidates[0], (4, 2, 2))
        self.assertTrue(before.eligible)
        self.assertEqual(frontier.bind_native_selection(review, before), 'bed')
        result = frontier.assess(review, BATCH, (), before, candidates, types, raw, source(raw))
        self.assertEqual(result['status'], 'ready')
        self.assertEqual(result['rows'][0]['current_position'], [4, 2, 0])
        self.assertEqual(result['rows'][0]['route_steps'], 6)
        self.assertEqual(result['remaining_steps'], 1)
        self.assertEqual(result['native_before_digest'], before.witness.hex())
        for flag in ('reallocation_performed', 'placement_authorized', 'retry_permitted',
                     'construction_completion_proven', 'native_pathfinding_proven'):
            self.assertFalse(result[flag])
        bound = result['work_units']
        same = frontier.assess(review, BATCH, (), before, candidates, types, raw, source(raw), maximum_work=bound)
        self.assertEqual(same['evidence_digest'], result['evidence_digest'])
        with self.assertRaises(ValueError):
            frontier.assess(review, BATCH, (), before, candidates, types, raw, source(raw), maximum_work=bound-1)

    def test_native_before_cannot_relax_level_material_or_identity(self):
        original = item(position=(4, 2, 1))
        review, _, _, _, _ = reviewed(request(1), (original,))
        before = capture(original, (4, 2, 2))
        self.assertEqual(frontier.bind_native_selection(review, before), 'bed')
        for changed in (replace(before, item=replace(before.item, pos=(4, 2, 0))),
                        replace(before, item=replace(before.item, material=420)),
                        replace(before, item=replace(before.item, native_type=99)),
                        replace(before, selection=replace(before.selection, item=2))):
            with self.subTest(changed=changed.witness.hex()), self.assertRaises(ValueError):
                frontier.bind_native_selection(review, changed)

    def test_placed_prefix_retains_constraints_on_every_remaining_item(self):
        review, receipt, before, remaining, raw, current = prefix()
        out = frontier.assess(review, BATCH, (receipt.raw,), before,
                              (remaining,), {2: 2}, raw, current)
        self.assertEqual(out['status'], 'ready')
        self.assertEqual(out['remaining_steps'], 1)
        self.assertEqual(out['rows'][0]['status'], 'historical_placed')
        self.assertEqual(out['rows'][0]['receipt_digest'], receipt.raw[-32:].hex())
        self.assertEqual(out['rows'][1]['status'], 'constraints_met')
        self.assertEqual(out['next_step'], 'b')
        self.assertEqual(out['original_plan'], review.unpack()[3].json())
        self.assertFalse(out['rows'][0]['current_condition_proven'])
        self.assertFalse(out['rows'][0]['construction_completion_proven'])
        self.assertEqual(out, frontier.assess(review, BATCH, (receipt.raw,), before,
                                             (remaining,), {2: 2}, raw, current))
        self.assertLess(len(canonical(out)), frontier.MAX_OUTPUT)

    def test_uncertain_predecessor_never_unlocks_the_next_floor(self):
        review, receipt, before, remaining, raw, current = prefix()
        for phase, reason in [('indeterminate', 'native_failure'), ('prepared', 'none'),
                              ('cancelled', 'cancelled'), ('refused', 'stale')]:
            record = wire.Record(receipt.plan, phase, reason)
            with self.subTest(phase=phase), self.assertRaises(ValueError):
                frontier.assess(review, BATCH, (record.raw,), before, (remaining,), {2: 2}, raw, current)
        foreign = wire.Record(wire.Plan('other', receipt.plan.before), 'placed', 'none',
                              receipt.after, receipt.insertion)
        with self.assertRaises(ValueError):
            frontier.assess(review, BATCH, (foreign.raw,), before, (remaining,), {2: 2}, raw, current)

    def test_broken_stairs_after_a_placed_prefix_cannot_authorize_later_work(self):
        review, receipt, before, remaining, _, _ = prefix()
        raw, _ = terrain(changes={(4, 2, 2): (2, 3, 0, 1, 0, 1),
                                   (2, 2, 1): (2, 3, 0, 0, 0, 1)})
        current = source(raw, inventory='broken', horizons=(31, 21, 100))
        substitute = item(3, (5, 3, 2), kind='chair')
        out = frontier.assess(review, BATCH, (receipt.raw,), before,
                              (remaining, substitute), {2: 2, 3: 2}, raw, current)
        self.assertEqual(out['status'], 'route_blocked')
        self.assertEqual(out['rows'][1]['item'], remaining.id)
        self.assertEqual(out['rows'][1]['reason'], 'no_route_in_observed_model')
        self.assertFalse(out['reallocation_performed'])
        moved = replace(remaining, position=(5, 3, 0))
        with self.assertRaises(ValueError):
            frontier.assess(review, BATCH, (receipt.raw,), replace(before, item=replace(before.item, pos=moved.position)),
                            (moved,), {2: 2}, raw, current)


if __name__ == '__main__':
    unittest.main(verbosity=2)
