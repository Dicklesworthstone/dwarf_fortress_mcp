"""Exercise whole-selection construction evidence and its actual replay core."""
from __future__ import annotations

from dataclasses import replace
import itertools
import struct
import unittest
from unittest.mock import patch

import construction_plan as c
import construction_receipt as single
from build_placement_wire import Plan, Record, Rejected, field
from test_construction_receipt import (
    RECEIPT, TICK, VECTORS, GUARD, building, item, job, operations,
)


def make_receipt(index=0, kind=1, max_stage=1):
    """Independent canonical native record using the unchanged engine fixture."""
    original = Record.decode(RECEIPT)
    before = original.plan.before
    position = (15 + 3 * (index % 8), 15 + 3 * (index // 8), 2)
    selection = replace(before.selection, kind=kind, item=42 + index,
                        x=position[0], y=position[1], z=position[2])
    before = replace(before, sequence=before.sequence + 2 * index,
                     next_building=70 + index, next_job=90 + index,
                     building_count=before.building_count + index, selection=selection,
                     item=replace(before.item, kind=kind, native_type=100 + kind))
    insertion = replace(original.insertion, building=before.next_building, job=before.next_job,
                        item=selection.item, kind=kind, pos=position, max_stage=max_stage)
    return Record(Plan(f'construction-{index}', before), 'placed', 'none',
                  before.expected_after(), insertion).raw


def make_goal(count=2, **kwargs):
    receipts = tuple(make_receipt(n, kind=n % 3 + 1) for n in range(count))
    return c.Goal(receipts, kwargs.pop('deadline', TICK + 100), **kwargs)


def operations_for(goal, tick=TICK + 1, statuses=None, stages=None, **kwargs):
    """One coherent sorted native capture for all receipts, with per-row state.

    statuses/stages map canonical zero-based selection positions to overrides.
    Additional keyword arguments replace complete operations() components.
    """
    statuses, stages = statuses or {}, stages or {}
    buildings, items, jobs = [], [], []
    records = tuple(child.record for child in goal.goals)
    for index, record in enumerate(records):
        p, expected = record.insertion, record.plan.before.item
        status = statuses.get(index, 'condition_met')
        stage = stages.get(index, 0 if status in ('pending', 'suspended', 'no_construction_job') else p.max_stage)
        x, y, z = p.pos
        buildings.append(building(p.building, kind=('', 'Bed', 'Chair', 'Table')[p.kind],
                                  stage=stage, maximum=p.max_stage, bounds=(x, y, x, y, z),
                                  native_type=p.kind))
        items.append(item(p.item, kind=('', 'BED', 'CHAIR', 'TABLE')[p.kind],
                          native_type=expected.native_type, subtype=expected.subtype,
                          material=expected.material, material_index=expected.material_index,
                          holder=p.building, flags=258 if status == 'item_unverified' else 256))
        if status in ('pending', 'suspended', 'removal_pending'):
            jobs.append(job(p.job, kind='DestroyBuilding' if status == 'removal_pending' else 'ConstructBuilding',
                            holder=p.building, suspended=status == 'suspended'))
    parts = {'jobs': tuple(sorted(jobs, key=lambda raw: raw[:4])), 'buildings': tuple(buildings),
             'items': tuple(sorted(items, key=lambda raw: raw[:4])),
             'horizons': (max(r.after.next_job for r in records),
                          max(r.after.next_building for r in records),
                          max(r.insertion.item for r in records) + 1)}
    parts.update(kwargs)
    return operations(tick, **parts)


def sample_for(goal, tick=TICK + 1, **kwargs):
    manifest = c.Manifest(goal.goals[0].record.plan.before.generation, 'test-df', 'test-dfhack')
    return c.LinkedSample(manifest, goal.receipts, replace(manifest, generation=987),
                          operations_for(goal, tick, **kwargs), manifest, goal.receipts)


def step(state, goal, sample):
    return c.advance(c.begin_read(state), goal, sample, GUARD)


class PlanGoalTests(unittest.TestCase):
    def test_canonical_order_and_domain_separation(self):
        goal = make_goal(3)
        for order in itertools.permutations(goal.receipts):
            other = replace(goal, receipts=order)
            self.assertEqual(other, goal)
            self.assertEqual(other.digest, goal.digest)
            self.assertEqual(other.encode(), goal.encode())
        self.assertEqual(c.Goal.decode(goal.encode()), goal)
        self.assertNotEqual(c.Goal((goal.receipts[0],), goal.deadline).digest, goal.goals[0].digest)
        wrong_order = (b'DFMCPG01' + bytes([len(goal.receipts)])
                       + b''.join(field(raw) for raw in reversed(goal.receipts))
                       + c.TIMING.pack(goal.deadline, goal.interval, goal.stable_samples,
                                       goal.stable_span, goal.max_gap, goal.max_observations))
        with self.assertRaises(Rejected):
            c.Goal.decode(wrong_order)

    def test_all_selection_bounds_and_original_receipt_validation(self):
        for count in (1, 2, 32):
            goal = make_goal(count)
            self.assertEqual(len(goal.goals), count)
            self.assertLessEqual(len(goal.encode()), c.MAX_GOAL)
            self.assertEqual(c.Goal.decode(goal.encode()), goal)
        for receipts in ((), [], [make_receipt()], tuple(make_receipt(n) for n in range(33)),
                         (bytearray(make_receipt()),), (b'x' * 6145,),
                         (bytes.fromhex(VECTORS['prepared']),),
                         (bytes.fromhex(VECTORS['indeterminate']),)):
            with self.subTest(count=len(receipts)), self.assertRaises(Rejected):
                c.Goal(receipts, TICK + 100)
        goal = make_goal()
        for policy in ({'stable_samples': True}, {'stable_samples': 1}, {'stable_span': 0},
                       {'interval': 0}, {'max_gap': 0}, {'max_observations': 513},
                       {'deadline': TICK + 1}):
            with self.subTest(policy=policy), self.assertRaises(Rejected):
                replace(goal, **policy)
        altered = bytearray(goal.receipts[0])
        altered[-1] ^= 1
        with self.assertRaises(Rejected):
            replace(goal, receipts=(bytes(altered), goal.receipts[1]))

    def test_duplicate_keys_and_native_targets_are_rejected(self):
        goal = make_goal()
        first, second = (child.record for child in goal.goals)

        def rebuild(before=None, key=None):
            before = second.plan.before if before is None else before
            p = replace(second.insertion, building=before.next_building, job=before.next_job,
                        item=before.selection.item, pos=before.selection.target)
            return Record(Plan(second.plan.key if key is None else key, before), 'placed', 'none',
                          before.expected_after(), p).raw

        changed = [goal.receipts[0], rebuild(key=first.plan.key),
                   rebuild(replace(second.plan.before, next_building=first.insertion.building)),
                   rebuild(replace(second.plan.before, next_job=first.insertion.job)),
                   rebuild(replace(second.plan.before, selection=replace(second.plan.before.selection,
                                                                       item=first.insertion.item))),
                   rebuild(replace(second.plan.before, selection=replace(second.plan.before.selection,
                                                                       x=first.insertion.pos[0],
                                                                       y=first.insertion.pos[1],
                                                                       z=first.insertion.pos[2])))]
        for raw in changed:
            with self.subTest(receipt=raw[:20]), self.assertRaises(Rejected):
                replace(goal, receipts=(goal.receipts[0], raw))

    def test_receipts_must_share_fortress_generation_and_dimensions(self):
        goal = make_goal()
        second = goal.goals[1].record
        for changed in ({'generation': 42}, {'site': 3}, {'folder': 'another-world'},
                        {'dimensions': (65, 64, 8)}):
            before = replace(second.plan.before, **changed)
            record = Record(Plan(second.plan.key, before), 'placed', 'none',
                            before.expected_after(), second.insertion)
            with self.subTest(changed=changed), self.assertRaises(Rejected):
                replace(goal, receipts=(goal.receipts[0], record.raw))

    def test_goal_and_sample_codecs_refuse_all_truncated_prefixes_and_other_generations(self):
        goal = make_goal()
        sample = sample_for(goal)
        for raw, decode in ((goal.encode(), c.Goal.decode), (sample.encode(), c.LinkedSample.decode)):
            for end in range(len(raw)):
                with self.subTest(length=end, total=len(raw)), self.assertRaises(Rejected):
                    decode(raw[:end])
            with self.assertRaises(Rejected):
                decode(raw + b'\0')
            self.assertIsNotNone(decode(raw))
        with self.assertRaises(Rejected):
            c.Goal.decode(goal.goals[0].encode())
        with self.assertRaises(Rejected):
            single.Goal.decode(c.Goal((goal.receipts[0],), goal.deadline).encode())


class WholeSelectionTests(unittest.TestCase):
    def test_single_complete_capture_is_shared_across_every_assessment(self):
        goal = make_goal(32)
        sample = sample_for(goal)
        with patch.object(c, 'decode_operations', wraps=c.decode_operations) as decoder:
            state = step(c.Progress(goal.digest), goal, c.LinkedSample.decode(sample.encode()))
        self.assertEqual(decoder.call_count, 1)
        self.assertEqual((state.phase, state.streak, len(state.assessments)), ('candidate', 1, 32))
        view = state.view()
        self.assertTrue(view['assessments_complete'])
        self.assertEqual(view['condition_met_count'], 32)
        self.assertEqual([row['building_id'] for row in view['assessments']], list(range(70, 102)))
        state = step(state, goal, sample_for(goal, tick=TICK + 2))
        self.assertEqual(state.phase, 'satisfied')
        self.assertFalse(state.view()['placement_effect_discharged'])

    def test_different_buildings_succeeding_at_different_times_never_latch(self):
        goal = make_goal(3)
        state = c.Progress(goal.digest)
        for offset, failures in enumerate(({0: 'item_unverified'}, {1: 'item_unverified'},
                                            {2: 'item_unverified'}, {0: 'item_unverified'}), 1):
            state = step(state, goal, sample_for(goal, tick=TICK + offset, statuses=failures))
            self.assertEqual((state.phase, state.streak), ('active', 0))
            self.assertEqual(state.view()['condition_met_count'], 2)
        state = step(state, goal, sample_for(goal, tick=TICK + 5))
        self.assertEqual((state.phase, state.streak), ('candidate', 1))
        state = step(state, goal, sample_for(goal, tick=TICK + 6, statuses={2: 'item_unverified'}))
        self.assertEqual((state.phase, state.streak), ('active', 0))
        for offset in (7, 8):
            state = step(state, goal, sample_for(goal, tick=TICK + offset))
        self.assertEqual(state.phase, 'satisfied')

    def test_late_target_failure_is_not_hidden_by_first_pending_or_satisfied_row(self):
        goal = make_goal(32)
        for statuses in ({31: 'removal_pending'}, {0: 'pending', 31: 'removal_pending'}):
            state = step(c.Progress(goal.digest), goal, sample_for(goal, statuses=statuses))
            self.assertEqual((state.phase, state.reason_building), ('failed', 101))
            self.assertEqual(len(state.assessments), 32)
            self.assertEqual(state.assessments[-1].condition.removal_jobs, 1)

    def test_every_original_receipt_and_bracket_must_match(self):
        goal = make_goal(3)
        value = sample_for(goal)
        substitutes = [replace(value, before_records=goal.receipts[:-1]),
                       replace(value, after_records=goal.receipts[:-1]),
                       replace(value, before_records=tuple(reversed(goal.receipts))),
                       replace(value, after_records=(goal.receipts[0],) * 3),
                       replace(value, after=replace(value.after, generation=42)),
                       replace(value, operations=replace(value.operations, df_version='different'))]
        for bad in substitutes:
            with self.subTest(sample=bad.before.generation), self.assertRaises(Rejected):
                step(c.Progress(goal.digest), goal, bad)
        # Generation 987 belongs to operations, not the furniture generation 41.
        self.assertIsNotNone(value.validate(goal, GUARD))

    def test_every_target_retains_stage_and_type_regressions(self):
        receipts = tuple(make_receipt(n, kind=n % 3 + 1, max_stage=3) for n in range(3))
        goal = c.Goal(receipts, TICK + 100)
        first = step(c.Progress(goal.digest), goal,
                     sample_for(goal, stages={0: 1, 1: 2, 2: 2}, statuses={0: 'pending'}))
        result = step(first, goal, sample_for(goal, tick=TICK + 2, stages={0: 2, 1: 2, 2: 1}))
        self.assertEqual((result.phase, result.reason, result.reason_building),
                         ('invalidated', 'construction_stage_regressed', 72))
        goal = make_goal(3, stable_samples=3)
        first = step(c.Progress(goal.digest), goal, sample_for(goal))
        changed = sample_for(goal, tick=TICK + 2)
        raw = changed.capture.replace(struct.pack('>IiH', 72, 3, len('Table')) + b'Table',
                                      struct.pack('>IiH', 72, 4, len('Table')) + b'Table')
        result = step(first, goal, replace(changed, capture=raw))
        self.assertEqual((result.phase, result.reason, result.reason_building),
                         ('invalidated', 'native_building_type_changed', 72))

    def test_late_continuity_invalidation_precedes_early_removal_failure(self):
        receipts = tuple(make_receipt(n, kind=n % 3 + 1, max_stage=3) for n in range(3))
        goal = c.Goal(receipts, TICK + 100)
        first = step(c.Progress(goal.digest), goal, sample_for(goal, stages={0: 2, 1: 2, 2: 2}))
        changed = sample_for(goal, tick=TICK + 2, statuses={0: 'removal_pending'},
                             stages={0: 3, 1: 3, 2: 1})
        result = step(first, goal, changed)
        self.assertEqual((result.phase, result.reason, result.reason_building),
                         ('invalidated', 'construction_stage_regressed', 72))
        self.assertEqual(result.assessments[0].condition.status, 'removal_pending')
        changed = sample_for(goal, tick=TICK + 2, statuses={0: 'removal_pending'})
        raw = changed.capture.replace(struct.pack('>IiH', 72, 3, len('Table')) + b'Table',
                                      struct.pack('>IiH', 72, 4, len('Table')) + b'Table')
        result = step(first, goal, replace(changed, capture=raw))
        self.assertEqual((result.phase, result.reason, result.reason_building),
                         ('invalidated', 'native_building_type_changed', 72))
        self.assertEqual(len(result.assessments), 3)

    def test_missing_late_item_or_building_invalidates_whole_goal(self):
        goal = make_goal(3)
        observed = single.decode_operations(operations_for(goal), GUARD)
        # Omit the final row entirely, retaining a valid complete roster.
        short = make_goal(2)
        raw = operations_for(short, horizons=observed.horizons)
        result = step(c.Progress(goal.digest), goal, replace(sample_for(goal), capture=raw))
        self.assertEqual((result.phase, result.reason_building), ('invalidated', 72))
        self.assertEqual(len(result.assessments), 3)

    def test_global_cadence_span_duplicates_and_interrupted_reads(self):
        goal = make_goal(3, interval=5, stable_span=10, stable_samples=3)
        state = step(c.Progress(goal.digest), goal, sample_for(goal))
        for tick in (TICK + 1, TICK + 2, TICK + 5):
            state = step(state, goal, sample_for(goal, tick=tick))
        self.assertEqual(state.streak, 1)
        state = step(state, goal, sample_for(goal, tick=TICK + 6))
        self.assertEqual(state.streak, 2)
        state = c.begin_read(state)
        state = step(state, goal, sample_for(goal, tick=TICK + 11))
        self.assertEqual((state.streak, state.interruptions), (1, 1))
        for tick in (TICK + 16, TICK + 21):
            state = step(state, goal, sample_for(goal, tick=tick))
        self.assertEqual(state.phase, 'satisfied')

    def test_source_horizons_clock_gap_and_same_tick_changes(self):
        goal = make_goal(3, stable_samples=3, max_gap=3)
        first = step(c.Progress(goal.digest), goal,
                     sample_for(goal, tick=TICK + 10, horizons=(120, 110, 100)))
        for sample in (sample_for(goal, tick=TICK + 9, horizons=(120, 110, 100)),
                       sample_for(goal, tick=TICK + 11),
                       replace(sample_for(goal, tick=TICK + 11, horizons=(120, 110, 100)),
                               operations=c.Manifest(988, 'test-df', 'test-dfhack'))):
            self.assertEqual(step(first, goal, sample).phase, 'invalidated')
        for tick in (TICK + 10, TICK + 14):
            state = step(first, goal, sample_for(goal, tick=tick, horizons=(121, 110, 100)))
            self.assertLessEqual(state.streak, 1)
            self.assertFalse(state.terminal)

    def test_terminal_budget_cancellation_and_transition_guards(self):
        goal = make_goal(2, max_observations=2)
        state = step(c.Progress(goal.digest), goal, sample_for(goal))
        exhausted = step(state, goal, sample_for(goal))
        self.assertEqual((exhausted.phase, exhausted.reason), ('expired', 'sample_budget_exhausted'))
        deadline = step(c.Progress(goal.digest), goal, sample_for(goal, tick=goal.deadline))
        self.assertEqual(deadline.reason, 'game_deadline_reached')
        for terminal in (exhausted, deadline, c.cancel(state)):
            with self.assertRaises(Rejected):
                c.begin_read(terminal)
            self.assertEqual(c.cancel(terminal), terminal)
        with self.assertRaises(Rejected):
            c.advance(c.Progress(goal.digest), goal, sample_for(goal), GUARD)
        with self.assertRaises(Rejected):
            step(c.Progress(make_goal(3).digest), goal, sample_for(goal))

    def test_cancellation_is_checked_during_complete_selection(self):
        goal = make_goal(32)
        calls = 0
        def guard():
            nonlocal calls
            calls += 1
            if calls == 180:
                raise TimeoutError('bounded whole-plan work exhausted')
        with self.assertRaises(TimeoutError):
            c.advance(c.begin_read(c.Progress(goal.digest)), goal, sample_for(goal), guard)
        self.assertEqual(calls, 180)

    def test_full_replay_and_whole_target_evidence_are_deterministic(self):
        goal = make_goal(3, stable_samples=3)
        samples = [sample_for(goal, tick=TICK + n, statuses={1: 'item_unverified'} if n == 2 else {})
                   for n in range(1, 6)]
        def replay():
            value = c.Goal.decode(goal.encode())
            state = c.Progress(value.digest)
            for sample in samples:
                state = step(state, value, c.LinkedSample.decode(sample.encode()))
            return state
        self.assertEqual(replay(), replay())
        self.assertEqual(replay().phase, 'satisfied')


if __name__ == '__main__':
    unittest.main(verbosity=2)
