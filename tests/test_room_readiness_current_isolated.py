"""Actual current-query module with explicit validated-producer test doubles.

These tests do not execute DFHack codecs, journal custody or native transport.
The production-codec companion is test_room_readiness_current.py.
"""
from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass, field, replace
import hashlib
import itertools
import json
from pathlib import Path
from types import ModuleType, SimpleNamespace as NS
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import room_readiness_current as current


@dataclass(frozen=True)
class Finding:
    status: str = 'condition_met'
    stage: int | None = 3
    max_stage: int = 3
    building_type: int | None = 51


@dataclass(frozen=True)
class Assessment:
    building_id: int
    item_id: int
    job_id: int
    key: str
    receipt_digest: str
    condition: Finding

    def view(self):
        return {**self.__dict__, **self.condition.__dict__, 'condition': None}


@dataclass(frozen=True)
class Goal:
    count: int = 2
    require_circulation: bool = False

    def encode(self):
        return current._canonical({'count': self.count, 'require_circulation': self.require_circulation})

    @classmethod
    def decode(cls, raw, guard):
        guard()
        return cls(**json.loads(raw))

    @property
    def digest(self):
        return hashlib.sha256(self.encode()).hexdigest()

    @property
    def condition(self):
        return NS(deadline=100, goals=tuple(NS(
            record=NS(insertion=NS(building=10+i, item=20+i, job=30+i), plan=NS(key=f'key{i}')),
            receipt=bytes([i+1])*64) for i in range(self.count)))

    @property
    def room(self):
        return NS(room_plan=NS(digest='a'*64), allocation=NS(plan=lambda: NS(
            steps=tuple(NS(target=(i, 0, 0)) for i in range(self.count)))))


@dataclass(frozen=True)
class Progress:
    goal_digest: str
    phase: str = 'satisfied'
    source: tuple | None = (41, 17, 'df', 'dfhack')
    map_source: bytes | None = b'{"generation":29}'
    last_tick: int | None = 90
    horizons: tuple | None = (40, 40, 40)
    assessments: tuple = ()
    sample_digest: str | None = 'c'*64
    reading: bool = False
    streak: int = 2


class Sample:
    def __init__(self, goal, tick=110, *, statuses=None, terrain=None, occupancy=None,
                 circulation='true', generation=29, horizons=(40, 40, 40), valid=True):
        self.goal = goal
        self.tick = tick
        self.statuses = tuple(statuses or (Finding(),)*goal.count)
        self.circulation = circulation
        self.valid = valid
        self.categories = {k: 0 for k in ('matched', 'wrong_shape', 'liquid', 'designated',
                                        'occupied_wall', 'hidden', 'missing')}
        self.categories['matched'] = 10
        self.categories.update(terrain or {})
        self.occupancy = tuple(occupancy or ('true',)*goal.count)
        self.map_before = NS(generation=generation)
        self._observed = NS(tick=tick, digest='d'*64, horizons=horizons, statuses=self.statuses)
        self.furnishings = NS(before=NS(generation=41), operations=NS(generation=17, software=('df', 'dfhack')),
                              validate=self.validate_furnishings)

    def validate_furnishings(self, condition, guard):
        guard()
        if not self.valid:
            raise ValueError('synthetic producer refused')
        return self._observed

    def validate(self, goal, guard):
        self.validate_furnishings(goal.condition, guard)
        tiles = tuple(NS(presence=1 if v == 'unknown' else 2,
                         attributes=(0, 3, 0, 0, 0, 0, int(v == 'true'))) for v in self.occupancy)
        capture = NS(tick=self.tick, witness='b'*64, region=NS(origin=(0,0,0), size=(goal.count,1,1)),
                     tiles=tiles, binding=lambda: {'generation': self.map_before.generation})
        diagnosis = {'counts': {'floors': dict(self.categories), 'required_walls': dict(self.categories)},
                     'deficits': {'rows': [], 'omitted': 12},
                     'circulation': {'truth': self.circulation}}
        return capture, diagnosis

    @property
    def digest(self):
        return hashlib.sha256(current._canonical({'tick': self.tick, 'terrain': self.categories,
            'statuses': [s.status for s in self.statuses], 'occupancy': self.occupancy,
            'circulation': self.circulation})).hexdigest()


