"""Combined temporal semantics using actual raw decoders, not fake conditions."""
from dataclasses import replace
import json
import unittest

import construction_plan as c
import room_readiness as r
import room_terrain
from room_provisioning import RoomPlan
from room_furniture_handoff import RoomFurnitureHandoff
import room_readiness_fixtures as f


class ReadinessTests(unittest.TestCase):
    def setUp(self):
        self.goal = f.goal()
        self.selected = room_terrain.selection(self.goal.room.room_plan)
        self.wall = min(self.selected.walls)
        self.floor = min(self.selected.floors)

    def advance(self, state=None, tick=300, **kwargs):
        state = state or r.Progress(self.goal.digest)
        return r.advance(r.begin_read(state), self.goal, f.sample(self.goal, tick, **kwargs))

    def test_goal_and_complete_sample_roundtrip_exactly(self):
        self.assertEqual(r.Goal.decode(self.goal.encode()).encode(), self.goal.encode())
        sample = f.sample(self.goal)
        self.assertEqual(r.Sample.decode(sample.encode()).encode(), sample.encode())
        self.assertEqual(self.advance().view(), self.advance().view())
        for raw in (self.goal.encode()[:-1], self.goal.encode() + b'x', b'bad'):
            self.assertRaises(ValueError, r.Goal.decode, raw)
        for raw in (sample.encode()[:-1], sample.encode() + b'x', b'bad'):
            self.assertRaises(ValueError, r.Sample.decode, raw)

    def test_every_original_receipt_required_and_subsets_rejected(self):
        condition = replace(self.goal.condition, receipts=self.goal.condition.receipts[:-1])
        self.assertRaises(ValueError, r.Goal, self.goal.room, condition)
        self.assertRaises(ValueError, c.Goal, self.goal.condition.receipts * 2, 10000)
        wrong = f.handoff(f.room(True))
        self.assertRaises(ValueError, r.Goal, wrong, self.goal.condition)

    def test_room_geometry_and_temporal_policy_bind_identity(self):
        request = self.goal.room.room_plan.json()['intent']
        request['areas'][0]['template']['room_size'][1] = 4
        plan = RoomPlan.compile(request)
        h = RoomFurnitureHandoff(plan, self.goal.room.allocation)
        different = r.Goal(h, self.goal.condition)
        self.assertNotEqual(different.digest, self.goal.digest)
        for field in ('deadline', 'interval', 'stable_samples', 'stable_span', 'max_gap', 'max_observations'):
            delta = -1 if field == 'max_observations' else 1
            changed = replace(self.goal.condition, **{field: getattr(self.goal.condition, field) + delta})
            self.assertNotEqual(r.Goal(self.goal.room, changed).digest, self.goal.digest)

    def test_shared_success_requires_advancing_ticks_and_full_span(self):
        p = self.advance()
        self.assertEqual((p.phase, p.streak), ('candidate', 1))
        p = self.advance(p, 300)
        self.assertEqual(p.streak, 1)
        p = self.advance(p, 309)
        self.assertEqual((p.phase, p.streak), ('candidate', 2))
        p = self.advance(p, 310)
        self.assertEqual((p.phase, p.streak), ('satisfied', 3))
        self.assertTrue(p.view()['room_readiness_sampled_condition'])
        self.assertRaises(ValueError, self.advance, p, 311)
        for name in ('room_completion_proven', 'room_assignments_observed', 'current_usability_proven',
                     'atomic_cross_profile_snapshot_proven', 'continuous_stability_proven', 'retry_placement_permitted'):
            self.assertIs(p.view()[name], False)

    def test_lost_wall_cannot_latch_earlier_terrain_or_furniture_success(self):
        p = self.advance()
        p = self.advance(p, 310, overrides={self.wall: f.tile(3)})
        self.assertEqual((p.phase, p.streak, p.reason), ('active', 0, 'original_room_terrain_not_ready'))
        self.assertTrue(p.view()['furnishing_conditions_at_sample'])
        self.assertEqual(p.view()['terrain_at_last_observation']['deficits']['count'], 1)
        p = self.advance(p, 320)
        self.assertEqual((p.streak, p.first_tick), (1, 320))
        self.assertEqual(self.advance(p, 330).phase, 'satisfied')

    def test_separately_successful_domains_do_not_combine(self):
        item = self.goal.room.allocation.plan().steps[0].item
        p = self.advance(pending_item=item)
        self.assertEqual(p.streak, 0)
        p = self.advance(p, 310, overrides={self.floor: f.tile(2)})
        self.assertEqual(p.streak, 0)
        p = self.advance(p, 320, pending_item=item)
        self.assertEqual(p.streak, 0)
        self.assertEqual(self.advance(p, 330).streak, 1)

    def test_map_failures_are_complete_diagnostics_not_false_completion(self):
        for raw in (b'\0', b'\1', f.tile(2), f.tile(3, depth=1), f.tile(3, magma=1), f.tile(3, dig=1)):
            with self.subTest(raw=raw):
                p = self.advance(overrides={self.floor: raw})
                self.assertEqual(p.streak, 0)
                self.assertFalse(p.view()['room_readiness_sampled_condition'])
        self.assertEqual(self.advance(overrides={self.floor: f.tile(3, building=1, units=1)}).streak, 1)

    def test_matching_raw_map_bracket_and_paused_cross_profile_tick_required(self):
        sample = f.sample(self.goal)
        variants = [replace(sample, after_capture=f.map_capture(self.goal, 301)),
                    replace(sample, map_after=replace(f.MAP, generation=30)),
                    replace(sample, furnishings=replace(sample.furnishings, capture=f.operations(self.goal, 301))),
                    replace(sample, furnishings=replace(sample.furnishings, capture=f.operations(self.goal, 300, paused=False)))]
        unpaused = f.map_capture(self.goal, 300, paused=False)
        variants.append(replace(sample, before_capture=unpaused, after_capture=unpaused))
        for changed in variants:
            self.assertRaises(ValueError, r.advance, r.begin_read(r.Progress(self.goal.digest)), self.goal, changed)

    def test_source_namespaces_remain_independent_and_drift_is_refused(self):
        self.assertEqual(len({f.MAP.generation, f.OPERATIONS.generation, f.BUILD.generation}), 3)
        self.assertEqual(self.advance().phase, 'candidate')
        sample = f.sample(self.goal)
        for changed in [replace(sample, map_before=replace(f.MAP, df_version='different')),
                        replace(sample, furnishings=replace(sample.furnishings,
                            operations=replace(f.OPERATIONS, generation=99)))]:
            self.assertRaises(ValueError, r.advance, r.begin_read(r.Progress(self.goal.digest)), self.goal, changed)
        for options in ({'dimensions': (127, 128, 16)}, {'folder': 'another'}, {'site': 3}):
            raw = f.map_capture(self.goal, 300, **options)
            self.assertRaises(ValueError, r.advance, r.begin_read(r.Progress(self.goal.digest)), self.goal,
                              replace(sample, before_capture=raw, after_capture=raw))

    def test_map_generation_change_between_complete_samples_invalidates(self):
        p = self.advance()
        p = self.advance(p, 310, map_manifest=replace(f.MAP, generation=30))
        self.assertEqual((p.phase, p.reason, p.streak), ('invalidated', 'room_map_source_changed', 0))

    def test_same_tick_changed_unselected_map_bytes_reset_stability(self):
        p = self.advance()
        unused = next(point for point in room_terrain.points(self.selected.region, lambda: None)
                      if point not in self.selected.floors | self.selected.walls)
        p = self.advance(p, 300, overrides={unused: b'\1'})
        self.assertEqual((p.streak, p.reason), (0, 'room_sample_changed_at_same_tick'))
        self.assertEqual(self.advance(p, 310).streak, 1)

    def test_interrupted_reads_and_gaps_reset_shared_window(self):
        p = self.advance()
        p = r.begin_read(r.begin_read(p))
        self.assertEqual((p.streak, p.interruptions), (0, 1))
        p = r.advance(p, self.goal, f.sample(self.goal, 310))
        self.assertEqual((p.streak, p.first_tick), (1, 310))
        p = self.advance(p, 2000)
        self.assertEqual((p.streak, p.first_tick), (1, 2000))

    def test_deadline_observation_bound_cancellation_and_terminal_immutability(self):
        p = self.advance()
        self.assertEqual(self.advance(p, 10000).phase, 'expired')
        stopped = r.cancel(p)
        self.assertEqual(stopped.phase, 'cancelled')
        self.assertEqual(r.cancel(stopped), stopped)
        self.assertRaises(ValueError, r.begin_read, stopped)
        goal = replace(self.goal, condition=replace(self.goal.condition, max_observations=2))
        p = r.advance(r.begin_read(r.Progress(goal.digest)), goal, f.sample(goal))
        p = r.advance(r.begin_read(p), goal, f.sample(goal, 301))
        self.assertEqual((p.phase, p.reason), ('expired', 'sample_budget_exhausted'))

    def test_removal_and_identity_failures_keep_existing_construction_semantics(self):
        p = self.advance(removal=True)
        self.assertEqual((p.phase, p.reason), ('failed', 'removal_pending'))
        p = self.advance(missing_building=2000)
        self.assertEqual((p.phase, p.reason), ('invalidated', 'building_missing'))
        p = self.advance()
        self.assertEqual(self.advance(p, 299).phase, 'invalidated')

    def test_guard_refusal_never_mutates_prior_progress(self):
        p = r.begin_read(r.Progress(self.goal.digest))
        before = p.view()
        calls = []
        sample = f.sample(self.goal)
        r.advance(p, self.goal, sample, lambda: calls.append(1))
        for boundary in (0, 1, 10, len(calls) // 2, len(calls) - 1):
            left = [boundary]
            def guard():
                if left[0] == 0:
                    raise ValueError('stop')
                left[0] -= 1
            self.assertRaises(ValueError, r.advance, p, self.goal, sample, guard)
            self.assertEqual(p.view(), before)

    def test_32_slots_646_exclusions_and_full_roster_use_one_condition(self):
        goal = f.goal(True)
        self.assertEqual(len(goal.room.allocation.request.excluded_items), 646)
        p = r.Progress(goal.digest)
        for tick in (300, 310):
            p = r.advance(r.begin_read(p), goal, f.sample(goal, tick, extra_items=2000))
        self.assertEqual((p.phase, len(p.assessments)), ('satisfied', 32))
        self.assertLess(len(json.dumps(p.view())), 65536)
        self.assertEqual(r.Goal.decode(goal.encode()).room.encode(), goal.room.encode())

    def test_installed_roster_cannot_override_missing_map_building_occupancy(self):
        target = self.goal.room.allocation.plan().steps[0].target
        for tile in (f.tile(3), f.tile(3, units=1)):
            with self.subTest(tile=tile):
                p = self.advance()
                p = self.advance(p, 310, overrides={target: tile})
                self.assertEqual((p.phase, p.streak), ('active', 0))
                self.assertTrue(p.view()['furnishing_conditions_at_sample'])
                diagnosis = p.view()['terrain_at_last_observation']
                self.assertTrue(diagnosis['all_required_shapes_at_sample'])
                self.assertFalse(diagnosis['furniture_occupancy']['all_targets_have_building_occupancy'])
                self.assertEqual(len(diagnosis['furniture_occupancy']['missing_targets']), 1)
                self.assertFalse(diagnosis['furniture_occupancy']['building_identity_from_map_proven'])
                p = self.advance(p, 320)
                self.assertEqual(p.streak, 1)
                self.assertEqual(self.advance(p, 330).phase, 'satisfied')


if __name__ == '__main__':
    unittest.main()
