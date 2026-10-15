"""Strict circulation through production room codecs and private journal owners.

The native-format observations and placement records are fixtures, not a real
DFHack process. Run with PYTHONPATH=scripts:tests in a complete checkout.
"""
from dataclasses import replace
from pathlib import Path
import tempfile
import unittest

import furniture_batch
import construction_plan as c
import room_readiness as r
import room_readiness_store as store
import room_readiness_fixtures as f
from room_readiness_rpc import Budget
from test_room_readiness_store import create_room_batch, render
import track_room_readiness as cli


class StrictCirculationTests(unittest.TestCase):
    def goal(self, **timing):
        return replace(f.goal(**timing), require_circulation=True)

    def step(self, goal, tick=300, state=None, **changes):
        state = r.Progress(goal.digest) if state is None else state
        return r.advance(r.begin_read(state), goal, f.sample(goal, tick, **changes))

    def door(self, goal):
        return tuple(goal.room.room_plan.json()['areas'][0]['units'][0]['doorway'])

    def test_goal_versions_preserve_legacy_and_strict_semantics(self):
        legacy = f.goal()
        strict = replace(legacy, require_circulation=True)
        self.assertEqual(legacy.encode()[:8], b'DFMRDG01')
        self.assertEqual(strict.encode()[:8], b'DFMRDG02')
        self.assertEqual(legacy.encode()[8:], strict.encode()[8:])
        self.assertNotEqual(legacy.digest, strict.digest)
        for goal in (legacy, strict):
            restored = r.Goal.decode(goal.encode())
            self.assertEqual(restored.encode(), goal.encode())
            self.assertEqual(restored.require_circulation, goal.require_circulation)
        self.assertRaises(ValueError, replace, legacy, require_circulation=1)

    def test_failed_terrain_resets_joint_readiness_in_both_versions(self):
        for strict in (False, True):
            goal = replace(f.goal(), require_circulation=strict)
            wall = min(goal.terrain_goal()._selection.walls)
            state = self.step(goal, 310, self.step(goal), overrides={wall: f.tile(3)})
            self.assertEqual((state.phase, state.streak), ('active', 0))
            self.assertTrue(state.view()['furnishing_conditions_at_sample'])
            self.assertFalse(state.view()['room_readiness_sampled_condition'])

    def test_completed_furniture_with_a_blocked_door_does_not_complete_strict_goal(self):
        goal = self.goal()
        state = self.step(goal, 310, self.step(goal, overrides={self.door(goal): f.tile(3, building=1)}),
                          overrides={self.door(goal): f.tile(3, building=1)})
        detail = state.view()['terrain_at_last_observation']
        self.assertTrue(detail['all_required_shapes_at_sample'])
        self.assertTrue(state.view()['furnishing_conditions_at_sample'])
        self.assertEqual(detail['circulation']['truth'], 'false')
        self.assertEqual(detail['circulation']['selected_furnishings'], 6)
        self.assertEqual(len(detail['circulation']['records']), 7)
        self.assertEqual((state.phase, state.reason), ('active', 'original_room_circulation_not_ready'))
        self.assertFalse(state.view()['native_pathfinding_proven'])

    def test_legacy_goal_is_not_silently_upgraded_to_circulation(self):
        goal = f.goal()
        state = self.step(goal, 310, self.step(goal, overrides={self.door(goal): f.tile(3, building=1)}),
                          overrides={self.door(goal): f.tile(3, building=1)})
        self.assertEqual(state.phase, 'satisfied')
        self.assertNotIn('circulation', state.view()['terrain_at_last_observation'])
        self.assertNotIn('circulation_required', state.view())
        self.assertEqual(state.view()['policy'], r.POLICY)

    def test_unit_occupied_and_unobserved_doorways_are_potential_not_clear(self):
        goal = self.goal()
        for tile in (f.tile(3, units=1), b'\0', b'\1'):
            state = self.step(goal, 310, self.step(goal), overrides={self.door(goal): tile})
            detail = state.view()['terrain_at_last_observation']['circulation']
            self.assertEqual(detail['truth'], 'unknown')
            self.assertFalse(detail['all_required_routes_clear'])
            self.assertEqual(state.streak, 0)

    def test_blockage_resets_shared_window_and_later_clear_samples_can_finish(self):
        goal = self.goal()
        state = self.step(goal)
        self.assertEqual(state.streak, 1)
        state = self.step(goal, 310, state, overrides={self.door(goal): f.tile(3, building=1)})
        self.assertEqual(state.streak, 0)
        state = self.step(goal, 320, state)
        self.assertEqual((state.streak, state.first_tick), (1, 320))
        state = self.step(goal, 330, state)
        self.assertEqual(state.phase, 'satisfied')
        self.assertEqual(state.view()['policy'], r.CIRCULATION_POLICY)

    def test_rejected_last_sample_preserves_observation_exhaustion(self):
        goal = self.goal(max_observations=2)
        state = self.step(goal, 310, self.step(goal), overrides={self.door(goal): f.tile(3, units=1)})
        self.assertEqual((state.phase, state.reason), ('expired', 'sample_budget_exhausted'))

    def test_maximum_furnishing_selection_is_not_shortened(self):
        goal = replace(f.goal(True), require_circulation=True)
        state = self.step(goal)
        state = self.step(goal, 310, state)
        self.assertEqual(state.phase, 'satisfied')
        report = state.view()['terrain_at_last_observation']['circulation']
        self.assertEqual(report['selected_furnishings'], 32)
        self.assertEqual(len(report['records']), 33)
        self.assertTrue(report['records_complete'])

    def test_private_journal_reopen_preserves_strict_policy_and_current_custody(self):
        with tempfile.TemporaryDirectory(prefix='dfmcp-circulation-') as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            batch_path, journal_path = root/'batch', root/'readiness'
            origin = create_room_batch(batch_path)
            legacy = store.Goal(origin, c.Goal(origin.receipts, 10000, stable_span=10))
            goal = replace(legacy, require_circulation=True)
            self.assertEqual(goal.encode()[:8], b'DFMRDC02')
            self.assertEqual(goal.encode()[8:], legacy.encode()[8:])
            self.assertNotEqual(goal.digest, legacy.digest)
            for index, tick in enumerate((300, 310, 320)):
                budget = Budget(60000)
                with store.Journal(str(journal_path), budget, writable=True,
                                   create=(goal, origin.address) if index == 0 else None) as journal:
                    with furniture_batch.Batch(str(batch_path), budget) as batch:
                        journal.bind_batch(batch)
                        self.assertTrue(journal.state.goal.require_circulation)
                        self.assertTrue(journal.state.goal.readiness_goal.require_circulation)
                        journal.start_read()
                        changes = ({'overrides': {self.door(goal.readiness_goal): f.tile(3, building=1)}}
                                   if index == 0 else {})
                        journal.accept(f.sample(goal.readiness_goal, tick, **changes), render)
                        self.assertEqual(journal.state.progress.phase, ('active', 'candidate', 'satisfied')[index])
                        final = journal.state
            prior = journal_path.read_bytes()
            with store.Journal(str(journal_path), Budget(60000)) as journal:
                self.assertEqual(journal.state.goal.encode(), goal.encode())
                self.assertEqual(journal.state.progress.view(), final.progress.view())
            self.assertEqual(journal_path.read_bytes(), prior)
            self.assertEqual(cli.packet('inspect', final, source_verified=True)['result']['condition_policy'],
                             r.CIRCULATION_POLICY)
            self.assertFalse(cli.packet('inspect', final, source_verified=False)['result']['room_readiness_sampled_condition'])


if __name__ == '__main__':
    unittest.main()
