"""Original room readiness through real CLI processes and query-only TCP.

The original batch and child journals use the real private custody owners and
production receipt codec. Map and operations bytes are independent synthetic
fixtures; these tests neither run DFHack nor perform native game writes.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

from dataclasses import replace
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

import build_placement_rpc as placement_rpc
import build_placement_wire as placement_wire
import construction_plan as construction
import furniture_batch as batch_module
import furniture_completion as completion
import room_readiness as readiness
import room_readiness_fixtures as fixtures
import room_readiness_rpc as rpc
from room_readiness_peer import BUILD_TOKEN, OPS_TOKEN, MAP_TOKEN, PROFILES, Peer
import room_terrain

ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / 'scripts' / 'track_room_readiness.py'
SOURCE = placement_rpc.Manifest(fixtures.BUILD.generation, *fixtures.SOFTWARE)
READ_METHODS = {
    ('build', 'Handshake'), ('build', 'QueryPlacement'),
    ('operations', 'Handshake'), ('operations', 'ReadObservation'),
    ('map', 'Handshake'), ('map', 'ReadObservation'),
}


def create_original_batch(path: Path, peer: Peer, *, placed: int | None = None,
                          registered: int | None = None) -> str:
    """Create replayable original batch/3 custody without any native calls."""
    handoff = peer.goal.room
    plan = handoff.allocation.plan()
    path.mkdir(mode=0o700)
    (path / 'effects').mkdir(mode=0o700)
    first = placement_wire.Record.decode(fixtures.receipts(handoff)[0]).plan.before
    manifest = {
        'schema': batch_module.ROOM_SCHEMA, 'nonce': 'ab' * 24,
        'plan': plan.json(), 'room_handoff': handoff.json(),
        'source': SOURCE.view(), 'endpoint': peer.address_text,
        'folder': 'region1', 'site': 2, 'dimensions': list(fixtures.DIMENSIONS),
        'first_tick': first.tick,
        'root_identity': [path.stat().st_dev, path.stat().st_ino],
        'effects_identity': [(path / 'effects').stat().st_dev, (path / 'effects').stat().st_ino],
    }
    budget = placement_rpc.Budget(60000)
    with batch_module.root_lock(str(path), budget) as root:
        for name, raw in (('batch.json', batch_module.seal(manifest)),
                          ('steps.jsonl', batch_module.HEADER)):
            owner = batch_module.File(root, name, budget, True, raw,
                                      maximum=(batch_module.MAX_ROOM_DEFINITION if name == 'batch.json'
                                               else batch_module.MAX_FILE))
            owner.close()
    placed = len(plan.steps) if placed is None else placed
    registered = placed if registered is None else registered
    originals = fixtures.receipts(handoff)
    with batch_module.Batch(str(path), budget, True) as owner:
        for index, step in enumerate(plan.ordered[:placed]):
            original = placement_wire.Record.decode(originals[index])
            native = placement_wire.Plan(owner.key(step), original.plan.before)
            child = owner.effects.create(native, SOURCE, peer.address)
            if index < registered:
                owner.register(step)
            prepared = placement_wire.Record(native, 'prepared', 'none')
            child.retain(placement_rpc.Reply(SOURCE, False, index + 1, record=prepared), prepared=True)
            child.append('dispatch', {'plan_digest': native.digest.hex()})
            record = placement_wire.Record(native, 'placed', 'none', native.before.expected_after(),
                                           original.insertion)
            child.retain(placement_rpc.Reply(SOURCE, False, index + 1, record=record))
        identity = owner.id
        if placed == registered == len(plan.steps):
            origin = completion.Origin.from_batch(owner, identity)
            peer.goal = readiness.Goal(handoff, construction.Goal(origin.receipts, 10000, stable_span=10))
        return identity


class ReadinessProcessTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='dfmcp-room-readiness-process-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.root.chmod(0o700)
        self.peer = self.enterContext(Peer())
        self.batch_path = self.root / 'batch'
        self.batch_id = create_original_batch(self.batch_path, self.peer)
        self.journal = self.root / 'readiness'
        self.base_env = {key: value for key, value in os.environ.items() if not key.startswith('DFMCP_')}
        self.base_env['PYTHONDONTWRITEBYTECODE'] = '1'
        self.base_env['PYTHONPATH'] = str(ROOT / 'scripts') + os.pathsep + str(ROOT / 'tests')
        self.original = self.original_bytes()
        self.last_stdout = b''

    def environment(self, online=True):
        env = dict(self.base_env)
        if online:
            env.update({rpc.OPT_IN: '1', rpc.ENDPOINT: self.peer.address_text,
                        rpc.wire.BUILD_TOKEN: BUILD_TOKEN.decode(),
                        rpc.wire.OPERATIONS_TOKEN: OPS_TOKEN.decode(), rpc.MAP_TOKEN: MAP_TOKEN.decode()})
        return env

    def original_bytes(self):
        return {str(path.relative_to(self.batch_path)): path.read_bytes()
                for path in sorted(self.batch_path.rglob('*')) if path.is_file()}

    def arguments(self, operation, *extra, journal=None):
        return [sys.executable, str(CLI), operation, '--journal', str(journal or self.journal),
                '--timeout-ms', '60000', *extra]

    def call(self, operation, *extra, online=True, success=True, journal=None, env=None):
        result = subprocess.run(self.arguments(operation, *extra, journal=journal), capture_output=True,
                                timeout=70, env=self.environment(online) if env is None else env)
        self.assertEqual(result.returncode, 0 if success else 2, result.stdout + result.stderr)
        self.assertEqual(result.stderr, b'')
        self.last_stdout = result.stdout
        self.assertEqual(result.stdout.count(b'\n'), 1, result.stdout)
        out = json.loads(result.stdout)
        self.assertEqual(out['schema'], 'dfmcp.room-readiness-monitor-result/1')
        self.assertEqual(out['ok'], success)
        self.assertEqual(out['agent_turn']['operation'], 'room_readiness.' + operation)
        self.assertIsNone(out['agent_turn']['anchor'])
        self.assertFalse(out['agent_turn']['briefing']['runtime_admitted'])
        self.assertFalse(out['agent_turn']['briefing']['mutation_admissible'])
        self.assertFalse(out['result']['placement_effects_discharged'])
        self.assertFalse(out['result']['retry_placement_permitted'])
        self.assertFalse(out['result']['current_usability_proven'])
        self.assertLessEqual(len(result.stdout), out['agent_turn']['budget']['output_bytes_limit'])
        for token in (BUILD_TOKEN, OPS_TOKEN, MAP_TOKEN):
            self.assertNotIn(token, result.stdout)
            self.assertNotIn(token, result.stderr)
        self.assertEqual(self.peer.errors, [])
        self.assertTrue(set(self.peer.binds) <= READ_METHODS, self.peer.binds)
        return out

    def start(self, *extra, deadline=10000, **kwargs):
        return self.call('start', '--batch', str(self.batch_path), '--batch-id', self.batch_id,
                         '--deadline-tick', str(deadline), '--stable-span-ticks', '10', *extra, **kwargs)

    def assert_original(self, result, *, verified=True):
        self.assertEqual(result['requested_room_plan'], self.peer.goal.room.room_plan.json())
        self.assertEqual(result['requested_plan'], self.peer.goal.room.allocation.plan().json())
        self.assertEqual(result['origin']['batch_id'], self.batch_id)
        self.assertEqual(result['original_placement_history_verified'], verified)
        self.assertEqual(result['origin']['source_custody_verified'], verified)
        expected = {step.name: (step.item, list(step.target), step.kind)
                    for step in self.peer.goal.room.allocation.plan().steps}
        self.assertEqual({row['plan_step']: (row['item_id'], row['position'], row['kind'])
                          for row in result['targets']}, expected)
        keys = {placement_wire.Record.decode(raw).plan.key for raw in self.peer.goal.condition.receipts}
        self.assertEqual({row['placement_key'] for row in result['targets']}, keys)
        self.assertTrue(all(row['room_area'] == 'rooms' and row['room_unit'].startswith('rooms.')
                            for row in result['targets']))
        self.assertFalse(result['room_completion_proven'])
        self.assertFalse(result['terrain_completion_proven'])
        self.assertFalse(result['room_assignments_observed'])

    def assert_read_order(self, calls):
        count = len(self.peer.goal.condition.receipts)
        before, after = calls.index(('map', 'before')), calls.index(('map', 'after'))
        release = calls.index(('operations', 'release'))
        self.assertEqual(calls[:before].count(('build', 'QueryPlacement')), count)
        self.assertEqual(calls[after + 1:].count(('build', 'QueryPlacement')), count)
        self.assertLess(before, release)
        self.assertLess(release, after)
        self.assertEqual(calls.count(('map', 'before')), 1)
        self.assertEqual(calls.count(('map', 'after')), 1)

    def test_full_original_room_reaches_one_joint_condition_across_processes(self):
        first = self.start()['result']
        self.assert_original(first)
        self.assertEqual((first['progress']['phase'], first['progress']['streak']), ('candidate', 1))
        self.assert_read_order(self.peer.calls)
        self.assertFalse(first['room_readiness_sampled_condition'])
        repeated = self.call('sample')['result']
        self.assertEqual(repeated['progress']['streak'], 1)
        self.assertEqual(repeated['progress']['observations'], 2)
        self.peer.tick += 10
        begin = len(self.peer.calls)
        done = self.call('sample')['result']
        self.assert_original(done)
        self.assert_read_order(self.peer.calls[begin:])
        self.assertEqual(done['progress']['phase'], 'satisfied')
        self.assertTrue(done['room_readiness_sampled_condition'])
        self.assertTrue(done['progress']['terrain_at_last_observation']['all_required_shapes_at_sample'])
        for name in ('atomic_cross_profile_snapshot_proven', 'continuous_wall_preservation_proven',
                     'continuous_stability_proven', 'room_completion_proven', 'room_assignments_observed'):
            self.assertFalse(done['progress'][name])
        raw, connections = self.journal.read_bytes(), self.peer.connections
        for operation in ('inspect', 'sample'):
            cached = self.call(operation, online=False)['result']
            self.assertEqual(cached['progress'], done['progress'])
            self.assert_original(cached)
            self.assertTrue(cached['room_readiness_sampled_condition'])
        self.assertEqual(self.peer.connections, connections)
        self.assertEqual(self.journal.read_bytes(), raw)
        self.assertEqual(self.original_bytes(), self.original)

    def test_terrain_and_furniture_success_at_different_samples_never_combine(self):
        selected = room_terrain.selection(self.peer.goal.room.room_plan)
        item = self.peer.goal.room.allocation.plan().steps[0].item
        self.peer.ops_options = {'pending_item': item}
        first = self.start()['result']['progress']
        self.assertEqual(first['streak'], 0)
        self.assertTrue(first['terrain_at_last_observation']['all_required_shapes_at_sample'])
        self.peer.ops_options = {}
        self.peer.map_overrides = {min(selected.walls): fixtures.tile(3)}
        self.peer.tick += 10
        split = self.call('sample')['result']
        self.assertTrue(split['progress']['furnishing_conditions_at_sample'])
        self.assertFalse(split['progress']['terrain_at_last_observation']['all_required_shapes_at_sample'])
        self.assertEqual(split['progress']['streak'], 0)
        self.assertFalse(split['room_readiness_sampled_condition'])
        self.peer.map_overrides = {}
        self.peer.ops_options = {'pending_item': item}
        self.peer.tick += 10
        self.assertEqual(self.call('sample')['result']['progress']['streak'], 0)
        self.peer.ops_options = {}
        self.peer.tick += 10
        self.assertEqual(self.call('sample')['result']['progress']['streak'], 1)
        self.peer.tick += 10
        self.assertTrue(self.call('sample')['result']['room_readiness_sampled_condition'])
        self.assertEqual(self.original_bytes(), self.original)

    def test_hidden_terrain_and_missing_occupancy_reset_complete_joint_streak(self):
        self.start()
        selected = room_terrain.selection(self.peer.goal.room.room_plan)
        target = self.peer.goal.room.allocation.plan().steps[0].target
        floor = next(point for point in selected.floors if point != target)
        for changed in ({floor: b'\x01'}, {target: fixtures.tile(3)}):
            self.peer.tick += 10
            self.peer.map_overrides = changed
            value = self.call('sample')['result']
            self.assertEqual(value['progress']['streak'], 0)
            self.assertTrue(value['progress']['furnishing_conditions_at_sample'])
            self.assertFalse(value['room_readiness_sampled_condition'])
        self.assertFalse(value['progress']['terrain_at_last_observation']['furniture_occupancy'][
            'all_targets_have_building_occupancy'])
        self.peer.map_overrides = {}
        self.peer.tick += 10
        self.assertEqual(self.call('sample')['result']['progress']['streak'], 1)
        self.peer.tick += 10
        self.assertTrue(self.call('sample')['result']['room_readiness_sampled_condition'])

    def test_lost_trailing_map_preserves_unknown_read_and_resets_after_restart(self):
        self.start()
        self.peer.fault = 'lost_map_after'
        self.peer.tick += 10
        before = self.peer.connections
        failure = self.call('sample', success=False)['result']
        self.assertIsNone(failure['progress'])
        self.assertFalse(failure['room_readiness_sampled_condition'])
        self.assertEqual(self.peer.connections, before + 1)
        raw = self.journal.read_bytes()
        retained = self.call('inspect', online=False)['result']['progress']
        self.assertEqual(retained['observations'], 1)
        self.assertTrue(retained['read_outcome_unknown'])
        self.assertEqual(retained['effective_streak'], 0)
        self.assertEqual(self.journal.read_bytes(), raw)
        self.peer.fault = None
        self.peer.tick += 10
        restarted = self.call('sample')['result']['progress']
        self.assertEqual((restarted['streak'], restarted['interrupted_reads']), (1, 1))
        self.assertEqual(restarted['effective_streak'], 1)
        self.assertEqual(restarted['first_tick'], self.peer.tick)
        self.peer.tick += 10
        self.assertTrue(self.call('sample')['result']['room_readiness_sampled_condition'])
        self.assertEqual(self.original_bytes(), self.original)

    def test_process_death_during_capture_cannot_publish_or_reuse_prior_streak(self):
        self.start()
        reached, release = threading.Event(), threading.Event()
        def hold(operation):
            if operation == ('operations', 'release'):
                reached.set()
                release.wait(10)
        self.peer.callback = hold
        self.peer.tick += 10
        process = subprocess.Popen(self.arguments('sample'), stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   env=self.environment())
        try:
            self.assertTrue(reached.wait(10), 'child did not reach held read boundary')
            process.kill()
            stdout, stderr = process.communicate(timeout=5)
            self.assertLess(process.returncode, 0)
            self.assertEqual((stdout, stderr), (b'', b''))
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate(timeout=5)
            release.set()
            self.peer.callback = None
        self.assertTrue(self.peer.connection_closed.wait(5))
        retained = self.call('inspect', online=False)['result']['progress']
        self.assertEqual(retained['observations'], 1)
        self.assertTrue(retained['read_outcome_unknown'])
        self.assertEqual(retained['effective_streak'], 0)
        self.peer.tick += 10
        recovered = self.call('sample')['result']['progress']
        self.assertEqual((recovered['streak'], recovered['interrupted_reads']), (1, 1))
        self.assertEqual(recovered['effective_streak'], 1)
        self.peer.tick += 10
        self.assertTrue(self.call('sample')['result']['room_readiness_sampled_condition'])
        self.assertEqual(self.original_bytes(), self.original)

    def test_malformed_or_changed_brackets_never_publish_partial_samples(self):
        for fault in ('changed_map', 'map_tick', 'map_generation', 'page_digest', 'release', 'receipt_after'):
            with self.subTest(fault=fault):
                self.peer.fault = fault
                journal = self.root / ('fault-' + fault)
                before = self.peer.connections
                out = self.start(journal=journal, success=False)['result']
                self.assertIsNone(out['progress'])
                self.assertFalse(out['room_readiness_sampled_condition'])
                self.assertEqual(self.peer.connections, before + 1)
                progress = self.call('inspect', journal=journal, online=False)['result']['progress']
                self.assertEqual(progress['observations'], 0)
                self.assertTrue(progress['read_outcome_unknown'])
        self.assertEqual(self.original_bytes(), self.original)

    def test_fixed_cadence_gap_and_deadline_survive_reopen_without_policy_renewal(self):
        self.start('--interval-ticks', '10', '--max-gap-ticks', '20')
        raw, connections = self.journal.read_bytes(), self.peer.connections
        self.call('sample', '--deadline-tick', '20000', success=False)
        self.assertEqual((self.journal.read_bytes(), self.peer.connections), (raw, connections))
        self.peer.tick += 5
        self.assertEqual(self.call('sample')['result']['progress']['streak'], 1)
        self.peer.tick += 30
        reset = self.call('sample')['result']['progress']
        self.assertEqual((reset['streak'], reset['first_tick']), (1, self.peer.tick))
        self.peer.tick += 10
        self.assertTrue(self.call('sample')['result']['room_readiness_sampled_condition'])
        journal = self.root / 'fixed-deadline'
        self.start(journal=journal, deadline=self.peer.tick + 10)
        self.peer.tick += 10
        expired = self.call('sample', journal=journal)['result']
        self.assertEqual((expired['progress']['phase'], expired['progress']['reason']),
                         ('expired', 'game_deadline_reached'))
        self.assertFalse(expired['room_readiness_sampled_condition'])

    def test_observation_limit_cannot_be_renewed_by_new_cli_process(self):
        self.start('--max-observations', '2')
        self.peer.tick += 1
        expired = self.call('sample')['result']
        self.assertEqual((expired['progress']['phase'], expired['progress']['reason']),
                         ('expired', 'sample_budget_exhausted'))
        self.assertEqual(expired['progress']['observations'], 2)
        self.assertFalse(expired['room_readiness_sampled_condition'])
        raw, connections = self.journal.read_bytes(), self.peer.connections
        self.assertEqual(self.call('sample', online=False)['result']['progress'], expired['progress'])
        self.assertEqual((self.journal.read_bytes(), self.peer.connections), (raw, connections))

    def test_cancel_after_original_file_loss_retains_intent_without_completion_claim(self):
        self.start()
        source = self.batch_path / 'batch.json'
        retained = self.root / 'retained-manifest'
        source.rename(retained)
        connections = self.peer.connections
        self.call('inspect', online=False, success=False)
        stopped = self.call('cancel', online=False)['result']
        self.assert_original(stopped, verified=False)
        self.assertEqual(stopped['progress']['phase'], 'cancelled')
        self.assertFalse(stopped['room_readiness_sampled_condition'])
        self.assertEqual(self.peer.connections, connections)
        raw = self.journal.read_bytes()
        self.assertEqual(self.call('cancel', online=False)['result']['progress'], stopped['progress'])
        self.assertEqual(self.journal.read_bytes(), raw)
        retained.rename(source)
        self.assertEqual(self.call('sample', online=False)['result']['progress'], stopped['progress'])
        self.assertEqual(self.original_bytes(), self.original)

    def test_replaced_original_child_custody_refuses_before_native_acquisition(self):
        self.start()
        child = next((self.batch_path / 'effects').iterdir())
        replacement = self.root / 'replacement-child'
        replacement.write_bytes(child.read_bytes())
        replacement.chmod(0o600)
        os.replace(replacement, child)
        raw, connections = self.journal.read_bytes(), self.peer.connections
        out = self.call('sample', success=False)['result']
        self.assertFalse(out['original_placement_history_verified'])
        self.assertFalse(out['room_readiness_sampled_condition'])
        self.assertEqual((self.journal.read_bytes(), self.peer.connections), (raw, connections))
        self.assertEqual(self.call('cancel', online=False)['result']['progress']['phase'], 'cancelled')

    def test_original_source_loss_during_final_read_prevents_sample_publication(self):
        self.start()
        source = self.batch_path / 'batch.json'
        retained = self.root / 'moved-during-read'
        def remove_source(operation):
            if operation == ('map', 'after'):
                source.rename(retained)
        self.peer.callback = remove_source
        self.peer.tick += 10
        out = self.call('sample', success=False)['result']
        self.assertFalse(out['original_placement_history_verified'])
        self.assertFalse(out['room_readiness_sampled_condition'])
        self.peer.callback = None
        retained.rename(source)
        evidence = self.call('inspect', online=False)['result']['progress']
        self.assertEqual(evidence['observations'], 1)
        self.assertTrue(evidence['read_outcome_unknown'])
        self.peer.tick += 10
        self.assertEqual(self.call('sample')['result']['progress']['streak'], 1)
        self.assertEqual(self.original_bytes(), self.original)

    def test_partial_or_unregistered_original_batch_never_creates_monitor_or_connects(self):
        for placed, registered in ((1, 1), (len(self.peer.goal.condition.receipts), 1)):
            with self.subTest(placed=placed, registered=registered):
                path = self.root / f'partial-{placed}-{registered}'
                identity = create_original_batch(path, self.peer, placed=placed, registered=registered)
                journal = self.root / f'partial-monitor-{placed}-{registered}'
                self.call('start', '--batch', str(path), '--batch-id', identity,
                          '--deadline-tick', '10000', journal=journal, success=False)
                self.assertFalse(journal.exists())
        self.assertEqual(self.peer.connections, 0)
        self.assertEqual(self.original_bytes(), self.original)

    def test_place_authority_and_unpaused_game_cannot_trigger_control_or_auto_retry(self):
        env = self.environment()
        env['DFMCP_BUILD_ALLOW_PLACE'] = '1'
        self.start(env=env, success=False)
        self.assertEqual(self.peer.connections, 0)
        self.assertFalse(self.journal.exists())
        self.peer.ops_options = {'paused': False}
        self.start(success=False)
        self.assertEqual(self.peer.connections, 1)
        self.assertFalse(any('Pause' in name or 'Commit' in name or 'Prepare' in name
                             for _, name in self.peer.binds))
        self.assertEqual(self.original_bytes(), self.original)

    def test_same_tick_changed_map_bytes_reset_shared_streak_across_processes(self):
        self.start()
        selected = room_terrain.selection(self.peer.goal.room.room_plan)
        unused = next(point for point in room_terrain.points(selected.region, lambda: None)
                      if point not in selected.floors | selected.walls)
        self.peer.map_overrides = {unused: b'\x01'}
        changed = self.call('sample')['result']['progress']
        self.assertEqual((changed['streak'], changed['reason']), (0, 'room_sample_changed_at_same_tick'))
        self.peer.tick += 10
        self.assertEqual(self.call('sample')['result']['progress']['streak'], 1)
        self.peer.tick += 10
        self.assertTrue(self.call('sample')['result']['room_readiness_sampled_condition'])

    def test_new_map_incarnation_invalidates_retained_joint_history_without_new_authority(self):
        self.start()
        profile = PROFILES[b'dfmcp_map_v1_5']
        changed = (*profile[:-1], replace(profile[-1], generation=profile[-1].generation + 1))
        self.peer.tick += 10
        with patch.dict(PROFILES, {b'dfmcp_map_v1_5': changed}):
            invalidated = self.call('sample')['result']
        self.assertEqual((invalidated['progress']['phase'], invalidated['progress']['reason']),
                         ('invalidated', 'room_map_source_changed'))
        self.assertFalse(invalidated['room_readiness_sampled_condition'])
        raw, connections = self.journal.read_bytes(), self.peer.connections
        self.assertEqual(self.call('sample', online=False)['result']['progress'], invalidated['progress'])
        self.assertEqual((self.journal.read_bytes(), self.peer.connections), (raw, connections))
        self.assertEqual(self.original_bytes(), self.original)

    def test_maximum_original_room_and_multipage_roster_remain_complete(self):
        handoff = fixtures.handoff(fixtures.room(True), self.peer.address_text)
        self.peer.goal = readiness.Goal(handoff,
            construction.Goal(fixtures.receipts(handoff), 10000, stable_span=10))
        self.peer.extra_items = 2000
        self.batch_path = self.root / 'maximum-batch'
        self.batch_id = create_original_batch(self.batch_path, self.peer)
        original = self.original_bytes()
        first = self.start()['result']
        self.assert_original(first)
        self.assertEqual(len(first['targets']), 32)
        self.assertEqual(len(first['requested_room_plan']['intent']['excluded_items']), 646)
        self.assertGreater(self.peer.calls.count(('operations', 'ReadObservation')), 1)
        self.assertEqual(self.peer.calls.count(('build', 'QueryPlacement')), 64)
        self.assert_read_order(self.peer.calls)
        self.peer.tick += 10
        done = self.call('sample')['result']
        self.assert_original(done)
        self.assertTrue(done['room_readiness_sampled_condition'])
        self.assertEqual(done['progress']['condition_met_count'], 32)
        self.assertEqual(self.original_bytes(), original)


if __name__ == '__main__':
    unittest.main()
