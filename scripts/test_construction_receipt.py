"""Execute the real construction decoder/state machine, not a translated model."""
from __future__ import annotations
from dataclasses import replace
import hashlib
import json
from pathlib import Path
import struct
import unittest

import construction_receipt as c
from build_placement_wire import Rejected, Record

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / 'bridge/common/tests/fixtures/build_placement_v1_19.json'
VECTORS = json.loads(FIXTURE.read_text())
RECEIPT = bytes.fromhex(VECTORS['placed'])
TICK = 806500
U = lambda *v: struct.pack('>' + 'I' * len(v), *v)
I = lambda *v: struct.pack('>' + 'i' * len(v), *v)
TEXT = lambda v: struct.pack('>H', len(v.encode('utf-8'))) + v.encode('utf-8')
REF = lambda n: b'\0' if n is None else b'\1' + U(n)
GUARD = lambda: None


def job(identity=90, kind='ConstructBuilding', holder=70, suspended=False, count=0, filters=0):
    return (U(identity) + I({'ConstructBuilding': 1, 'DestroyBuilding': 2}.get(kind, 3))
            + TEXT(kind) + TEXT('') + bytes([suspended, False]) + I(15, 15, 2)
            + REF(None) + REF(holder) + I(-1) + U(count, filters))


def building(identity=70, kind='Bed', stage=1, maximum=1, bounds=(15, 15, 15, 15, 2), native_type=1):
    return U(identity) + I(native_type) + TEXT(kind) + I(*bounds, stage, maximum)


def item(identity=42, kind='BED', native_type=101, subtype=-1, material=419,
         material_index=-1, stack=1, flags=256, holder=70, container=None):
    return (U(identity) + I(native_type) + TEXT(kind) + I(subtype, material, material_index)
            + U(stack) + I(15, 15, 2) + U(flags) + REF(container) + REF(holder))


def operations(tick=TICK + 1, *, jobs=(), buildings=None, items=None, attachments=(),
               folder='region1', site=2, horizons=(91, 71, 43)):
    buildings = (building(),) if buildings is None else buildings
    items = (item(),) if items is None else items
    year, annual_tick = divmod(tick, 403200)
    jp = (b'DFMJ1200' + U(year, annual_tick) + b'\1' + I(site) + U(horizons[0])
          + TEXT(folder) + U(len(jobs)) + b''.join(jobs))
    return (b'DFMO1400' + U(len(jp)) + jp + U(*horizons[1:], len(buildings)) + b''.join(buildings)
            + U(len(items)) + b''.join(items) + U(len(attachments)) + b''.join(I(*a) for a in attachments))


def goal(**kwargs):
    return c.Goal(RECEIPT, kwargs.pop('deadline', TICK + 100), **kwargs)


def sample(raw=None, **kwargs):
    manifest = c.Manifest(41, 'test-df', 'test-dfhack')
    return c.LinkedSample(manifest, RECEIPT, replace(manifest, generation=987),
                          operations(**kwargs) if raw is None else raw, manifest, RECEIPT)


def step(state, g, s):
    return c.advance(c.begin_read(state), g, s, GUARD)


