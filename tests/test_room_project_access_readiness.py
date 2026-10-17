"""Full existing raw-codec fixture regressions for entrance-aware room goals.

Requires the complete repository's ordinary readiness dependencies and fixtures.
Synthetic native responses only, not live fortress qualification.
"""
from dataclasses import replace
import hashlib
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import construction_plan as construction
import room_readiness as readiness
import room_readiness_current as current
from room_provisioning import RoomPlan
import room_readiness_fixtures as f


def goal(version=3, **options):
    plan = RoomPlan.compile({'schema': 'dfmcp.room-provisioning-request/1',
        'world_folder': 'region1', 'site': 2, 'connect_areas': True,
        'access': {'point': [10, 10, 2], 'region': {'origin': [10, 10, 2], 'size': [10, 6, 1]}},
        'areas': [{'name': 'hall', 'origin': [16, 10, 2],
                   'template': {'kind': 'dining_hall', 'table_count': 1}}]})
    handoff = f.handoff(plan)
    policy = dict(stable_samples=2, stable_span=10, interval=10)
    policy.update(options)
    return readiness.Goal(handoff, construction.Goal(f.receipts(handoff), 10000, **policy),
        require_circulation=version >= 2, require_project_access=version == 3)


def initial(g):
    return readiness.Progress(g.digest, circulation_required=g.require_circulation,
                              project_access_required=g.require_project_access)


def advance(state, g, tick, **options):
    return readiness.advance(readiness.begin_read(state), g, f.sample(g, tick, **options))


