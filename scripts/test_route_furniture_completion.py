"""Actual original-goal/receipt/operations codecs; injected parent, no native I/O."""
from dataclasses import replace
from types import SimpleNamespace
import struct
import unittest

import build_placement_wire as wire
import construction_plan as plan_condition
from construction_receipt import Manifest
from furniture_plan import FurniturePlan, Step
import route_furniture_completion as completion

BATCH_ID = 'a' * 64
REVIEW = 'b' * 64
BUILD = Manifest(7, 'test-df', 'test-dfhack')
OPS = Manifest(13, 'test-df', 'test-dfhack')


def fixture(count=2, origin='/private/original-route-batch'):
    steps, receipts = [], []
    for index in range(count):
        kind = index % 3 + 1
        name = f'furnishing-{index:02}'
        step = Step(name, wire.KINDS[kind], 10 + index, (2 + index, 2, 0),
                    () if index == 0 else (steps[-1].name,))
        steps.append(step)
        tile = wire.Tile(presence=2, tiletype=1, shape=3)
        item = wire.Item(presence=2, pos=(2 + index, 3, 0), kind=kind, native_type=kind + 20,
                         subtype=-1, material=419, material_index=-1, on_ground=True, ground=tile)
        before = wire.Capture(7, index, 10 + index, 2, (128, 128, 2), 100 + index, 200 + index,
                              index, 'region1', True, True, True,
                              wire.Selection(kind, step.item, *step.target), (tile,) * 9, item)
        native = wire.Plan('frt-' + BATCH_ID + '-' + name, before)
        insertion = wire.Insertion(100 + index, 200 + index, step.item, kind, step.target,
                                   419, -1, 0, 3, True, True, True, False)
        receipts.append(wire.Record(native, 'placed', 'none', before.expected_after(), insertion).raw)
    goal = completion.Goal(origin, BATCH_ID, REVIEW, FurniturePlan(tuple(steps)), BUILD, 13,
                           plan_condition.Goal(tuple(receipts), 1000))
    return goal


def number(value, signed=False):
    return value.to_bytes(4, 'big', signed=signed)


def text(value):
    raw = value.encode('utf-8')
    return len(raw).to_bytes(2, 'big') + raw


def operations(goal, tick=100, *, pending=(), removed=(), missing=(), flags=None,
               extra_items=0, changed_item=None):
    """Independent operations/1.4 encoder, no production encoder used."""
    records = [child.record for child in goal.condition.goals]
    jobs, buildings, items = [], [], []
    for i, r in enumerate(records):
        p = r.insertion
        kind = ('', 'Bed', 'Chair', 'Table')[p.kind]
        if i not in missing:
            buildings.append(number(p.building) + number(p.kind + 40) + text(kind)
                             + struct.pack('>iiiii', p.pos[0], p.pos[1], p.pos[0], p.pos[1], p.pos[2])
                             + number(1 if i in pending else 3) + number(3))
        item = r.plan.before.item
        holder = b'\0' if i in missing or i in pending else b'\1' + number(p.building)
        native_kind = item.native_type + (1 if changed_item == i else 0)
        items.append(number(p.item) + number(native_kind) + text(wire.KINDS[p.kind].upper())
                     + struct.pack('>iiiIiiiI', item.subtype, item.material, item.material_index, 1,
                                   *p.pos, flags if flags is not None else 64 if i in pending else 256)
                     + b'\0' + holder)
        if i in pending or i in removed:
            k = 'DestroyBuilding' if i in removed else 'ConstructBuilding'
            jobs.append(number(p.job) + number(91 if i in removed else 90) + text(k)
                        + text('') + b'\0\0' + struct.pack('>iii', *p.pos)
                        + b'\0\1' + number(p.building) + number(-1, True) + number(0) + number(0))
    for index in range(extra_items):
        items.append(number(1000 + index) + number(99) + text('BOULDER')
                     + struct.pack('>iiiIiiiI', -1, 0, -1, 1, 1, 1, 0, 64) + b'\0\0')
    raw_jobs = (b'DFMJ1200' + struct.pack('>IIBII', 0, tick, 1, 2, 500)
                + text('region1') + number(len(jobs)) + b''.join(jobs))
    return (b'DFMO1400' + number(len(raw_jobs)) + raw_jobs + number(500)
            + number(1001 + extra_items) + number(len(buildings)) + b''.join(buildings)
            + number(len(items)) + b''.join(items) + number(0))


def sample(goal, tick=100, **kw):
    return plan_condition.LinkedSample(BUILD, goal.condition.receipts, OPS,
                                      operations(goal, tick, **kw), BUILD, goal.condition.receipts)