@contextmanager
def producers():
    readiness, construction = ModuleType('room_readiness'), ModuleType('construction_plan')
    readiness.Goal, readiness.Sample, readiness.Progress = Goal, Sample, Progress
    construction.Assessment = Assessment
    def assess(child, observed, guard):
        guard()
        return observed.statuses[child.record.insertion.building - 10]
    construction.assess = assess
    with patch.dict(sys.modules, room_readiness=readiness, construction_plan=construction):
        yield


class CurrentInspectionTests(unittest.TestCase):
    def setUp(self):
        self.context = producers()
        self.context.__enter__()
        self.addCleanup(self.context.__exit__, None, None, None)
        self.goal = Goal()
        self.prior = Progress(self.goal.digest)

    def inspect(self, sample=None, prior=None):
        return current.inspect(self.goal, sample or Sample(self.goal), baseline=prior or self.prior)

    def test_truth_algebra_all_27_inputs_and_invalid_values(self):
        for values in itertools.product(('false','unknown','true'), repeat=3):
            expected = 'false' if values.count('false') else 'unknown' if values.count('unknown') else 'true'
            self.assertEqual(current._all(values), expected)
        for bad in ((), ('maybe',), (True,)):
            self.assertRaises(ValueError, current._all, bad)

    def test_true_now_after_deadline_does_not_complete_or_change_history(self):
        before = repr(self.prior)
        out = self.inspect()
        self.assertEqual(out['status'], 'condition_met')
        self.assertGreater(out['game_tick'], self.goal.condition.deadline)
        self.assertEqual(repr(self.prior), before)
        for key in ('stability_evaluated','deadline_evaluated','monitor_updated','durable_sample_published',
                    'unresolved_read_cleared','placement_effect_reconciled','room_completion_proven','mutation_dispatched'):
            self.assertIs(out[key], False)

    def test_lost_wall_overrides_historical_satisfaction_with_full_counts(self):
        out = self.inspect(Sample(self.goal, terrain={'wrong_shape': 1}))
        self.assertEqual(out['status'], 'condition_not_met')
        self.assertEqual(out['domain_truth']['terrain'], 'false')
        self.assertEqual(out['historical_basis']['phase'], 'satisfied')
        self.assertEqual(len(out['assessments']), 2)

    def test_unknown_terrain_never_inferred_from_empty_deficit_preview(self):
        for category in ('hidden','missing'):
            out = self.inspect(Sample(self.goal, terrain={category: 1}))
            self.assertEqual(out['status'], 'blocked_unknown')
        out = self.inspect(Sample(self.goal, terrain={'hidden': 1,'liquid':1}))
        self.assertEqual(out['condition_truth'], 'false')

    def test_occupancy_unknown_and_false_remain_distinct(self):
        for value, status in (('unknown','blocked_unknown'),('false','condition_not_met')):
            out = self.inspect(Sample(self.goal, occupancy=('true',value)))
            self.assertEqual(out['status'], status)
            self.assertEqual(out['domain_truth']['furniture_occupancy'], value)

    def test_pending_and_uncertain_furnishings_do_not_become_success(self):
        for value in current.KNOWN_PENDING:
            out = self.inspect(Sample(self.goal, statuses=(Finding(), Finding(value))))
            self.assertEqual(out['condition_truth'], 'false')
        for value in ('item_unverified','future_unknown_status'):
            self.assertEqual(self.inspect(Sample(self.goal, statuses=(Finding(value), Finding())))['condition_truth'], 'unknown')

    def test_all_identity_failures_invalidate_even_after_decisive_false(self):
        for status in current.IDENTITY_FAILURES:
            out = self.inspect(Sample(self.goal, statuses=(Finding('pending'),Finding(status))))
            self.assertEqual(out['status'], 'invalidated_reference')
            self.assertEqual(out['condition_truth'], 'unknown')
            self.assertEqual(out['invalidators'][0]['building_id'], 11)
            self.assertEqual(out['selected_furnishings'], 2)

    def test_source_clock_and_horizon_floors_survive_read_only_checks(self):
        samples = [Sample(self.goal, generation=30), Sample(self.goal, tick=89),
                   Sample(self.goal, horizons=(39,40,40))]
        reasons = ['room_map_source_changed','game_clock_regressed','native_horizon_regressed']
        for sample, reason in zip(samples, reasons):
            out = self.inspect(sample)
            self.assertEqual(out['status'], 'invalidated_reference')
            self.assertIn(reason, [r['reason'] for r in out['invalidators']])
        source = replace(self.prior, source=(41,16,'df','dfhack'))
        self.assertEqual(self.inspect(prior=source)['status'], 'invalidated_reference')

    def test_terminal_states_and_unresolved_read_are_never_reopened(self):
        for phase in ('satisfied','failed','expired','cancelled','candidate','active'):
            prior = replace(self.prior, phase=phase, reading=True)
            out = self.inspect(prior=prior)
            self.assertEqual(out['status'], 'condition_met')
            self.assertTrue(out['historical_basis']['read_outcome_unknown'])
            self.assertTrue(prior.reading)
        prior = replace(self.prior, phase='invalidated')
        self.assertEqual(self.inspect(prior=prior)['status'], 'invalidated_reference')

    def test_prior_building_type_and_stage_cannot_be_forgotten(self):
        old = Assessment(10,20,30,'key0','e'*64, Finding(stage=3, building_type=52))
        prior = replace(self.prior, assessments=(old,))
        out = self.inspect(Sample(self.goal, statuses=(Finding(stage=2),Finding())), prior)
        self.assertEqual({r['reason'] for r in out['invalidators']},
                         {'native_building_type_changed','construction_stage_regressed'})

    def test_circulation_only_applies_to_original_opt_in(self):
        sample = Sample(self.goal, circulation='false')
        self.assertEqual(self.inspect(sample)['status'], 'condition_met')
        goal = Goal(require_circulation=True)
        for truth, status in (('true','condition_met'),('false','condition_not_met'),('unknown','blocked_unknown')):
            out = current.inspect(goal, Sample(goal, circulation=truth))
            self.assertEqual(out['status'], status)
            self.assertEqual(out['domain_truth']['circulation'], truth)

    def test_baseline_and_input_substitutions_refuse(self):
        self.assertRaises(ValueError, current.inspect, object(), Sample(self.goal))
        self.assertRaises(ValueError, current.inspect, self.goal, object())
        self.assertRaises(ValueError, current.inspect, self.goal, Sample(self.goal),
                          baseline=replace(self.prior, goal_digest='0'*64))
        self.assertRaises(ValueError, current.inspect, self.goal, Sample(self.goal, valid=False))

    def test_every_guard_interruption_leaves_original_history_untouched(self):
        calls = []
        sample = Sample(self.goal)
        current.inspect(self.goal, sample, baseline=self.prior, guard=lambda: calls.append(1))
        before = repr(self.prior)
        for boundary in range(len(calls)):
            remaining = [boundary]
            def guard():
                if remaining[0] == 0:
                    raise InterruptedError('stop')
                remaining[0] -= 1
            self.assertRaises(InterruptedError, current.inspect, self.goal, sample, baseline=self.prior, guard=guard)
            self.assertEqual(repr(self.prior), before)

    def test_output_bound_and_evidence_identity(self):
        out = self.inspect()
        self.assertEqual(out, self.inspect())
        digest = out.pop('evidence_digest')
        self.assertEqual(digest, hashlib.sha256(b'dfmcp-room-current-condition/1\0'+current._canonical(out)).hexdigest())
        self.assertNotEqual(digest, self.inspect(Sample(self.goal,tick=111))['evidence_digest'])
        with patch.object(current, 'MAX_OUTPUT', 1):
            self.assertRaises(ValueError, self.inspect)
        self.assertEqual(self.prior.phase, 'satisfied')

    def test_all_32_original_furnishings_survive_complete_report(self):
        goal = Goal(32, True)
        statuses = (Finding(),)*31 + (Finding('pending'),)
        out = current.inspect(goal, Sample(goal, statuses=statuses))
        self.assertEqual(out['condition_truth'], 'false')
        self.assertEqual(len(out['assessments']), 32)
        self.assertTrue(out['assessments_complete'])
        self.assertLess(len(current._canonical(out)), current.MAX_OUTPUT)


if __name__ == '__main__':
    unittest.main()
