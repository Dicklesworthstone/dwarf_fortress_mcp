"""Executable whole-room temporal tests using independent native-format bytes."""
from dataclasses import replace
import json
import unittest

import excavation_observer as e
import room_terrain as terrain
import room_terrain_goal as g
from room_provisioning import RoomPlan
import room_terrain_fixtures as f


def sample(goal, tick=100, overrides=None, manifest=f.MANIFEST, **kwargs):
    selected = terrain.selection(goal.room_plan)
    tiles = {p: f.visible(3) for p in selected.floors}
    tiles.update(overrides or {})
    raw = f.capture(goal.region, tiles, tick=tick, **kwargs)
    return e.decode_capture(raw, manifest, goal.region)


class RoomTerrainGoalTests(unittest.TestCase):
    def setUp(self):
        self.goal = g.RoomTerrainGoal(f.plan(), 1000)
        self.selected = terrain.selection(self.goal.room_plan)
        self.floor = min(self.selected.floors)
        self.wall = min(self.selected.walls)

    def test_all_original_floors_and_walls_share_advancing_tick_streak(self):
        p = g.advance(self.goal, None, sample(self.goal))
        self.assertEqual((p.status, p.streak), ('stabilizing', 1))
        p = g.advance(self.goal, p, sample(self.goal, 100))
        self.assertEqual((p.status, p.streak), ('stabilizing', 1))
        p = g.advance(self.goal, p, sample(self.goal, 109))
        self.assertEqual((p.status, p.streak), ('stabilizing', 2))
        p = g.advance(self.goal, p, sample(self.goal, 110))
        self.assertEqual((p.status, p.streak, p.since_tick), ('satisfied', 3, 100))
        self.assertRaises(ValueError, g.advance, self.goal, p, sample(self.goal, 111))

    def test_original_wall_loss_blocks_even_when_every_floor_is_done(self):
        captured = sample(self.goal, overrides={self.wall: f.visible(3)})
        d = g.diagnose(self.goal, captured)
        self.assertEqual(d['counts']['floors']['matched'], len(self.selected.floors))
        self.assertEqual(d['counts']['required_walls']['wrong_shape'], 1)
        self.assertFalse(d['all_required_shapes_at_sample'])
        self.assertEqual(g.advance(self.goal, None, captured).status, 'pending')

    def test_completion_cannot_shrink_to_residual_or_separate_room_successes(self):
        a, b = sorted(self.selected.floors)[0], sorted(self.selected.floors)[-1]
        p = None
        for tick in range(100, 150):
            # Every target is good on some samples, never all targets together.
            p = g.advance(self.goal, p, sample(self.goal, tick, {a if tick % 2 else b: f.visible(2)}))
            self.assertEqual((p.status, p.streak), ('pending', 0))
        p = g.advance(self.goal, p, sample(self.goal, 150))
        self.assertEqual(p.since_tick, 150)
        p = g.advance(self.goal, p, sample(self.goal, 160))
        self.assertEqual(p.status, 'satisfied')

    def test_liquid_designation_hidden_missing_occupancy_have_explicit_counts(self):
        variants = [('liquid', f.visible(3, depth=1)), ('liquid', f.visible(3, magma=1)),
                    ('designated', f.visible(3, dig=1)), ('wrong_shape', f.visible(2)),
                    ('hidden', b'\x01'), ('missing', b'\x00')]
        for reason, tile in variants:
            with self.subTest(reason=reason, tile=tile):
                captured = sample(self.goal, overrides={self.floor: tile})
                d = g.diagnose(self.goal, captured)
                self.assertEqual(d['counts']['floors'][reason], 1)
                self.assertEqual(d['deficits']['count'], 1)
                self.assertEqual(g.advance(self.goal, None, captured).status,
                                 'unknown' if reason in ('hidden', 'missing') else 'pending')
        for kwargs in ({'building': 1}, {'units': 1}):
            captured = sample(self.goal, overrides={self.wall: f.visible(2, **kwargs)})
            self.assertEqual(g.diagnose(self.goal, captured)['counts']['required_walls']['occupied_wall'], 1)

    def test_occupied_floor_is_not_furniture_readiness_and_unused_holes_are_unknown(self):
        unused = set(f.coordinates(self.goal.region)) - self.selected.floors - self.selected.walls
        tiles = {p: b'\x01' for p in unused}
        tiles[self.floor] = f.visible(3, building=1, units=1)
        d = g.diagnose(self.goal, sample(self.goal, overrides=tiles))
        self.assertTrue(d['all_required_shapes_at_sample'])
        self.assertEqual(d['occupied_floor_tiles'], 1)
        self.assertEqual(d['unselected_tiles'], len(unused))
        self.assertFalse(d['unselected_cells_evaluated'])
        self.assertFalse(d['furniture_placement_eligibility_proven'])
        self.assertFalse(d['room_completion_proven'])

    def test_shared_walls_count_once_and_dining_has_no_invented_walls(self):
        shell_size = 2 * (3 + 2) + 2 * 3 - 1
        self.assertLess(len(self.selected.walls), 2 * shell_size)
        d = g.diagnose(self.goal, sample(self.goal))
        self.assertEqual(d['evaluated_tiles'], len(self.selected.floors | self.selected.walls))
        dining = g.RoomTerrainGoal(f.plan(f.dining()), 1000)
        d = g.diagnose(dining, sample(dining))
        self.assertEqual(d['required_wall_tiles'], 0)
        self.assertTrue(d['all_required_shapes_at_sample'])

    def test_multilevel_original_goals_do_not_evaluate_unselected_levels(self):
        goal = g.RoomTerrainGoal(f.plan(f.dining(1, 1), f.dining(1, 1, origin=(10, 10, 4), name='other')), 1000)
        selected = terrain.selection(goal.room_plan)
        tiles = {p: b'\x00' for p in f.coordinates(goal.region) if p[2] == 3}
        d = g.diagnose(goal, sample(goal, overrides=tiles))
        self.assertTrue(d['all_required_shapes_at_sample'])
        self.assertEqual(d['floor_tiles'], len(selected.floors))

    def test_failure_unknown_and_gap_each_reset_global_streak(self):
        for reset in ('read_failed', 'unfinished_read', 'unknown', 'pending', 'gap'):
            with self.subTest(reset=reset):
                goal = replace(self.goal, max_gap_ticks=15)
                p = g.advance(goal, None, sample(goal))
                if reset in ('read_failed', 'unfinished_read'):
                    p = p.interrupted(reset)
                elif reset in ('unknown', 'pending'):
                    p = g.advance(goal, p, sample(goal, 105, {self.wall: b'\x00' if reset == 'unknown' else f.visible(3)}))
                tick = 120 if reset == 'gap' else 110
                p = g.advance(goal, p, sample(goal, tick))
                self.assertEqual((p.status, p.streak, p.since_tick), ('stabilizing', 1, tick))
                self.assertEqual(g.advance(goal, p, sample(goal, tick + 10)).status, 'satisfied')

    def test_source_incarnation_dimensions_software_and_clock_changes_invalidate(self):
        p = g.advance(self.goal, None, sample(self.goal))
        changes = [dict(folder='other'), dict(site=3), dict(dimensions=(100, 100, 100)),
                   dict(manifest=e.Manifest(8, 'test-df', 'test-dfhack')),
                   dict(manifest=e.Manifest(7, 'changed-df', 'test-dfhack'))]
        for kwargs in changes:
            with self.subTest(kwargs=kwargs):
                result = g.advance(self.goal, p, sample(self.goal, 110, **kwargs))
                self.assertEqual(result.status, 'invalidated')
                self.assertEqual(result.latest.witness, p.latest.witness)
        self.assertEqual(g.advance(self.goal, p, sample(self.goal, 99)).status, 'invalidated')
        self.assertRaises(ValueError, g.advance, self.goal, None, sample(self.goal, folder='other'))

    def test_deadline_is_inclusive_and_never_overflows(self):
        goal = replace(self.goal, deadline_tick=110)
        p = g.advance(goal, None, sample(goal, 100))
        self.assertEqual(g.advance(goal, p, sample(goal, 110)).status, 'satisfied')
        self.assertEqual(g.advance(goal, p, sample(goal, 111)).status, 'expired')
        self.assertRaises(ValueError, g.advance, goal, None, sample(goal, 111))
        for value in (-1, e.MAX_TICK + 1, True):
            self.assertRaises(ValueError, replace, self.goal, deadline_tick=value)

    def test_closed_goal_preserves_full_intent_and_all_parameters_bind_digest(self):
        value = self.goal.json()
        restored = g.RoomTerrainGoal.from_json(json.loads(e.canonical(value)))
        self.assertEqual(restored.json(), value)
        self.assertEqual(restored.room_plan.encode(), self.goal.room_plan.encode())
        for key in ('deadline_tick', 'stable_ticks', 'required_samples', 'max_gap_ticks'):
            modified = {**value, key: value[key] + 1}
            self.assertNotEqual(g.RoomTerrainGoal.from_json(modified).digest, self.goal.digest)
            self.assertRaises(ValueError, g.RoomTerrainGoal.from_json, {**value, key: True})
        for modified in ({**value, 'extra': 1}, {**value, 'policy': 'anything'}, {**value, 'schema': 'anything'}):
            self.assertRaises(ValueError, g.RoomTerrainGoal.from_json, modified)

    def test_forged_compiled_room_outputs_and_caller_capture_fields_are_not_trusted(self):
        value = self.goal.room_plan.json()
        value.pop('plan_digest')
        value['areas'][0]['units'][0]['doorway'] = [10, 10, 2]
        forged = RoomPlan(e.canonical(value))
        self.assertRaises(ValueError, g.RoomTerrainGoal, forged, 1000)
        object.__setattr__(self.goal, 'room_plan', forged)
        self.assertRaises(ValueError, g.advance, self.goal, None, sample(g.RoomTerrainGoal(f.plan(), 1000)))
        goal = g.RoomTerrainGoal(f.plan(), 1000)
        original = sample(goal, overrides={self.wall: f.visible(3)})
        pretend = replace(original, tiles=sample(goal).tiles, tick=999, folder='pretend')
        self.assertEqual(g.advance(goal, None, pretend).status, 'pending')
        self.assertEqual(g.diagnose(goal, pretend)['observed_tick'], 100)
        self.assertRaises(ValueError, g.advance, goal, None, replace(original, raw=original.raw + b'x'))

    def test_diagnostic_bound_keeps_complete_counts_and_deterministic_order(self):
        goal = g.RoomTerrainGoal(f.plan(f.bedroom(10)), 1000)
        raw = f.capture(goal.region, {p: b'\x01' for p in f.coordinates(goal.region)})
        captured = e.decode_capture(raw, f.MANIFEST, goal.region)
        full = g.diagnose(goal, captured)
        narrow = g.diagnose(goal, captured, 0)
        self.assertEqual(len(full['deficits']['rows']), g.MAX_DEFICITS)
        self.assertEqual(full['deficits']['count'], full['evaluated_tiles'])
        self.assertEqual(full['deficits']['omitted'], full['evaluated_tiles'] - g.MAX_DEFICITS)
        self.assertEqual(narrow['counts'], full['counts'])
        self.assertEqual(narrow['deficits']['omitted'], full['evaluated_tiles'])
        self.assertEqual(full, g.diagnose(goal, captured))

    def test_every_guard_boundary_can_abort_without_publishing_transition(self):
        ticks = []
        captured = sample(self.goal)
        g.advance(self.goal, None, captured, lambda: ticks.append(1))
        class Stop(Exception):
            pass
        # Cover every guarded boundary on a representative complete room goal.
        for boundary in range(len(ticks)):
            count = [0]
            def guard():
                if count[0] == boundary:
                    raise Stop()
                count[0] += 1
            self.assertRaises(Stop, g.advance, self.goal, None, captured, guard)


if __name__ == '__main__':
    unittest.main()