def advance(state, goal, tick=100, **kw):
    return completion.advance(plan_condition.begin_read(state), goal, sample(goal, tick, **kw))


class Parent:
    """Injection of the existing Batch API; private ownership tested separately."""
    def __init__(self, goal):
        self.path, self.id, self.plan = goal.origin, goal.batch_id, goal.plan
        self.review = SimpleNamespace(digest=goal.review_digest)
        self.native = goal.build
        self.source = SimpleNamespace(inventory_generation=goal.operations_generation)
        self.budget = SimpleNamespace(work=lambda: None)
        self.receipts = goal.condition.receipts
        self.entries = list(range(len(goal.plan.steps)))
        self.status, self.registered, self.valid = 'all_placed', True, True
        self.checks = 0

    def check(self):
        self.checks += 1
        if not self.valid:
            raise ValueError('lost original custody')

    def audit(self):
        self.check()
        return {'status': self.status, 'pending_step': None,
                'steps': [{'registered': self.registered} for _ in self.plan.steps]}

    def placed_prefix(self):
        self.check()
        return self.receipts


class OriginalCompletionTests(unittest.TestCase):
    def test_complete_round_trip_preserves_plan_dependencies_and_fixed_timing(self):
        goal = fixture(3)
        raw = goal.encode()
        restored = completion.Goal.decode(raw)
        self.assertEqual(restored, goal)
        self.assertEqual(restored.encode(), raw)
        self.assertEqual(restored.digest, goal.digest)
        self.assertEqual(restored.plan.ordered[-1].after, ('furnishing-01',))
        self.assertEqual(restored.condition.deadline, 1000)

    def test_all_original_steps_are_required_and_not_replaceable_by_other_items(self):
        goal = fixture(3)
        for change in ('missing', 'extra', 'foreign_key', 'kind', 'item', 'target', 'chronology'):
            receipts = list(goal.condition.receipts)
            if change == 'missing':
                receipts.pop()
            elif change == 'extra':
                receipts.append(receipts[0])
            else:
                record = wire.Record.decode(receipts[-1])
                before, native = record.plan.before, record.plan
                if change == 'foreign_key':
                    native = replace(native, key='different-original-key')
                else:
                    s = before.selection
                    if change == 'kind':
                        s = replace(s, kind=1)
                        before = replace(before, item=replace(before.item, kind=1))
                    elif change == 'item':
                        s = replace(s, item=99)
                    elif change == 'target':
                        s = replace(s, x=80)
                    else:
                        before = replace(before, sequence=0)
                    before = replace(before, selection=s)
                    native = replace(native, before=before)
                insertion = replace(record.insertion, item=native.before.selection.item,
                                    kind=native.before.selection.kind, pos=native.before.selection.target)
                receipts[-1] = replace(record, plan=native, after=native.before.expected_after(), insertion=insertion).raw
            with self.subTest(change=change), self.assertRaises(ValueError):
                replace(goal, condition=plan_condition.Goal(tuple(receipts), 1000)).encode()

    def test_installed_all_together_and_two_advancing_samples_are_required(self):
        goal = fixture(3)
        state = plan_condition.Progress(goal.digest)
        state = advance(state, goal, 100, pending=(1,))
        self.assertEqual(state.phase, 'active')
        self.assertEqual(len(state.assessments), 3)
        state = advance(state, goal, 101)
        self.assertEqual((state.phase, state.streak), ('candidate', 1))
        state = advance(state, goal, 101)
        self.assertEqual(state.streak, 1)
        state = advance(state, goal, 102)
        self.assertEqual(state.phase, 'satisfied')
        self.assertFalse(state.view()['placement_effect_discharged'])
        self.assertFalse(state.view()['current_usability_proven'])

    def test_individual_success_is_not_latched_and_regression_is_not_hidden(self):
        goal = fixture()
        state = advance(plan_condition.Progress(goal.digest), goal, 100, pending=(1,))
        state = advance(state, goal, 101, pending=(0,))
        self.assertNotEqual(state.phase, 'satisfied')
        self.assertEqual(state.reason, 'construction_stage_regressed')

    def test_original_sources_are_required_on_the_first_sample(self):
        goal = fixture()
        state = plan_condition.begin_read(plan_condition.Progress(goal.digest))
        valid = sample(goal)
        for changed in (replace(valid, operations=replace(OPS, generation=14)),
                        replace(valid, operations=replace(OPS, df_version='changed')),
                        replace(valid, before=replace(BUILD, generation=8)),
                        replace(valid, after=replace(BUILD, dfhack_version='changed'))):
            with self.assertRaises(ValueError):
                completion.advance(state, goal, changed)

    def test_missing_or_removal_does_not_become_completion(self):
        goal = fixture()
        for kw, phase in (({'missing': (1,)}, 'invalidated'), ({'removed': (1,)}, 'failed')):
            state = advance(plan_condition.Progress(goal.digest), goal, **kw)
            self.assertEqual(state.phase, phase)
            self.assertEqual(len(state.assessments), 2)

    def test_installed_item_link_policy_is_enforced(self):
        goal = fixture()
        for flags in (0, 64, 128, 258, 264, 320):
            state = advance(plan_condition.Progress(goal.digest), goal, flags=flags)
            self.assertEqual(state.phase, 'active')
        state = advance(plan_condition.Progress(goal.digest), goal, changed_item=1)
        self.assertEqual(state.phase, 'invalidated')

    def test_inclusive_original_deadline_cannot_be_renewed_on_decode(self):
        goal = fixture()
        goal = completion.Goal.decode(goal.encode())
        state = advance(plan_condition.Progress(goal.digest), goal, 999)
        state = advance(state, goal, 1000)
        self.assertEqual(state.phase, 'expired')
        other = replace(goal, condition=replace(goal.condition, deadline=2000))
        with self.assertRaises(ValueError):
            advance(state, other, 1001)

    def test_interruptions_reset_pending_stability(self):
        goal = fixture()
        state = advance(plan_condition.Progress(goal.digest), goal, 100)
        state = plan_condition.begin_read(state)
        state = advance(state, goal, 101)
        self.assertEqual((state.phase, state.streak, state.interruptions), ('candidate', 1, 1))

    def test_terminal_history_is_not_reopened_by_a_fresh_sample(self):
        goal = fixture()
        state = advance(plan_condition.Progress(goal.digest), goal, 100)
        state = advance(state, goal, 101)
        with self.assertRaises(ValueError):
            advance(state, goal, 102)

    def test_all_prefix_truncations_and_trailing_bytes_refuse(self):
        raw = fixture(1).encode()
        for length in range(len(raw)):
            with self.subTest(length=length), self.assertRaises(ValueError):
                completion.Goal.decode(raw[:length])
        with self.assertRaises(ValueError):
            completion.Goal.decode(raw + b'x')

    def test_changed_goal_fields_change_the_commitment(self):
        goal = fixture()
        for name, value in (('origin', '/private/different'), ('review_digest', 'c' * 64),
                            ('build', replace(BUILD, df_version='different')),
                            ('operations_generation', 14)):
            self.assertNotEqual(replace(goal, **{name: value}).digest, goal.digest)
        for path in ('relative', '/', '/private/../batch', '/private//batch', '/private/batch/'):
            with self.assertRaises(ValueError):
                replace(goal, origin=path).encode()

    def test_full_batch_factory_and_historical_verification(self):
        goal = fixture(3)
        parent = Parent(goal)
        self.assertEqual(completion.from_batch(parent, 1000), goal)
        completion.verify_origin(goal, parent)
        parent.valid = False
        with self.assertRaises(ValueError):
            completion.verify_origin(goal, parent)

    def test_partial_unregistered_changed_or_foreign_parent_is_refused(self):
        goal = fixture()
        mutations = (('status', 'pending_recovery'), ('registered', False), ('entries', []),
                     ('receipts', goal.condition.receipts[:1]), ('id', 'c' * 64),
                     ('path', '/private/copy'))
        for key, value in mutations:
            parent = Parent(goal)
            setattr(parent, key, value)
            with self.subTest(key=key), self.assertRaises(ValueError):
                completion.verify_origin(goal, parent)
        for key, value in mutations[:4]:
            parent = Parent(goal)
            setattr(parent, key, value)
            with self.subTest(key=key), self.assertRaises(ValueError):
                completion.from_batch(parent, 1000)

    def test_maximum_original_steps_are_all_evaluated(self):
        goal = fixture(32)
        self.assertLessEqual(len(goal.encode()), completion.MAX_GOAL)
        state = advance(plan_condition.Progress(goal.digest), goal)
        self.assertEqual(len(state.assessments), 32)
        state = advance(state, goal, 101)
        self.assertEqual(state.phase, 'satisfied')

    def test_every_codec_guard_refusal_returns_no_goal(self):
        goal = fixture()
        calls = 0
        def count():
            nonlocal calls
            calls += 1
        raw = goal.encode()
        completion.Goal.decode(raw, count)
        for limit in range(1, calls + 1):
            seen = 0
            def guard():
                nonlocal seen
                seen += 1
                if seen == limit:
                    raise ValueError('cancelled')
            with self.assertRaises(ValueError):
                completion.Goal.decode(raw, guard)


if __name__ == '__main__':
    unittest.main()