class ProjectAccessReadinessTests(unittest.TestCase):
    def test_inner_goal_versions_roundtrip_and_do_not_share_identity(self):
        goals = [goal(version) for version in (1, 2, 3)]
        self.assertEqual([g.encode()[:8] for g in goals], [b'DFMRDG01', b'DFMRDG02', b'DFMRDG03'])
        self.assertEqual(len({g.digest for g in goals}), 3)
        for g in goals:
            restored = readiness.Goal.decode(g.encode())
            self.assertEqual(restored, g)
            self.assertEqual(restored.encode(), g.encode())

    def test_added_corridor_building_cannot_complete_shared_access_stability(self):
        obstruction = {(13, 10, 2): f.tile(3, building=1)}
        legacy = goal(2)
        g = goal(3)
        # Both monitors earn one good sample; obstruction appears exactly when
        # the unchanged two-sample, ten-tick condition could otherwise finish.
        old = advance(advance(initial(legacy), legacy, 300), legacy, 310, overrides=obstruction)
        new = advance(advance(initial(g), g, 300), g, 310, overrides=obstruction)
        self.assertEqual(old.phase, 'satisfied')
        self.assertEqual(new.phase, 'active')
        self.assertEqual(new.streak, 0)
        self.assertEqual(new.reason, 'original_room_circulation_not_ready')

    def test_unknown_access_breaks_the_shared_streak_before_fresh_success(self):
        g = goal()
        state = advance(initial(g), g, 300)
        self.assertEqual(state.streak, 1)
        state = advance(state, g, 310, overrides={(13, 10, 2): f.tile(3, units=1)})
        self.assertEqual(state.streak, 0)
        self.assertNotEqual(state.phase, 'satisfied')
        state = advance(state, g, 320)
        self.assertEqual(state.streak, 1)
        self.assertNotEqual(state.phase, 'satisfied')
        state = advance(state, g, 330)
        self.assertEqual(state.phase, 'satisfied')
        self.assertTrue(state.view()['project_access_required'])

    def test_fresh_failed_access_does_not_rewrite_historical_success(self):
        g = goal()
        state = advance(advance(initial(g), g, 300), g, 310)
        self.assertEqual(state.phase, 'satisfied')
        before = state
        report = current.inspect(g, f.sample(g, 320, overrides={(13, 10, 2): f.tile(3, building=1)}), baseline=state)
        self.assertEqual(report['condition_truth'], 'false')
        self.assertEqual(report['domain_truth']['circulation'], 'false')
        self.assertEqual(state, before)
        self.assertFalse(report['monitor_updated'])

    def test_v3_cannot_be_created_without_original_access_or_circulation(self):
        old = f.goal()
        with self.assertRaises(ValueError):
            readiness.Goal(old.room, old.condition, require_circulation=True, require_project_access=True)
        g = goal()
        with self.assertRaises(ValueError): replace(g, require_circulation=False)

    def test_changing_scope_cannot_reuse_existing_progress_identity(self):
        g = goal()
        old = replace(g, require_project_access=False)
        self.assertNotEqual(g.digest, old.digest)
        with self.assertRaises(ValueError):
            readiness.advance(readiness.begin_read(initial(old)), g, f.sample(g, 300))

    def test_deadline_failure_precedes_access_pending(self):
        g = goal()
        state = advance(initial(g), g, 10001, overrides={(13, 10, 2): f.tile(3, building=1)})
        self.assertEqual(state.phase, 'expired')
        self.assertTrue(state.terminal)

    def test_query_and_joint_diagnosis_share_the_original_access_source(self):
        g = goal()
        sample = f.sample(g, 300)
        captured, diagnosis = sample.validate(g, lambda: None)
        access = diagnosis['circulation']
        self.assertEqual(access['access_point'], [10, 10, 2])
        self.assertEqual(access['map_observation_witness'], captured.witness)
        self.assertEqual(access['room_plan_digest'], g.room.room_plan.digest)
        self.assertEqual(current.inspect(g, sample)['condition_truth'], 'true')


    def test_maximum_eight_area_thirty_two_furnishing_goal_checks_entrance(self):
        areas = [{'name': f'hall{i}', 'origin': [12 + (i % 4) * 8, 12 + (i // 4) * 8, 2],
                  'template': {'kind': 'dining_hall', 'table_count': 2, 'columns': 2}}
                 for i in range(8)]
        plan = RoomPlan.compile({'schema': 'dfmcp.room-provisioning-request/1',
            'world_folder': 'region1', 'site': 2, 'connect_areas': True, 'areas': areas,
            'access': {'point': [10, 10, 2],
                       'region': {'origin': [10, 10, 2], 'size': [40, 20, 1]}}})
        handoff = f.handoff(plan)
        g = readiness.Goal(handoff, construction.Goal(f.receipts(handoff), 10000,
            interval=10, stable_span=10), require_circulation=True, require_project_access=True)
        self.assertEqual(readiness.Goal.decode(g.encode()), g)
        state = advance(advance(initial(g), g, 300), g, 310)
        self.assertEqual(state.phase, 'satisfied')
        routes = state.view()['terrain_at_last_observation']['circulation']
        self.assertEqual((routes['selected_areas'], routes['selected_furnishings']), (8, 32))
        self.assertEqual(len(routes['records']), 41)
        report = current.inspect(g, f.sample(g, 320,
            overrides={(10, 10, 2): f.tile(3, units=1)}), baseline=state)
        self.assertEqual(report['condition_truth'], 'unknown')
        self.assertEqual(state.phase, 'satisfied')

    def test_same_tick_obstruction_discards_credit_before_clear_observations(self):
        g = goal()
        state = advance(initial(g), g, 300)
        state = advance(state, g, 300, overrides={(13, 10, 2): f.tile(3, building=1)})
        self.assertEqual(state.streak, 0)
        state = advance(state, g, 310)
        self.assertEqual((state.phase, state.streak), ('candidate', 1))
        self.assertEqual(advance(state, g, 320).phase, 'satisfied')

    def test_interrupted_read_resumes_original_deadline_with_fresh_streak(self):
        g = goal()
        state = readiness.begin_read(advance(initial(g), g, 300))
        # Re-entering begin_read represents a retained but unpublished read.
        state = advance(state, g, 310)
        self.assertEqual((state.streak, state.interruptions), (1, 1))
        self.assertEqual(g.condition.deadline, 10000)
        self.assertEqual(advance(state, g, 320).phase, 'satisfied')

    def test_hidden_entrance_never_becomes_false_or_clear_in_current_query(self):
        g = goal()
        report = current.inspect(g, f.sample(g, 300, overrides={(10, 10, 2): b'\x01'}))
        self.assertEqual(report['condition_truth'], 'unknown')
        self.assertEqual(report['domain_truth']['terrain'], 'unknown')
        self.assertEqual(report['domain_truth']['circulation'], 'unknown')
        self.assertFalse(report['all_original_conditions_at_sample'])

    def test_raw_sample_substitution_refuses_without_changing_prior_progress(self):
        g = goal()
        state = advance(initial(g), g, 300)
        sample = f.sample(g, 310)
        for changed in (replace(sample, after_capture=sample.after_capture[:-1]),
                        replace(sample, map_after=replace(f.MAP, generation=30)),
                        replace(sample, furnishings=replace(sample.furnishings,
                            before_records=sample.furnishings.before_records[:-1]))):
            before = state.view()
            with self.assertRaises(ValueError):
                readiness.advance(readiness.begin_read(state), g, changed)
            self.assertEqual(state.view(), before)
            self.assertEqual(state.streak, 1)

    def test_source_and_clock_regression_are_terminal_not_new_access_streaks(self):
        g = goal()
        state = advance(initial(g), g, 300)
        for tick, options in ((299, {}), (310, {'map_manifest': replace(f.MAP, generation=30)})):
            invalid = advance(state, g, tick, **options)
            self.assertEqual(invalid.phase, 'invalidated')
            self.assertEqual(invalid.streak, 0)
            with self.assertRaises(ValueError):
                advance(invalid, g, 320)

    def test_sample_allowance_cannot_be_refilled_by_blocked_access(self):
        g = goal(max_observations=2)
        state = advance(initial(g), g, 300)
        state = advance(state, g, 310, overrides={(13, 10, 2): f.tile(3, units=1)})
        self.assertEqual((state.phase, state.reason), ('expired', 'sample_budget_exhausted'))
        self.assertEqual(state.observations, 2)
        with self.assertRaises(ValueError):
            advance(state, g, 320)

    def test_cancelled_access_goal_can_be_checked_without_restarting_monitor(self):
        g = goal()
        cancelled = readiness.cancel(advance(initial(g), g, 300))
        report = current.inspect(g, f.sample(g, 310), baseline=cancelled)
        self.assertEqual(report['condition_truth'], 'true')
        self.assertEqual(cancelled.phase, 'cancelled')
        self.assertFalse(report['stability_evaluated'])
        self.assertFalse(report['monitor_updated'])
        self.assertFalse(report['placement_effect_reconciled'])

if __name__ == '__main__':
    unittest.main()
