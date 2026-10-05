"""Real journal/CLI/TCP tests. The joined peer is not DFHack or a live fortress."""
from contextlib import redirect_stdout
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import excavation_observer as e
import excavation_blueprint as b
import room_terrain_goal as g
import track_excavation as t
import room_terrain_fixtures as f
from room_terrain_peer import Peer, TOKEN
from test_room_terrain_goal import sample

ROOT = Path(__file__).resolve().parents[1]


class SequencePeer(Peer):
    """One joined server, with the existing peer's one-read assertions per session."""
    def session(self, connection):
        self.handshakes = self.observations = 0
        super().session(connection)


def env(address):
    return {'DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5': '1',
            'DFMCP_MAP_TOKEN': TOKEN.decode(), 'DFMCP_MAP_ENDPOINT': address}


def begin_event(goal, captured, address='127.0.0.1:5000'):
    return {'kind': 'begin', 'format': t.profile_for_goal(goal).format, 'nonce': 'ab' * 32,
            'endpoint': address, 'goal': goal.json(), 'sample': t.sample_value(captured)}


def history_bytes(events, profile=t.ROOM_PROFILE):
    previous, chunks = '0' * 64, []
    for index, event in enumerate(events):
        line = t.frame(event, index, previous, profile)
        previous = json.loads(line)['sha256']
        chunks.append(line)
    return b''.join(chunks)


def legacy_traces():
    """Stable inputs for pre-room tracker goldens (original blob 996811792e...)."""
    region = e.Region((10, 10, 2), (2, 2, 1))
    floor = e.Goal(region, 'region1', 2, 110)
    blueprint = b.BlueprintGoal(b.Blueprint((b.Part(e.Region((10, 10, 2), (1, 1, 1)), 'floor'),
        b.Part(e.Region((11, 11, 2), (1, 1, 1)), 'wall'))), 'region1', 2, 110)
    traces = {}
    for name, goal in (('floor', floor), ('blueprint', blueprint)):
        profile = t.profile_for_goal(goal)
        def capture(tick=100, *, hidden=False, manifest=f.MANIFEST, matched=True):
            targets = (set(f.coordinates(region)) if name == 'floor' else {(10, 10, 2)})
            tiles = {point: f.visible(3) for point in targets} if matched else {}
            if hidden:
                tiles[(10, 10, 2)] = b'\x01'
            return e.decode_capture(f.capture(region, tiles, tick=tick), manifest, region)
        begin = begin_event(goal, capture())
        started = {'kind': 'read_started'}
        success = {'kind': 'sample', 'sample': t.sample_value(capture(110))}
        scenarios = {
            'stabilizing': [begin],
            'pending': [begin_event(goal, capture(matched=False))],
            'unknown': [begin_event(goal, capture(hidden=True))],
            'satisfied': [begin, started, success],
            'unfinished': [begin, started],
            'recovered': [begin, started, started, success],
            'failed': [begin, started, {'kind': 'read_failed'}],
            'cancelled': [begin, {'kind': 'cancel'}],
            'invalidated': [begin, started, {'kind': 'sample', 'sample': t.sample_value(
                capture(110, manifest=e.Manifest(8, 'test-df', 'test-dfhack')))}],
            'expired': [begin, started, {'kind': 'sample', 'sample': t.sample_value(capture(111))}],
        }
        for state, events in scenarios.items():
            traces[name + '.' + state] = history_bytes(events, profile)
    return traces


class TrackRoomTerrainTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='whole-room-')
        self.root = Path(self.directory.name).resolve()
        self.root.chmod(0o700)
        self.path = self.root / 'goal.jsonl'
        self.spec = self.root / 'rooms.json'
        self.goal = g.RoomTerrainGoal(f.plan(), 1000)
        self.spec.write_bytes(self.goal.room_plan.encode())

    def tearDown(self):
        self.directory.cleanup()

    def seed(self, address='127.0.0.1:5000', captured=None, goal=None, events=()):
        goal = goal or self.goal
        raw = history_bytes([begin_event(goal, captured or sample(goal), address), *events])
        self.path.write_bytes(raw)
        self.path.chmod(0o600)
        return raw

    def cli(self, command, *args, address=None):
        environment = {key: value for key, value in os.environ.items() if not key.startswith('DFMCP_')}
        if address:
            environment.update(env(address))
        result = subprocess.run([sys.executable, str(ROOT / 'scripts/track_excavation.py'), command,
            '--journal', str(self.path), *map(str, args)], env=environment,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20, check=False)
        self.assertEqual(result.stderr, b'')
        return result.returncode, json.loads(result.stdout), result.stdout

    def test_legacy_floor_and_blueprint_traces_and_envelopes_are_byte_compatible(self):
        golden = json.loads((Path(__file__).parent / 'room_goal_legacy_golden.json').read_text())
        for name, raw in legacy_traces().items():
            with self.subTest(name=name):
                output = t.encode_result(t.report(t.replay(raw), False, 0))
                self.assertEqual([hashlib.sha256(raw).hexdigest(), hashlib.sha256(output).hexdigest()], golden[name])

    def test_cli_original_plan_survives_restart_and_wall_loss_blocks_completion(self):
        wall = min(self.goal._selection.walls)
        with SequencePeer(sample(self.goal, overrides={wall: f.visible(3)}).raw, self.goal.region) as peer:
            code, first, _ = self.cli('start-rooms', '--plan-file', self.spec,
                                      '--max-game-ticks', 900, address=peer.address)
            self.assertEqual((code, first['goal_status']), (0, 'pending'))
            self.assertEqual(first['goal']['room_plan'], self.goal.room_plan.json())
            self.spec.unlink()  # Later commands must not reopen or replace the intention.
            peer.raw = sample(self.goal, 110).raw
            code, second, _ = self.cli('sample', address=peer.address)
            self.assertEqual((code, second['goal_status']), (0, 'stabilizing'))
            peer.raw = sample(self.goal, 120).raw
            code, final, _ = self.cli('sample', address=peer.address)
            self.assertEqual((code, final['goal_status']), (0, 'satisfied'))
            self.assertTrue(final['room_terrain_goal_satisfied_at_sample'])
            self.assertFalse(final['floor_goal_satisfied_at_sample'])
            self.assertFalse(final['room_completion_proven'])
            original = self.path.read_bytes()
            for command in ('inspect', 'sample', 'cancel'):
                code, offline, _ = self.cli(command)
                self.assertEqual((code, offline['journal_head']), (0, final['journal_head']))
                self.assertEqual(offline['native_reads_attempted'], 0)
            self.assertEqual(self.path.read_bytes(), original)
            self.assertEqual(peer.accepted, 3)
            self.assertEqual(peer.binds, ['Handshake', 'ReadObservation'] * 3)
            self.assertEqual(peer.requests, [2, 3] * 3)
            self.assertGreater(peer.fragments, 20)

    def test_cli_original_request_compiles_once_and_exports_complete_intent(self):
        self.spec.write_bytes(e.canonical(f.request()))
        with Peer(sample(self.goal).raw, self.goal.region) as peer:
            code, result, raw = self.cli('start-rooms', '--request-file', self.spec,
                                        '--max-game-ticks', 900, address=peer.address)
        self.assertEqual(code, 0)
        self.assertEqual(result['goal']['room_plan'], self.goal.room_plan.json())
        self.assertEqual(result['agent_turn']['operation'], 'room.terrain.start-rooms')
        self.assertLessEqual(len(raw) - 1, t.ROOM_PROFILE.max_output)
        self.assertNotIn(TOKEN, self.path.read_bytes())
        self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)

    def test_durable_unfinished_read_resets_recovered_streak(self):
        with SequencePeer(sample(self.goal, 120).raw, self.goal.region) as peer:
            self.seed(peer.address, events=[{'kind': 'read_started'}])
            with patch.dict(os.environ, {}, clear=True):
                before = t.inspect(self.path)
            self.assertEqual((before['goal_status'], before['matching_samples']), ('unknown', 0))
            with patch.dict(os.environ, env(peer.address), clear=True):
                first = t.sample(self.path)
                self.assertEqual((first['goal_status'], first['matching_samples']), ('stabilizing', 1))
                peer.raw = sample(self.goal, 130).raw
                self.assertEqual(t.sample(self.path)['goal_status'], 'satisfied')
            self.assertEqual(peer.requests, [2, 3] * 2)

    def test_faults_never_reconnect_and_failed_samples_are_durable(self):
        for fault in ('lost_reply', 'truncated_reply', 'nonce', 'generation', 'software', 'profile',
                      'duplicate_field', 'native_refusal', 'oversized_header', 'alias_binding'):
            with self.subTest(fault=fault):
                with Peer(sample(self.goal, 110).raw, self.goal.region, fault=fault) as peer:
                    self.seed(peer.address)
                    with patch.dict(os.environ, env(peer.address), clear=True):
                        result = t.sample(self.path)
                    self.assertFalse(result['ok'])
                    self.assertEqual((result['goal_status'], result['matching_samples']), ('unknown', 0))
                    self.assertEqual(peer.accepted, 1)
                    self.assertLessEqual(peer.observations, 1)
                    self.assertFalse(t.replay(self.path.read_bytes()).pending_read)
                    self.assertEqual(t.inspect(self.path)['interruption'], 'read_failed')

    def test_source_change_is_terminal_and_does_not_rebind_fortress(self):
        with Peer(sample(self.goal, 110, folder='other-fort').raw, self.goal.region) as peer:
            self.seed(peer.address)
            with patch.dict(os.environ, env(peer.address), clear=True):
                result = t.sample(self.path)
        self.assertEqual(result['goal_status'], 'invalidated')
        self.assertEqual(result['source']['folder'], 'region1')
        self.assertFalse(result['room_terrain_goal_satisfied_at_sample'])
        with patch.dict(os.environ, {}, clear=True), patch.object(t, 'RoomMapClient', side_effect=AssertionError('read')):
            self.assertEqual(t.sample(self.path)['goal_status'], 'invalidated')

    def test_offline_cancel_changes_only_this_monitor_and_keeps_full_plan(self):
        self.seed()
        with patch.dict(os.environ, {'DFMCP_UNRELATED_MUTATION_TOKEN': 'do-not-use'}, clear=True), \
                patch.object(t, 'RoomMapClient', side_effect=AssertionError('read')):
            result = t.cancel(self.path)
            self.assertEqual(result['goal_status'], 'cancelled')
            self.assertEqual(result['goal']['room_plan'], self.goal.room_plan.json())
            self.assertEqual(result['interruption'], 'monitor_cancelled_not_game_action')
            self.assertFalse(result['native_effect_obligations_changed'])
            self.assertFalse(result['game_mutations_dispatched'])
            original = self.path.read_bytes()
            self.assertEqual(t.cancel(self.path)['journal_head'], result['journal_head'])
        self.assertEqual(self.path.read_bytes(), original)

    def test_journal_corruption_and_goal_substitution_refused_without_read_or_repair(self):
        good = self.seed()
        changed = json.loads(good)
        changed['event']['goal']['room_plan']['areas'][0]['units'][0]['doorway'] = [1, 1, 1]
        tampered = history_bytes([changed['event']])  # Even a recomputed outer chain is insufficient.
        cases = [good[:-1], good.replace(b'"sequence":0', b'"sequence":1'), good + b'\n',
                 tampered, good.replace(b'"schema":', b'"schema":"duplicate","schema":', 1),
                 history_bytes([begin_event(self.goal, sample(self.goal)), {'kind': 'sample',
                     'sample': t.sample_value(sample(self.goal, 110))}])]
        with patch.object(t, 'RoomMapClient', side_effect=AssertionError('native access')):
            for raw in cases:
                with self.subTest(size=len(raw)):
                    self.path.write_bytes(raw)
                    self.assertRaises((ValueError, KeyError), t.sample, self.path)
                    self.assertEqual(self.path.read_bytes(), raw)

    def test_profile_bounds_do_not_widen_old_formats(self):
        self.assertEqual((t.FLOOR_PROFILE.max_frame, t.FLOOR_PROFILE.max_journal,
                          t.FLOOR_PROFILE.max_sample, t.FLOOR_PROFILE.max_output, t.FLOOR_PROFILE.max_depth),
                         (16384, 2 * 1024 * 1024, 2048, 32768, 8))
        self.assertEqual((t.BLUEPRINT_PROFILE.max_frame, t.BLUEPRINT_PROFILE.max_journal,
                          t.BLUEPRINT_PROFILE.max_sample, t.BLUEPRINT_PROFILE.max_output, t.BLUEPRINT_PROFILE.max_depth),
                         (65536, 8 * 1024 * 1024, b.MAX_SAMPLE_BYTES, 32768, 8))
        self.assertRaises(ValueError, t.decode_record, b'[' * 9 + b'0' + b']' * 9)
        self.assertRaises(ValueError, t.replay, b'[' * 14 + b'0' + b']' * 14 + b'\n')
        event = begin_event(self.goal, sample(self.goal))
        for profile in (t.FLOOR_PROFILE, t.BLUEPRINT_PROFILE):
            wrong = {**event, 'format': profile.format}
            self.assertRaises(ValueError, t.replay, history_bytes([wrong], profile))
        self.assertRaises(ValueError, t.encode_result, {'padding': 'x' * t.MAX_OUTPUT})

    def test_revocation_during_io_resets_streak_and_withholds_live_result(self):
        def revoke():
            os.environ.pop('DFMCP_MAP_TOKEN', None)
        with Peer(sample(self.goal, 110).raw, self.goal.region, on_observation=revoke) as peer:
            self.seed(peer.address)
            with patch.dict(os.environ, env(peer.address), clear=True):
                self.assertRaises(ValueError, t.sample, self.path)
            self.assertEqual(peer.accepted, 1)
        result = t.inspect(self.path)
        self.assertEqual((result['goal_status'], result['matching_samples']), ('unknown', 0))

    def test_final_serialization_rechecks_authority_after_durable_success(self):
        original = t.encode_result
        def encode(value, operation='inspect', checkpoint=lambda: None):
            if operation == 'start-rooms' and value.get('ok'):
                os.environ.pop('DFMCP_MAP_TOKEN', None)
            return original(value, operation, checkpoint)
        with Peer(sample(self.goal).raw, self.goal.region) as peer:
            with patch.dict(os.environ, env(peer.address), clear=True), patch.object(t, 'encode_result', encode):
                out = io.StringIO()
                with redirect_stdout(out):
                    code = t.main(['start-rooms', '--journal', str(self.path), '--plan-file', str(self.spec),
                                   '--max-game-ticks', '900'])
        self.assertEqual(code, 2)
        result = json.loads(out.getvalue())
        self.assertFalse(result['ok'])
        self.assertNotIn('goal', result)
        self.assertNotIn(TOKEN.decode(), out.getvalue())
        self.assertNotIn(str(self.spec), out.getvalue())
        self.assertEqual(t.inspect(self.path)['goal_status'], 'stabilizing')

    def test_output_failure_does_not_repeat_read_or_emit_second_object(self):
        class Broken(io.StringIO):
            calls = 0
            def write(self, text):
                self.calls += 1
                return len(text) - 1
        out = Broken()
        with Peer(sample(self.goal).raw, self.goal.region) as peer:
            with patch.dict(os.environ, env(peer.address), clear=True), redirect_stdout(out):
                code = t.main(['start-rooms', '--journal', str(self.path), '--plan-file', str(self.spec),
                               '--max-game-ticks', '900'])
        self.assertEqual((code, out.calls, peer.accepted), (2, 1, 1))
        self.assertEqual(t.inspect(self.path)['observations_retained'], 1)

    def test_shared_budgets_stop_before_new_io_or_publication(self):
        for work, calls, network in ((0, 4, e.MAX_WIRE), (t.MAX_WORK, 0, e.MAX_WIRE),
                                      (t.MAX_WORK, 4, 0)):
            with self.subTest(work=work, calls=calls, network=network):
                with Peer(sample(self.goal, 110).raw, self.goal.region) as peer:
                    self.seed(peer.address)
                    budget = t.Budget(10000)
                    budget.work_left, budget.calls_left, budget.network_left = work, calls, network
                    with patch.dict(os.environ, env(peer.address), clear=True):
                        if work == 0:
                            self.assertRaises(ValueError, t.sample, self.path, _budget=budget)
                        else:
                            self.assertFalse(t.sample(self.path, _budget=budget)['ok'])
                    self.assertEqual(peer.observations, 0)
        budget = t.Budget(10000)
        budget.deadline = 0
        self.assertRaises(ValueError, t.inspect, self.path, _budget=budget)

    def test_invalid_room_file_and_bad_custody_fail_before_network(self):
        with patch.dict(os.environ, env('127.0.0.1:5000'), clear=True), \
                patch.object(t, 'RoomMapClient', side_effect=AssertionError('native access')):
            for raw in (b'', b'[' * 12 + b'0' + b']' * 12, b'{}', b'x' * (g.MAX_PLAN_BYTES + 1)):
                self.spec.write_bytes(raw)
                self.assertRaises((ValueError, KeyError), t.start_rooms, self.path, self.spec, 900)
                self.assertFalse(self.path.exists())
            self.spec.unlink()
            target = self.root / 'target'
            target.write_bytes(self.goal.room_plan.encode())
            self.spec.symlink_to(target)
            self.assertRaises(OSError, t.start_rooms, self.path, self.spec, 900)
            self.seed()
            self.path.chmod(0o644)
            self.assertRaises(ValueError, t.sample, self.path)
            self.path.chmod(0o600)
            os.link(self.path, self.root / 'linked')
            self.assertRaises(ValueError, t.inspect, self.path)

    def test_lock_contention_and_failed_append_do_not_repair_or_dispatch(self):
        original = self.seed()
        with t.open_journal(self.path, t.Budget(10000), writable=True):
            self.assertRaises(OSError, t.cancel, self.path)
        self.assertEqual(self.path.read_bytes(), original)
        with t.open_journal(self.path, t.Budget(10000), writable=True) as journal:
            with patch.object(os, 'write', return_value=0):
                self.assertRaises(ValueError, journal.append, {'kind': 'read_started'})
            self.assertTrue(journal.fenced)
            self.assertRaises(ValueError, journal.verify)
        self.assertEqual(self.path.read_bytes(), original)

    def test_large_capture_and_full_32_slot_intent_fit_complete_envelopes(self):
        intent = f.request(f.bedroom(10), f.dining(1, 1, origin=(26, 10, 2)))
        intent['excluded_items'] = list(range(2147400000, 2147400646))
        large = f.RoomPlan.compile(intent)
        self.assertEqual(large.json()['furniture_count'], 32)
        for plan in (large, f.plan(f.dining(1, 1),
                     f.dining(1, 1, origin=(132, 13, 2), name='far'))):
            goal = g.RoomTerrainGoal(plan, 1000)
            captured = sample(goal, overrides={point: f.visible(2) for point in goal._selection.floors})
            raw = self.seed(goal=goal, captured=captured)
            history = t.replay(raw)
            output = t.encode_result(t.report(history, False, 0))
            self.assertEqual(history.goal.room_plan.json(), plan.json())
            self.assertLessEqual(len(raw), t.ROOM_PROFILE.max_frame)
            self.assertLessEqual(len(output), t.ROOM_PROFILE.max_output)
            self.assertEqual(history.goal.region, goal.region)
            if plan is large:
                self.assertGreater(len(output), t.MAX_OUTPUT)
                self.assertEqual(len(history.goal.room_plan.json()['intent']['excluded_items']), 646)
            self.spec.write_bytes(plan.encode())
            with Peer(captured.raw, goal.region) as peer:
                with patch.dict(os.environ, env(peer.address), clear=True):
                    live = t.start_rooms(self.root / (plan.digest + '.jsonl'), self.spec, 900)
                    live_output = t.encode_result(live)
                self.assertEqual((peer.accepted, peer.observations), (1, 1))
                self.assertEqual(peer.requests, [2, 3])
                self.assertEqual(live['goal'], history.goal.json())
                self.assertLessEqual(len(live_output), t.ROOM_PROFILE.max_output)
        self.assertEqual(goal.region.volume, 3072)

    def test_legacy_floor_and_blueprint_cli_still_read_and_complete(self):
        region = e.Region((10, 10, 2), (2, 2, 1))
        tiles = {p: f.visible(3) for p in f.coordinates(region)}
        for command in ('start', 'start-blueprint'):
            with self.subTest(command=command):
                if self.path.exists():
                    self.path.unlink()
                self.spec.write_bytes(e.canonical({'schema': b.SCHEMA,
                    'parts': [{'region': region.json(), 'shape': 'floor'}]}))
                extra = (['--x', '10', '--y', '10', '--z', '2', '--width', '2', '--height', '2']
                         if command == 'start' else ['--blueprint', self.spec])
                with SequencePeer(f.capture(region, tiles), region) as peer:
                    code, first, _ = self.cli(command, *extra, '--world-folder', 'region1', '--site', '2',
                                              '--max-game-ticks', '900', address=peer.address)
                    self.assertEqual((code, first['goal_status']), (0, 'stabilizing'))
                    peer.raw = f.capture(region, tiles, tick=110)
                    code, final, _ = self.cli('sample', address=peer.address)
                    self.assertEqual((code, final['goal_status']), (0, 'satisfied'))
                    self.assertNotIn('room_terrain_goal_satisfied_at_sample', final)
                    self.assertEqual(peer.requests, [2, 3] * 2)

    def test_full_retention_replay_refuses_more_reads_but_allows_offline_cancel(self):
        goal = g.RoomTerrainGoal(f.plan(f.dining(1, 1),
            f.dining(1, 1, origin=(132, 13, 2), name='far')), 1000)
        floor = min(goal._selection.floors)
        captured = sample(goal, overrides={floor: f.visible(2)})
        events = [begin_event(goal, captured)]
        for attempt in range(t.MAX_READS):
            captured = sample(goal, 101 + attempt, {floor: f.visible(2)})
            events.extend([{'kind': 'read_started'}, {'kind': 'sample', 'sample': t.sample_value(captured)}])
        raw = history_bytes(events)
        self.path.write_bytes(raw)
        self.path.chmod(0o600)
        budget = t.Budget(60000)
        history = t.replay(raw, budget.checkpoint)
        self.assertEqual((history.attempts, history.events, history.progress.observations), (128, 257, 129))
        self.assertEqual(history.progress.status, 'pending')
        self.assertLess(len(raw), t.ROOM_PROFILE.max_journal)
        self.assertGreater(budget.work_left, 0)
        with patch.dict(os.environ, {}, clear=True), patch.object(t, 'RoomMapClient', side_effect=AssertionError('read')):
            self.assertRaises(ValueError, t.sample, self.path, 60000)
            self.assertEqual(self.path.read_bytes(), raw)
            result = t.cancel(self.path, 60000)
            self.assertEqual((result['goal_status'], result['journal_events']), ('cancelled', 258))
            self.assertEqual(result['goal']['room_plan'], goal.room_plan.json())



if __name__ == '__main__':
    unittest.main()
