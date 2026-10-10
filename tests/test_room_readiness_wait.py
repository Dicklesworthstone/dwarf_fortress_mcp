"""Bounded room waits through original custody, joint samples and real TCP.

The joined peer controls explicit synthetic game ticks between captures. Tests
do not run DFHack, advance a real game clock or establish live admission.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from contextlib import redirect_stdout
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import construction_plan as construction
import construction_wait as foreground
import furniture_batch as batch_module
import furniture_completion as completion
import room_readiness_rpc as rpc
import room_readiness_store as store
import room_readiness_fixtures as fixtures
from room_readiness_peer import BUILD_TOKEN, OPS_TOKEN, MAP_TOKEN, Peer
import room_terrain
import track_room_readiness as cli
from test_track_room_readiness import create_original_batch, READ_METHODS

ROOT = Path(__file__).resolve().parents[1]


class RoomReadinessWaitTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='dfmcp-room-wait-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.root.chmod(0o700)
        self.peer = self.enterContext(Peer())
        self.batch_path = self.root / 'batch'
        self.batch_id = create_original_batch(self.batch_path, self.peer)
        self.journal = self.root / 'readiness'
        self.original = {path.relative_to(self.batch_path): path.read_bytes()
                         for path in self.batch_path.rglob('*') if path.is_file()}

    def environment(self, online=True):
        env = {key: value for key, value in os.environ.items() if not key.startswith('DFMCP_')}
        env['PYTHONDONTWRITEBYTECODE'] = '1'
        env['PYTHONPATH'] = str(ROOT / 'scripts') + os.pathsep + str(ROOT / 'tests')
        if online:
            env.update({rpc.OPT_IN: '1', rpc.ENDPOINT: self.peer.address_text,
                rpc.wire.BUILD_TOKEN: BUILD_TOKEN.decode(),
                rpc.wire.OPERATIONS_TOKEN: OPS_TOKEN.decode(), rpc.MAP_TOKEN: MAP_TOKEN.decode()})
        return env

    def create(self, **policy):
        budget = rpc.Budget(60000)
        with batch_module.Batch(str(self.batch_path), budget) as batch:
            origin = completion.Origin.from_batch(batch, self.batch_id)
            condition = construction.Goal(origin.receipts, policy.pop('deadline', 10000),
                                         stable_span=policy.pop('stable_span', 10), **policy)
            goal = store.Goal(origin, condition, guard=budget.work)
            with store.Journal(str(self.journal), budget, writable=True, create=(goal, self.peer.address)) as owner:
                owner.bind_batch(batch)
        self.peer.goal = goal.readiness_goal
        return goal

    def state(self):
        with store.Journal(str(self.journal), rpc.Budget(60000)) as journal:
            return journal.state

    def args(self, *extra, operation='wait'):
        return [operation, '--journal', str(self.journal), '--timeout-ms', '60000', *extra]

    def command(self, *extra, online=True, success=True, operation='wait'):
        process = subprocess.run([sys.executable, str(ROOT / 'scripts' / 'track_room_readiness.py'),
                                  *self.args(*extra, operation=operation)],
                                 env=self.environment(online), capture_output=True, timeout=70)
        self.assertEqual(process.returncode, 0 if success else 2, process.stdout + process.stderr)
        self.assertEqual(process.stderr, b'')
        self.assertEqual(process.stdout.count(b'\n'), 1)
        result = json.loads(process.stdout)
        self.assertEqual(result['ok'], success)
        self.assertEqual(result['schema'], 'dfmcp.room-readiness-monitor-result/1')
        self.assertEqual(result['agent_turn']['operation'], 'room_readiness.' + operation)
        self.assertLessEqual(len(process.stdout), result['agent_turn']['budget']['output_bytes_limit'])
        self.assertTrue(set(self.peer.binds) <= READ_METHODS)
        for token in (BUILD_TOKEN, OPS_TOKEN, MAP_TOKEN):
            self.assertNotIn(token, process.stdout)
        return result

    def advance_ticks(self, increment=10):
        def advance(operation):
            if operation == ('map', 'after'):
                self.peer.tick += increment
        self.peer.callback = advance

    def test_one_process_wait_reaches_complete_joint_readiness_and_terminal_wait_is_offline(self):
        goal = self.create()
        self.advance_ticks()
        value = self.command('--poll-ms', '10')['result']
        self.assertEqual(value['wait']['samples_published'], 2)
        self.assertEqual(value['wait']['stop_reason'], 'terminal')
        self.assertTrue(value['room_readiness_sampled_condition'])
        self.assertEqual(value['progress']['phase'], 'satisfied')
        self.assertEqual(value['identity']['goal_digest'], goal.digest)
        self.assertEqual(value['requested_room_plan'], goal.origin.room_handoff.room_plan.json())
        self.assertEqual(value['requested_plan'], goal.origin.plan.json())
        self.assertEqual(len(value['targets']), len(goal.receipts))
        self.assertFalse(value['wait']['game_time_advanced_by_wait'])
        self.assertFalse(value['wait']['background_work_started'])
        self.assertFalse(value['room_completion_proven'])
        self.assertFalse(value['progress']['atomic_cross_profile_snapshot_proven'])
        self.assertEqual(self.peer.connections, 2)
        raw = self.journal.read_bytes()
        terminal = self.command(online=False)['result']
        self.assertEqual(terminal['wait']['samples_published'], 0)
        self.assertEqual(terminal['wait']['stop_reason'], 'terminal')
        self.assertFalse(terminal['native_connection_attempted'])
        self.assertEqual(terminal['progress'], value['progress'])
        self.assertEqual(self.journal.read_bytes(), raw)
        self.assertEqual(self.peer.connections, 2)
        self.assertEqual(self.original, {path.relative_to(self.batch_path): path.read_bytes()
                         for path in self.batch_path.rglob('*') if path.is_file()})

    def test_wall_loss_inside_wait_resets_the_single_streak_before_later_joint_success(self):
        goal = self.create()
        wall = min(room_terrain.selection(goal.origin.room_handoff.room_plan).walls)
        def next_capture(operation):
            if operation == ('map', 'after'):
                self.peer.tick += 10
                self.peer.map_overrides = {wall: fixtures.tile(3)} if self.peer.connections == 1 else {}
        self.peer.callback = next_capture
        value = self.command('--poll-ms', '10')['result']
        self.assertEqual(value['wait']['samples_published'], 4)
        self.assertEqual(value['progress']['observations'], 4)
        self.assertEqual((value['progress']['streak'], value['progress']['first_tick']), (2, 320))
        self.assertTrue(value['room_readiness_sampled_condition'])
        self.assertEqual(self.peer.connections, 4)

    def test_stalled_paused_game_stops_after_bounded_samples_without_false_readiness(self):
        self.create()
        value = self.command('--poll-ms', '10')['result']
        self.assertEqual((value['wait']['samples_published'], value['wait']['stop_reason']),
                         (3, 'game_tick_not_advanced'))
        self.assertEqual(value['progress']['streak'], 1)
        self.assertFalse(value['progress']['read_outcome_unknown'])
        self.assertFalse(value['room_readiness_sampled_condition'])
        self.assertEqual(self.peer.connections, 3)

    def test_lost_trailing_map_is_not_retried_and_explicit_wait_resets_stability(self):
        goal = self.create()
        def lose_next(operation):
            if operation == ('map', 'after'):
                self.peer.tick += 10
                self.peer.fault = 'lost_map_after'
        self.peer.callback = lose_next
        failed = self.command('--poll-ms', '10', success=False)['result']
        self.assertIsNone(failed['progress'])
        self.assertEqual(self.peer.connections, 2)
        state = self.state()
        self.assertTrue(state.progress.reading)
        self.assertEqual(state.progress.observations, 1)
        self.peer.fault = None
        self.peer.tick += 10
        self.advance_ticks()
        recovered = self.command('--poll-ms', '10')['result']
        self.assertEqual(recovered['identity']['goal_digest'], goal.digest)
        self.assertEqual(recovered['wait']['samples_published'], 2)
        self.assertEqual(recovered['progress']['interrupted_reads'], 1)
        self.assertEqual(recovered['progress']['first_tick'], 320)
        self.assertTrue(recovered['room_readiness_sampled_condition'])
        self.assertEqual(self.peer.connections, 4)

    def test_authority_or_original_custody_loss_during_delay_prevents_next_intent(self):
        for loss in ('authority', 'custody'):
            with self.subTest(loss=loss):
                self.journal = self.root / ('delay-' + loss)
                self.create()
                self.advance_ticks()
                previous_connections = self.peer.connections
                source = self.batch_path / 'batch.json'
                held = self.root / 'held-manifest'
                def invalidate(_):
                    if loss == 'authority':
                        os.environ[rpc.MAP_TOKEN] = 'x' * 32
                    else:
                        source.rename(held)
                output = io.StringIO()
                with patch.dict(os.environ, self.environment(), clear=True), \
                     patch.object(foreground.time, 'sleep', invalidate), redirect_stdout(output):
                    code = cli.main(self.args('--poll-ms', '10'))
                self.assertEqual(code, 2)
                result = json.loads(output.getvalue())
                self.assertFalse(result['ok'])
                self.assertFalse(result['result']['room_readiness_sampled_condition'])
                self.assertEqual(self.peer.connections, previous_connections + 1)
                state = self.state()
                self.assertEqual(state.progress.observations, 1)
                self.assertFalse(state.progress.reading)
                if held.exists():
                    held.rename(source)

    def test_map_rpc_allowance_is_reserved_before_any_new_durable_read_intent(self):
        self.create()
        budget = rpc.Budget(60000)
        with batch_module.Batch(str(self.batch_path), budget) as batch, \
             store.Journal(str(self.journal), budget, writable=True) as owner, \
             patch.dict(os.environ, self.environment(), clear=True):
            owner.bind_batch(batch)
            authority = rpc.Authority.load()
            # Enough for the legacy furnishing monitor, four calls short of
            # the five additional map operations this profile must reserve.
            budget.calls = 8 + 2 * len(owner.state.goal.receipts) + 1
            raw, frames = self.journal.read_bytes(), owner.state.frames
            def forbidden(*_):
                self.fail('insufficient joint allowance acquired another native sample')
            result = foreground.run(owner, authority, forbidden, lambda _: None,
                                    foreground.Limits(), additional_rpc_calls=5,
                                    source_guard=lambda: owner.state.goal.origin.verify_batch(batch),
                                    sleeper=forbidden)
            self.assertEqual((result.samples, result.stop_reason), (0, 'rpc_allowance'))
            self.assertEqual(owner.state.frames, frames)
            self.assertFalse(owner.state.progress.reading)
            self.assertFalse(owner.read_owned)
            self.assertEqual(self.journal.read_bytes(), raw)
        self.assertEqual(self.peer.connections, 0)

    def test_wait_slices_preserve_original_deadline_and_total_observation_budget(self):
        goal = self.create(deadline=315, stable_samples=3)
        self.advance_ticks()
        first = self.command('--wait-samples', '1', '--poll-ms', '10')['result']
        self.assertEqual(first['wait']['stop_reason'], 'sample_limit')
        self.assertEqual(first['progress']['observations'], 1)
        second = self.command('--wait-samples', '1', '--poll-ms', '10')['result']
        self.assertEqual(second['progress']['observations'], 2)
        last = self.command('--poll-ms', '10')['result']
        self.assertEqual(last['progress']['phase'], 'expired')
        self.assertEqual(last['progress']['reason'], 'game_deadline_reached')
        self.assertEqual(last['goal']['deadline_tick'], 315)
        self.assertEqual(last['identity']['goal_digest'], goal.digest)
        self.assertFalse(last['room_readiness_sampled_condition'])
        self.assertEqual(self.peer.connections, 3)
        self.journal = self.root / 'observation-budget'
        goal = self.create(max_observations=2)
        self.advance_ticks(1)
        value = self.command('--poll-ms', '10')['result']
        self.assertEqual(value['progress']['reason'], 'sample_budget_exhausted')
        self.assertEqual(value['progress']['observations'], 2)
        self.assertEqual(value['identity']['goal_digest'], goal.digest)
        self.assertFalse(value['room_readiness_sampled_condition'])

    def test_extra_rpc_reservation_refuses_values_that_could_weaken_or_unbound_admission(self):
        self.create()
        budget = rpc.Budget(60000)
        with batch_module.Batch(str(self.batch_path), budget) as batch, \
             store.Journal(str(self.journal), budget, writable=True) as owner, \
             patch.dict(os.environ, self.environment(), clear=True):
            owner.bind_batch(batch)
            authority = rpc.Authority.load()
            raw = self.journal.read_bytes()
            def forbidden(*_):
                self.fail('invalid extra RPC count reached acquisition')
            for invalid in (-1, 33, True, False, 5.0, '5', None):
                with self.subTest(additional_rpc_calls=invalid):
                    self.assertRaises(ValueError, foreground.run, owner, authority,
                        forbidden, lambda _: None, foreground.Limits(),
                        additional_rpc_calls=invalid, sleeper=forbidden)
                    self.assertEqual(self.journal.read_bytes(), raw)
                    self.assertFalse(owner.state.progress.reading)
        self.assertEqual(self.peer.connections, 0)

    def test_wait_options_cannot_create_retarget_or_change_other_operations(self):
        self.command(success=False)
        self.assertFalse(self.journal.exists())
        goal = self.create()
        raw = self.journal.read_bytes()
        for option, value in (('--deadline-tick', '20000'), ('--stable-samples', '5'),
                              ('--wait-samples', '0'), ('--poll-ms', '9')):
            self.command(option, value, success=False)
            self.assertEqual(self.journal.read_bytes(), raw)
        for operation in ('sample', 'inspect', 'cancel'):
            self.command('--wait-samples', '2', operation=operation, success=False)
            self.assertEqual(self.journal.read_bytes(), raw)
        self.assertEqual(self.state().goal.digest, goal.digest)
        self.assertEqual(self.peer.connections, 0)

    def test_complete_wait_envelope_must_fit_before_intent_and_before_sample_publication(self):
        for stage in ('before_intent', 'before_publication'):
            with self.subTest(stage=stage):
                self.journal = self.root / stage
                self.create()
                original_output = cli.output
                raw = self.journal.read_bytes()
                prior_connections = self.peer.connections
                def bounded_output(value):
                    result = value['result']
                    progress = result.get('progress')
                    if value['ok'] and result.get('wait') is not None:
                        self.assertEqual(result['wait']['max_samples'], 8)
                        if stage == 'before_intent' or (progress is not None and progress['observations']):
                            raise ValueError('complete original room and wait envelope exceed output budget')
                    return original_output(value)
                output = io.StringIO()
                with patch.dict(os.environ, self.environment(), clear=True), \
                     patch.object(cli, 'output', bounded_output), redirect_stdout(output):
                    code = cli.main(self.args('--poll-ms', '10'))
                self.assertEqual(code, 2)
                self.assertFalse(json.loads(output.getvalue())['ok'])
                state = self.state()
                self.assertEqual(state.progress.observations, 0)
                if stage == 'before_intent':
                    self.assertEqual(self.peer.connections, prior_connections)
                    self.assertEqual(self.journal.read_bytes(), raw)
                    self.assertFalse(state.progress.reading)
                else:
                    self.assertEqual(self.peer.connections, prior_connections + 1)
                    self.assertTrue(state.progress.reading)


if __name__ == '__main__':
    unittest.main()