class OperationsTests(unittest.TestCase):
    def test_existing_receipt_fixture_identity_and_native_field_order(self):
        raw = FIXTURE.read_bytes()
        self.assertEqual(hashlib.sha1(b'blob ' + str(len(raw)).encode() + b'\0' + raw).hexdigest(),
                         '5a63fe10e7aa5b09827730e040ad08bc5bfaef96')
        observed = c.decode_operations(operations(), GUARD)
        self.assertEqual(observed.tick, TICK + 1)
        self.assertEqual(observed.horizons, (91, 71, 43))
        self.assertEqual(observed.buildings[70].bounds, (15, 15, 15, 15, 2))
        self.assertEqual(observed.items[42].material, Record.decode(RECEIPT).insertion.material)
        self.assertEqual(c.assess(goal(), observed, GUARD).status, 'condition_met')
        with self.assertRaises(TypeError):
            observed.buildings[9] = observed.buildings[70]

    def test_all_truncated_prefixes_and_trailing_bytes(self):
        payloads = [(operations(jobs=(job(count=1),), attachments=((90, 42, 0, -1),)),
                     lambda b: c.decode_operations(b, GUARD)),
                    (sample().encode(), c.LinkedSample.decode), (goal().encode(), c.Goal.decode)]
        for raw, decode in payloads:
            for end in range(len(raw)):
                with self.subTest(length=end, total=len(raw)), self.assertRaises(Rejected):
                    decode(raw[:end])
            with self.assertRaises(Rejected):
                decode(raw + b'\0')
            self.assertIsNotNone(decode(raw))

    def test_profile_lengths_counts_boolean_text_and_enum_contradictions(self):
        good = operations()
        malformed = [b'DFMO1300' + good[8:], good[:8] + U(2**32 - 1) + good[12:],
                     good[:28] + b'\2' + good[29:], good.replace(b'region1', b'regi\x00n1'),
                     operations(buildings=(building(), building())),
                     operations(buildings=(building(71),), horizons=(91, 71, 43)),
                     operations(items=(item(flags=512),)), operations(site=-1),
                     operations(buildings=(building(stage=2),)),
                     operations(buildings=(building(bounds=(16, 15, 15, 15, 2)),)),
                     operations(buildings=(building(), building(71, kind='Chair')), horizons=(91, 72, 43))]
        for raw in malformed:
            with self.subTest(raw=raw[:40]), self.assertRaises(Rejected):
                c.decode_operations(raw, GUARD)
        for raw in (bytearray(good), good * (c.MAX_CAPTURE // len(good) + 1)):
            with self.assertRaises(Rejected):
                c.decode_operations(raw, GUARD)

    def test_complete_roster_and_relationship_invariants(self):
        malformed = [
            operations(jobs=(job(holder=69),)), operations(items=(item(container=41),)),
            operations(items=(item(holder=69),)), operations(items=(item(container=42),)),
            operations(jobs=(job(count=1),)),
            operations(attachments=((90, 42, 0, -1),)),
            operations(jobs=(job(count=1),), attachments=((90, 43, 0, -1),)),
            operations(jobs=(job(count=2),), attachments=((90, 42, 0, -1), (90, 42, 0, -1))),
            operations(jobs=(job(count=1),), attachments=((90, 42, 0, 0),)),
            operations(jobs=(job(count=2),), attachments=((90, 42, 1, -1), (90, 42, 0, -1))),
        ]
        for raw in malformed:
            with self.subTest(size=len(raw)), self.assertRaises(Rejected):
                c.decode_operations(raw, GUARD)
        # Distinct roles remain distinct references but count one related job.
        obs = c.decode_operations(operations(jobs=(job(count=2),),
            attachments=((90, 42, 0, -1), (90, 42, 1, -1))), GUARD)
        self.assertEqual(c.assess(goal(), obs, GUARD).item_job_links, 1)

    def test_linear_container_walk_and_guard_cancellation(self):
        count = 4096
        records = tuple(item(identity=n, holder=None, container=n + 1 if n < count else None)
                        for n in range(1, count + 1))
        raw = operations(items=records, horizons=(91, 71, count + 1))
        calls = 0
        def guard():
            nonlocal calls
            calls += 1
        self.assertEqual(len(c.decode_operations(raw, guard).items), count)
        self.assertLess(calls, count * 4 + 20)
        def cancelled():
            raise TimeoutError('injected shared allowance exhausted')
        with self.assertRaises(TimeoutError):
            c.decode_operations(raw, cancelled)
        cyclic = records[:-1] + (item(identity=count, container=1, holder=None),)
        with self.assertRaises(Rejected):
            c.decode_operations(operations(items=cyclic, horizons=(91, 71, count + 1)), GUARD)

    def test_all_item_flag_words_and_exact_item_identity(self):
        for flags in range(512):
            finding = c.assess(goal(), c.decode_operations(operations(items=(item(flags=flags),)), GUARD), GUARD)
            expected = bool(flags & 256) and not flags & (2 | 8 | 64 | 128)
            self.assertEqual(finding.status == 'condition_met', expected, flags)
        for kwargs in ({'native_type': 102}, {'kind': 'CHAIR'}, {'subtype': 0},
                       {'material': 420}, {'material_index': 0}):
            finding = c.assess(goal(), c.decode_operations(operations(items=(item(**kwargs),)), GUARD), GUARD)
            self.assertEqual(finding.status, 'item_identity_mismatch')
        for kwargs in ({'stack': 0}, {'stack': 2}, {'holder': None}):
            finding = c.assess(goal(), c.decode_operations(operations(items=(item(**kwargs),)), GUARD), GUARD)
            self.assertEqual(finding.status, 'item_unverified')

    def test_building_footprint_stage_missing_and_reused_id(self):
        for kwargs in ({'kind': 'Chair'}, {'maximum': 2}, {'bounds': (16, 15, 16, 15, 2)},
                       {'bounds': (15, 15, 15, 15, 3)}):
            finding = c.assess(goal(), c.decode_operations(operations(buildings=(building(**kwargs),)), GUARD), GUARD)
            self.assertEqual(finding.status, 'building_identity_mismatch')
        self.assertEqual(c.assess(goal(), c.decode_operations(operations(buildings=(), items=()), GUARD), GUARD).status,
                         'building_missing')
        self.assertEqual(c.assess(goal(), c.decode_operations(operations(items=()), GUARD), GUARD).status, 'item_missing')
        self.assertEqual(c.assess(goal(), c.decode_operations(operations(buildings=(building(stage=0),)), GUARD), GUARD).status,
                         'no_construction_job')

    def test_all_jobs_count_even_after_example_limit(self):
        jobs = tuple(job(n, kind='Other', holder=70) for n in range(1, 20)) + (job(90, kind='DestroyBuilding'),)
        finding = c.assess(goal(), c.decode_operations(operations(jobs=jobs), GUARD), GUARD)
        self.assertEqual(finding.status, 'removal_pending')
        self.assertEqual(finding.removal_jobs, 1)
        for suspended in (False, True):
            finding = c.assess(goal(), c.decode_operations(operations(jobs=(job(suspended=suspended),)), GUARD), GUARD)
            self.assertEqual(finding.status, 'suspended' if suspended else 'pending')
        finding = c.assess(goal(), c.decode_operations(operations(jobs=(job(kind='Other'),)), GUARD), GUARD)
        self.assertEqual(finding.status, 'original_job_identity_mismatch')
        finding = c.assess(goal(), c.decode_operations(operations(jobs=(job(80, kind='Other', holder=None, count=1),),
            attachments=((80, 42, 0, -1),)), GUARD), GUARD)
        self.assertEqual(finding.status, 'item_unverified')


class LinkedGoalTests(unittest.TestCase):
    def test_strict_goal_and_manifest_bounds(self):
        for kwargs in ({'interval': True}, {'stable_samples': 1}, {'stable_span': 0},
                       {'max_gap': 0}, {'max_observations': 513}, {'deadline': TICK + 1},
                       {'deadline': c.MAX_TICK + 1}):
            with self.subTest(kwargs=kwargs), self.assertRaises(Rejected):
                goal(**kwargs)
        for name in ('prepared', 'expired', 'indeterminate', 'cancelled'):
            with self.assertRaises(Rejected):
                c.Goal(bytes.fromhex(VECTORS[name]), TICK + 100)
        for manifest in (c.Manifest(0, 'df', 'hack'), c.Manifest(41, '', 'hack'),
                         c.Manifest(41, 'a\0b', 'hack'), c.Manifest(41, 'df', 'x' * 129)):
            with self.assertRaises(Rejected):
                manifest.encode()
        self.assertEqual(c.Goal.decode(goal().encode()), goal())

    def test_receipt_and_source_bracket_cannot_be_substituted(self):
        value = sample()
        for forged in (replace(value, before_record=bytes.fromhex(VECTORS['prepared'])),
                       replace(value, after_record=bytes.fromhex(VECTORS['indeterminate'])),
                       replace(value, after=replace(value.after, generation=42)),
                       replace(value, before=replace(value.before, generation=42), after=replace(value.after, generation=42)),
                       replace(value, operations=replace(value.operations, df_version='other'))):
            with self.assertRaises(Rejected):
                forged.validate(goal(), GUARD)
        # Native generation namespaces are independent, not numerically equated.
        self.assertIsNotNone(value.validate(goal(), GUARD))
        self.assertNotEqual(value.before.generation, value.operations.generation)

    def test_distinct_advancing_samples_not_paused_replays(self):
        g = goal()
        state = step(c.Progress(g.digest), g, sample())
        self.assertEqual((state.phase, state.streak), ('candidate', 1))
        for _ in range(5):
            state = step(state, g, sample())
        self.assertEqual(state.streak, 1)
        state = step(state, g, sample(tick=TICK + 2))
        self.assertEqual(state.phase, 'satisfied')
        self.assertFalse(state.view()['placement_effect_discharged'])
        self.assertFalse(state.view()['continuous_stability_proven'])
        with self.assertRaises(Rejected):
            c.begin_read(state)
        self.assertEqual(c.cancel(state), state)

    def test_span_and_cadence_use_last_counted_sample(self):
        g = goal(interval=10, stable_span=20, stable_samples=3)
        state = c.Progress(g.digest)
        for tick in range(TICK + 1, TICK + 21):
            state = step(state, g, sample(tick=tick))
            self.assertFalse(state.terminal)
        state = step(state, g, sample(tick=TICK + 21))
        self.assertEqual((state.phase, state.streak), ('satisfied', 3))

    def test_negative_same_tick_gap_and_interruption_reset_stability(self):
        g = goal(max_gap=3)
        for mechanism in ('false', 'same_tick', 'gap', 'interrupted'):
            state = step(c.Progress(g.digest), g, sample())
            if mechanism == 'false':
                state = step(state, g, sample(tick=TICK + 2, items=(item(flags=258),)))
                final_tick = TICK + 3
            elif mechanism == 'same_tick':
                state = step(state, g, sample(tick=TICK + 1, horizons=(92, 71, 43)))
                final_tick = TICK + 2
            elif mechanism == 'gap':
                final_tick = TICK + 10
            else:
                state = c.begin_read(state)  # Crash after intent, no accepted sample.
                final_tick = TICK + 2
            extra = {'horizons': (92, 71, 43)} if mechanism == 'same_tick' else {}
            state = step(state, g, sample(tick=final_tick, **extra))
            self.assertEqual(state.phase, 'candidate', mechanism)
            self.assertEqual(state.streak, 1, mechanism)
            state = step(state, g, sample(tick=final_tick + 1, **extra))
            self.assertEqual(state.phase, 'satisfied', mechanism)

    def test_source_clock_horizon_and_type_changes_invalidate(self):
        g = goal(stable_samples=3)
        first = step(c.Progress(g.digest), g, sample(tick=TICK + 10, horizons=(100, 80, 50)))
        changes = [sample(tick=TICK + 9, horizons=(100, 80, 50)), sample(tick=TICK + 11),
                   sample(tick=TICK + 11, folder='different'),
                   sample(tick=TICK + 11, buildings=(building(native_type=2),), horizons=(100, 80, 50))]
        other = sample(tick=TICK + 11, horizons=(100, 80, 50))
        changes.append(replace(other, operations=replace(other.operations, generation=988)))
        for changed in changes:
            self.assertEqual(step(first, g, changed).phase, 'invalidated')

    def test_removal_failure_deadline_and_observation_budget(self):
        g = goal()
        state = c.Progress(g.digest)
        self.assertEqual(step(state, g, sample(jobs=(job(kind='DestroyBuilding'),))).phase, 'failed')
        self.assertEqual(step(state, g, sample(tick=g.deadline)).phase, 'expired')
        limited = goal(max_observations=2)
        state = step(c.Progress(limited.digest), limited, sample())
        state = step(state, limited, sample())
        self.assertEqual((state.phase, state.reason), ('expired', 'sample_budget_exhausted'))
        self.assertEqual(c.cancel(c.Progress(g.digest)).phase, 'cancelled')

    def test_stage_regression_and_full_replay_are_deterministic(self):
        record = Record.decode(RECEIPT)
        record = replace(record, insertion=replace(record.insertion, max_stage=3))
        g = c.Goal(record.raw, TICK + 100, stable_samples=2)
        samples = [sample(tick=TICK + n, buildings=(building(stage=stage, maximum=3),))
                   for n, stage in ((1, 1), (2, 2), (3, 3), (4, 3))]
        samples = [replace(s, before_record=g.receipt, after_record=g.receipt) for s in samples]
        def replay():
            state = c.Progress(g.digest)
            for s in samples:
                state = step(state, g, c.LinkedSample.decode(s.encode()))
            return state
        self.assertEqual(replay(), replay())
        self.assertEqual(replay().phase, 'satisfied')
        state = step(c.Progress(g.digest), g, samples[1])
        lower = replace(samples[0], capture=operations(tick=TICK + 3, buildings=(building(stage=1, maximum=3),)))
        self.assertEqual(step(state, g, lower).reason, 'construction_stage_regressed')


if __name__ == '__main__':
    unittest.main(verbosity=2)
