"""Actual blueprint CLI/journal integration with joined TCP and private POSIX files."""
from dataclasses import replace
import hashlib
import json
import os
from pathlib import Path
import stat
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import excavation_blueprint as b
import excavation_observer as e
import track_excavation as t
from test_excavation_blueprint import capture, part, goal
from test_excavation_observer import Peer, TOKEN


class BlueprintPeer(Peer):
    """Fragmented real TCP; independently checks the entire requested enclosure."""
    def __init__(self, region, scenarios):
        self.region = region
        super().__init__(scenarios=[(c.raw, c.manifest, fault) for c, fault in scenarios])

    def serve_one(self):
        try:
            sock, _ = self.sock.accept()
            self.connection = sock
            sock.settimeout(3)
            with sock:
                assert self.exact(sock, 12) == b'DFHack?\n\x01\0\0\0'
                self.send(b'DFHack!\n\x01\0\0\0')
                for index, name in enumerate(('Handshake', 'ReadObservation')):
                    method, n = struct.unpack('<h2xi', self.exact(sock, 8))
                    assert method == 0 and 0 <= n <= 2048
                    assert e.decode(self.exact(sock, n), 11) == {
                        1: name.encode(), 2: b'dfmcp.map.v1_5.Request',
                        3: b'dfmcp.map.v1_5.Reply', 4: b'dfmcp_map_v1_5'}
                    self.reply({1: index + 2})
                nonce = None
                for method_id, name in ((2, 'Handshake'), (3, 'ReadObservation')):
                    method, n = struct.unpack('<h2xi', self.exact(sock, 8))
                    assert method == method_id and 0 <= n <= 2048
                    fields = e.decode(self.exact(sock, n), 11)
                    assert set(fields) == set(range(1, 12))
                    assert fields[1] == TOKEN and len(fields[2]) == 32
                    assert nonce is None or fields[2] == nonce
                    nonce = fields[2]
                    assert (fields[3], fields[4]) == (1, 5)
                    assert tuple(fields[k] for k in range(5, 11)) == self.region.origin + self.region.size
                    assert fields[11] == max(1024, 575 + 20 * self.region.volume)
                    self.calls.append(name)
                    source = self.manifest
                    reply = {1: 1, 2: 0, 3: nonce, 4: 1, 5: 5, 6: source.generation,
                             7: source.df_version.encode(), 8: source.dfhack_version.encode()}
                    if method == 3:
                        if self.fault == 'drop':
                            return
                        reply[9] = self.raw
                    self.reply(reply)
                # No third invocation, mutation or implicit retry is allowed.
                assert sock.recv(1) == b''
        except (EOFError, BrokenPipeError, ConnectionResetError):
            pass
        except BaseException as cause:
            self.errors.append(cause)


class BlueprintTrackerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='blueprint-goal-')
        self.root = Path(self.temp.name).resolve()
        self.root.chmod(0o700)
        self.path = self.root / 'goal.jsonl'
        self.spec = self.root / 'blueprint.json'
        self.blueprint = b.Blueprint((part(size=(3, 2, 1)),
            part(origin=(13, 21, 30), shape='stair_up'),
            part(origin=(13, 21, 31), shape='stair_down')))
        self.spec.write_bytes(e.canonical(self.blueprint.json()))

    def tearDown(self):
        self.temp.cleanup()

    @staticmethod
    def environment(address):
        return {'DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5': '1',
                'DFMCP_MAP_TOKEN': TOKEN.decode(), 'DFMCP_MAP_ENDPOINT': address}

    def observed(self, tick=100, matching=True, **kwargs):
        cells = {index: (2, shape if matching else 2, 0, 0)
                 for index, shape in self.blueprint.targets}
        return capture(self.blueprint.region, tick, cells, **kwargs)

    def seed(self, *, observed=None, address='127.0.0.1:5000', definition=None):
        definition = definition or goal(self.blueprint)
        observed = observed or self.observed()
        with t.open_journal(self.path, t.Budget(10000), writable=True, create=True) as journal:
            journal.append({'kind': 'begin', 'format': b.GOAL_FORMAT, 'nonce': 'ab' * 32,
                            'endpoint': address, 'goal': definition.json(), 'sample': t.sample_value(observed)})
        return self.path.read_bytes()

    def sample(self, address):
        with patch.dict(os.environ, self.environment(address), clear=True):
            return t.sample(self.path, timeout_ms=5000)

    def command(self, command, address=None, *extra):
        args = [sys.executable, t.__file__, command, '--journal', str(self.path), *extra]
        if command == 'start-blueprint':
            args += ['--blueprint', str(self.spec), '--world-folder', 'region1', '--site', '1',
                     '--max-game-ticks', '400']
        result = subprocess.run(args, env=self.environment(address) if address else {},
                                capture_output=True, text=True, timeout=8)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(result.stderr, '')
        self.assertEqual(len(result.stdout.splitlines()), 1)
        self.assertNotIn(TOKEN.decode(), result.stdout)
        self.assertLessEqual(len(result.stdout.rstrip('\n').encode()), t.MAX_OUTPUT)
        return json.loads(result.stdout)

    def test_actual_multilevel_cli_restart_and_terminal_offline_reads(self):
        samples = [(self.observed(matching=False), None), (self.observed(110), None), (self.observed(120), None)]
        with BlueprintPeer(self.blueprint.region, samples) as peer:
            first = self.command('start-blueprint', peer.address)
            self.assertEqual(first['schema'], 'dfmcp.excavation-blueprint-progress/1')
            self.assertEqual(first['goal_status'], 'pending')
            self.assertEqual(first['blueprint_at_last_observation']['target_tiles'], 8)
            self.assertEqual(first['agent_turn']['phase'], 'bootstrap')
            self.assertEqual(first['agent_turn']['coverage']['scope'], self.blueprint.region.json())
            self.spec.unlink()  # Recovery depends on retained content, not a mutable input path.
            self.assertEqual(self.command('sample', peer.address)['goal_status'], 'stabilizing')
            final = self.command('sample', peer.address)
        self.assertEqual(final['goal_status'], 'satisfied')
        self.assertTrue(final['blueprint_goal_satisfied_at_sample'])
        self.assertFalse(final['floor_goal_satisfied_at_sample'])
        self.assertEqual(final['agent_turn']['active_work']['obligations'], [])
        self.assertEqual(peer.calls, ['Handshake', 'ReadObservation'] * 3)
        raw = self.path.read_bytes()
        for operation in ('sample', 'inspect', 'cancel'):
            result = self.command(operation)
            self.assertEqual(result['journal_head'], final['journal_head'])
            self.assertEqual(result['native_reads_attempted'], 0)
            self.assertFalse(result['game_mutations_dispatched'])
            self.assertFalse(result['mining_action_completed_proven'])
        self.assertEqual(self.path.read_bytes(), raw)
        self.assertNotIn(str(self.spec).encode(), raw)
        self.assertNotIn(TOKEN, raw)

    def test_unfinished_and_failed_reads_reset_the_whole_blueprint_streak(self):
        samples = [(self.observed(110), 'drop'), (self.observed(120), None), (self.observed(130), None)]
        with BlueprintPeer(self.blueprint.region, samples) as peer:
            self.seed(address=peer.address)
            with t.open_journal(self.path, t.Budget(10000), writable=True) as journal:
                journal.append({'kind': 'read_started'})
            interrupted = t.inspect(self.path)
            self.assertEqual((interrupted['goal_status'], interrupted['matching_samples']), ('unknown', 0))
            self.assertTrue(interrupted['pending_read'])
            self.assertFalse(self.sample(peer.address)['ok'])
            self.assertEqual(t.inspect(self.path)['interruption'], 'read_failed')
            self.assertEqual(self.sample(peer.address)['matching_samples'], 1)
            self.assertEqual(self.sample(peer.address)['goal_status'], 'satisfied')

    def test_profile_domain_and_goal_format_substitution_are_rejected(self):
        raw = self.seed()
        first = json.loads(raw)['event']
        self.assertEqual(raw, t.frame(first, 0, '0' * 64, t.BLUEPRINT_PROFILE))
        self.assertNotEqual(raw, t.frame(first, 0, '0' * 64))
        with self.assertRaises(e.Rejected):
            t.replay(t.frame(first, 0, '0' * 64))
        for name in ('dfmcp.excavation-goal/1', 'dfmcp.excavation-blueprint-goal/2'):
            event = dict(first, format=name)
            with self.assertRaises(e.Rejected):
                t.replay(t.frame(event, 0, '0' * 64, t.BLUEPRINT_PROFILE))
        h = t.replay(raw)
        with self.assertRaises(e.Rejected):
            t.replay(raw + t.frame({'kind': 'read_started'}, h.events, h.head))
        with self.assertRaises(e.Rejected):
            t.replay(raw + t.frame({'kind': 'sample', 'sample': t.sample_value(self.observed(110))},
                                  h.events, h.head, h.profile))
        event = dict(first, goal=dict(first['goal'], status='satisfied'))
        with self.assertRaises(e.Rejected):
            t.replay(t.frame(event, 0, '0' * 64, t.BLUEPRINT_PROFILE))

    def test_legacy_frames_keep_the_original_checksum_and_limits(self):
        legacy = e.Goal(e.Region((10, 20, 30), (1, 1, 1)), 'region1', 1, 1000)
        event = {'kind': 'begin', 'format': 'dfmcp.excavation-goal/1', 'nonce': 'ab' * 32,
                 'endpoint': '127.0.0.1:5000', 'goal': legacy.json(),
                 'sample': t.sample_value(capture(legacy.region))}
        payload = {'event': event, 'sequence': 0, 'previous': '0' * 64}
        digest = hashlib.sha256(b'dfmcp-excavation-journal/1\0' + e.canonical(payload)).hexdigest()
        expected = e.canonical(dict(payload, sha256=digest)) + b'\n'
        self.assertEqual(t.frame(event, 0, '0' * 64), expected)
        self.assertIs(t.replay(expected).profile, t.FLOOR_PROFILE)
        self.assertEqual((t.FLOOR_PROFILE.max_frame, t.FLOOR_PROFILE.max_journal, t.FLOOR_PROFILE.max_sample),
                         (16384, 2 * 1024 * 1024, 2048))
        # The wider outer decoder must not widen a declared legacy journal.
        with self.assertRaises(e.Rejected):
            t.replay(expected + b' ' * t.FLOOR_PROFILE.max_journal + b'\n')
        with self.assertRaises(e.Rejected):
            t.sample_decode({'manifest': capture(legacy.region).manifest.json(), 'capture_hex': '00' * 2049}, legacy.region)

    def test_large_coherent_capture_fits_new_profile_without_clipping(self):
        self.blueprint = b.Blueprint((part(origin=(0, 0, 0), size=(8, 8, 7)), part(origin=(7, 7, 15))))
        self.spec.write_bytes(e.canonical(self.blueprint.json()))
        self.assertEqual(self.blueprint.region.volume, 1024)
        initial = self.observed()
        self.assertGreater(len(initial.raw), t.FLOOR_PROFILE.max_sample)
        with BlueprintPeer(self.blueprint.region, [(initial, None), (self.observed(110), None)]) as peer:
            self.command('start-blueprint', peer.address)
            self.assertEqual(self.command('sample', peer.address)['goal_status'], 'satisfied')
        self.assertEqual(t.inspect(self.path)['blueprint_at_last_observation']['captured_tiles'], 1024)
        self.assertEqual(len(t.replay(self.path.read_bytes()).progress.latest.tiles), 1024)

    def test_maximal_targets_parts_strings_and_result_fit_exact_bounds(self):
        self.blueprint = b.Blueprint(tuple(part(origin=(32704 + 2 * i, 32760, 32766), size=(1, 8, 2))
                                         for i in range(32)))
        self.assertEqual(len(self.blueprint.targets), 512)
        source = e.Manifest(2**64 - 2, '\1' * 128, '\1' * 128)
        folder = '\1' * 512
        definition = b.BlueprintGoal(self.blueprint, folder, 2**31 - 1, e.MAX_TICK, 403200, 128, 403200)
        value = self.observed(e.MAX_TICK - 403200, False, folder=folder, site=2**31 - 1, manifest=source)
        raw = self.seed(definition=definition, observed=value)
        self.assertLessEqual(len(raw), t.BLUEPRINT_PROFILE.max_frame)
        self.assertLessEqual(len(value.raw), t.BLUEPRINT_PROFILE.max_sample)
        result = t.inspect(self.path)
        result['native_reads_attempted'] = 1
        self.assertEqual(result['blueprint_at_last_observation']['remaining_omitted'], 496)
        for operation in ('start-blueprint', 'sample', 'inspect', 'cancel'):
            self.assertLessEqual(len(t.encode_result(result, operation)), t.MAX_OUTPUT)
        self.assertEqual(result['counts_at_last_observation']['mismatched'], 512)

    def test_read_attempt_limit_reserves_cancellation_for_blueprints(self):
        raw = self.seed(observed=self.observed(matching=False))
        h = t.replay(raw)
        previous = h.head
        # Produce a full valid 257-frame journal without quadratic disk rewriting.
        for sequence in range(1, 257):
            event = {'kind': 'read_started'} if sequence % 2 else {'kind': 'read_failed'}
            addition = t.frame(event, sequence, previous, h.profile)
            previous = json.loads(addition)['sha256']
            raw += addition
        self.path.write_bytes(raw)
        with patch.object(e, 'MapClient', side_effect=AssertionError('native access')):
            with self.assertRaises(e.Rejected):
                self.sample('127.0.0.1:5000')
            self.assertEqual(t.cancel(self.path)['goal_status'], 'cancelled')
        self.assertTrue(self.path.read_bytes().startswith(raw))
        self.assertEqual(t.replay(self.path.read_bytes()).attempts, 128)

    def test_bad_input_refuses_before_journal_creation_or_native_access(self):
        for raw in (b'{}', b'[' * 100 + b']' * 100, b' ' * (b.MAX_SPEC_BYTES + 1),
                    e.canonical(dict(self.blueprint.json(), commit=True))):
            self.spec.write_bytes(raw)
            with patch.object(e, 'MapClient', side_effect=AssertionError('native access')):
                with self.assertRaises(e.Rejected):
                    t.start_blueprint(self.path, self.spec, 'region1', 1, 100)
            self.assertFalse(self.path.exists())
        with self.assertRaises(e.Rejected):
            t.replay(b'\n' * 300)
        with self.assertRaises(e.Rejected):
            t.decode_record(b'[' * 100 + b']' * 100)

    def test_input_symlink_fifo_directory_and_changed_bytes_are_refused(self):
        for kind in ('symlink', 'fifo', 'directory'):
            path = self.root / kind
            if kind == 'symlink':
                path.symlink_to(self.spec)
            elif kind == 'fifo':
                os.mkfifo(path)
            else:
                path.mkdir()
            with self.assertRaises((e.Rejected, OSError)):
                t.read_blueprint(path, t.Budget(5000))
        original_read = os.read
        changed = []
        def read(fd, size):
            result = original_read(fd, size)
            if not changed:
                changed.append(True)
                self.spec.write_bytes(self.spec.read_bytes().replace(b'floor', b'empty', 1))
            return result
        with patch.object(t.os, 'read', side_effect=read), self.assertRaises(e.Rejected):
            t.read_blueprint(self.spec, t.Budget(5000))

    def test_blueprint_journal_custody_and_cross_process_lock(self):
        raw = self.seed()
        with t.open_journal(self.path, t.Budget(5000)):
            result = subprocess.run([sys.executable, t.__file__, 'inspect', '--journal', str(self.path)],
                                    env={}, capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 2)
        for mode in (0o400, 0o640):
            self.path.chmod(mode)
            with self.assertRaises(e.Rejected):
                t.inspect(self.path)
        self.path.chmod(0o600)
        alias = self.root / 'alias'
        alias.symlink_to(self.path)
        with self.assertRaises((e.Rejected, OSError)):
            t.inspect(alias)
        os.link(self.path, self.root / 'hardlink')
        with self.assertRaises(e.Rejected):
            t.inspect(self.path)
        self.assertEqual(self.path.read_bytes(), raw)

    def test_read_intent_sync_faults_do_not_start_native_work(self):
        actual = os.fsync
        for failure in range(1, 5):
            self.path = self.root / f'failure-{failure}.jsonl'
            self.seed()
            calls = []
            def sync(fd):
                calls.append(stat.S_ISDIR(os.fstat(fd).st_mode))
                if len(calls) == failure:
                    raise OSError('injected sync fault')
                actual(fd)
            with patch.object(t.os, 'fsync', side_effect=sync), \
                 patch.object(e, 'MapClient', side_effect=AssertionError('native access')):
                with self.assertRaises(OSError):
                    self.sample('127.0.0.1:5000')
            self.assertEqual(calls, ([False, True] * 2)[:failure])
            self.assertEqual(t.inspect(self.path)['pending_read'], failure >= 3)

    def test_terminal_sync_faults_never_acknowledge_but_keep_historical_evidence(self):
        actual = os.fsync
        for failure in (5, 6):
            self.path = self.root / f'final-{failure}.jsonl'
            with BlueprintPeer(self.blueprint.region, [(self.observed(110), None)]) as peer:
                self.seed(address=peer.address)
                calls = []
                def sync(fd):
                    calls.append(fd)
                    if len(calls) == failure:
                        raise OSError('injected terminal sync fault')
                    actual(fd)
                with patch.object(t.os, 'fsync', side_effect=sync), self.assertRaises(OSError):
                    self.sample(peer.address)
            historical = t.inspect(self.path)
            self.assertEqual(historical['goal_status'], 'satisfied')
            self.assertTrue(historical['evidence_is_historical'])
            self.assertFalse(historical['current_conditions_proven'])

    def test_torn_sample_fences_without_repair_and_output_refusal_precedes_write(self):
        raw = self.seed()
        with t.open_journal(self.path, t.Budget(10000), writable=True) as journal:
            with patch.object(t, 'MAX_OUTPUT', 1), self.assertRaises(e.Rejected):
                journal.append({'kind': 'read_started'})
            self.assertEqual(self.path.read_bytes(), raw)
        actual = os.write
        writes = []
        def write(fd, data):
            if writes or b'"kind":"sample"' in bytes(data):
                writes.append(fd)
                if len(writes) == 1:
                    return actual(fd, data[:41])
                raise OSError('injected torn sample')
            return actual(fd, data)
        with BlueprintPeer(self.blueprint.region, [(self.observed(110), None)]) as peer:
            # A fresh journal binds the actual peer endpoint.
            self.path = self.root / 'torn.jsonl'
            self.seed(address=peer.address)
            with patch.object(t.os, 'write', side_effect=write), self.assertRaises(OSError):
                self.sample(peer.address)
        retained = self.path.read_bytes()
        with patch.object(e, 'MapClient', side_effect=AssertionError('native access')):
            for operation in (t.inspect, t.sample, t.cancel):
                with self.assertRaises((ValueError, e.Rejected)):
                    operation(self.path)
        self.assertEqual(retained, self.path.read_bytes())

    def test_source_change_and_original_deadline_survive_reopen(self):
        for name, value, expected in (
            ('generation', self.observed(110, manifest=e.Manifest(8, 'test-df', 'test-dfhack')), 'invalidated'),
            ('clock', self.observed(99), 'invalidated'), ('deadline', self.observed(1001), 'expired')):
            self.path = self.root / f'{name}.jsonl'
            with BlueprintPeer(self.blueprint.region, [(value, None)]) as peer:
                self.seed(address=peer.address)
                self.assertEqual(self.sample(peer.address)['goal_status'], expected)
            result = self.command('sample')
            self.assertEqual(result['goal_status'], expected)
            self.assertEqual(result['goal']['deadline_tick'], 1000)
            self.assertFalse(result['blueprint_goal_satisfied_at_sample'])

    def test_local_cancel_and_bad_environment_cannot_dispatch(self):
        original = self.seed()
        with patch.object(e, 'MapClient', side_effect=AssertionError('native access')):
            for name in ('DFMCP_DIG_TOKEN', 'DFMCP_ADMITTED_BRIDGE_PROTOCOL', 'DFMCP_DIG_ALLOW_DESIGNATE'):
                with patch.dict(os.environ, dict(self.environment('127.0.0.1:5000'), **{name: '1'}), clear=True):
                    with self.assertRaises(e.Rejected):
                        t.sample(self.path)
            self.assertEqual(self.path.read_bytes(), original)
            first = t.cancel(self.path)
            second = t.cancel(self.path)
            self.assertEqual(first['journal_head'], second['journal_head'])
            self.assertEqual(first['goal_status'], 'cancelled')
            self.assertFalse(first['native_effect_obligations_changed'])

    def test_initial_horizon_overflow_and_output_refusal_never_publish_a_goal(self):
        with BlueprintPeer(self.blueprint.region, [(self.observed(e.MAX_TICK), None)]) as peer:
            with patch.dict(os.environ, self.environment(peer.address), clear=True), self.assertRaises(e.Rejected):
                t.start_blueprint(self.path, self.spec, 'region1', 1, 100)
        self.assertEqual(self.path.read_bytes(), b'')
        with self.assertRaises(e.Rejected):
            t.inspect(self.path)  # Existing failed initialization is not auto-repaired.
        self.path = self.root / 'no-output.jsonl'
        with BlueprintPeer(self.blueprint.region, [(self.observed(), None)]) as peer:
            with patch.dict(os.environ, self.environment(peer.address), clear=True), \
                 patch.object(t, 'MAX_OUTPUT', 1), self.assertRaises(e.Rejected):
                t.start_blueprint(self.path, self.spec, 'region1', 1, 100)
        self.assertEqual(self.path.read_bytes(), b'')


if __name__ == '__main__':
    unittest.main()
