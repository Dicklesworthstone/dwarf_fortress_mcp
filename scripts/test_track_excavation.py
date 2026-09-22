#!/usr/bin/env python3
"""Actual goal-journal/CLI/fault tests with real files and joined loopback peers."""
from __future__ import annotations

from contextlib import redirect_stdout
import io
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import excavation_observer as e
import track_excavation as t
from test_excavation_observer import MANIFEST, REGION, TOKEN, Peer, raw_capture, visible, capture, goal


class TrackerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='excavation-goal-')
        self.root = Path(self.temp.name).resolve()
        self.root.chmod(0o700)
        self.path = self.root / 'goal.jsonl'

    def tearDown(self):
        self.temp.cleanup()

    @staticmethod
    def env(address):
        return {'DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5': '1', 'DFMCP_MAP_TOKEN': TOKEN.decode(),
                'DFMCP_MAP_ENDPOINT': address}

    def begin(self, address):
        with patch.dict(os.environ, self.env(address), clear=True):
            return t.start(self.path, REGION, 'region1', 1, 400, timeout_ms=3000)

    def sample(self, address):
        with patch.dict(os.environ, self.env(address), clear=True):
            return t.sample(self.path, timeout_ms=3000)

    def seed(self, cells=None, address='127.0.0.1:5000'):
        with t.open_journal(self.path, t.Budget(10000), writable=True, create=True) as journal:
            journal.append({'kind': 'begin', 'format': 'dfmcp.excavation-goal/1', 'nonce': 'ab' * 32,
                'endpoint': address, 'goal': goal().json(), 'sample': t.sample_value(capture(cells=cells))})
        return self.path.read_bytes()

    def test_walls_to_floors_stable_goal_survives_restart(self):
        samples = [(raw_capture(100, cells=[visible(2, 0, 1)] * 4), MANIFEST, None),
                   (raw_capture(110), MANIFEST, None), (raw_capture(120), MANIFEST, None)]
        with Peer(scenarios=samples) as peer:
            first = self.begin(peer.address)
            self.assertEqual(first['goal_status'], 'pending')
            self.assertEqual(first['counts_at_last_observation']['active_designations'], 4)
            self.assertEqual(self.sample(peer.address)['goal_status'], 'stabilizing')
            final = self.sample(peer.address)
        self.assertEqual(final['goal_status'], 'satisfied')
        self.assertTrue(final['floor_goal_satisfied_at_sample'])
        original = self.path.read_bytes()
        with patch.dict(os.environ, {}, clear=True), patch.object(e, 'MapClient', side_effect=AssertionError('native access')):
            for operation in (t.inspect, t.sample, t.cancel):
                result = operation(self.path)
                self.assertEqual(result['journal_head'], final['journal_head'])
                self.assertEqual(result['goal_status'], 'satisfied')
                self.assertEqual(result['native_reads_attempted'], 0)
                self.assertFalse(result['mining_action_completed_proven'])
                self.assertFalse(result['continuous_stability_proven'])
                self.assertFalse(result['game_mutations_dispatched'])
        self.assertEqual(self.path.read_bytes(), original)
        self.assertEqual(peer.calls, ['Handshake', 'ReadObservation'] * 3)

    def test_unfinished_read_is_visible_and_resets_recovered_streak(self):
        with Peer(scenarios=[(raw_capture(120), MANIFEST, None), (raw_capture(130), MANIFEST, None)]) as peer:
            self.seed(address=peer.address)
            with t.open_journal(self.path, t.Budget(10000), writable=True) as journal:
                journal.append({'kind': 'read_started'})
            before = t.inspect(self.path)
            self.assertTrue(before['pending_read'])
            self.assertEqual((before['goal_status'], before['matching_samples']), ('unknown', 0))
            first = self.sample(peer.address)
            self.assertEqual((first['goal_status'], first['matching_samples']), ('stabilizing', 1))
            self.assertEqual(self.sample(peer.address)['goal_status'], 'satisfied')

    def test_failed_native_read_is_durable_not_successful_stability(self):
        with Peer(scenarios=[(raw_capture(110), MANIFEST, 'drop'), (raw_capture(120), MANIFEST, None)]) as peer:
            self.seed(address=peer.address)
            failed = self.sample(peer.address)
            self.assertFalse(failed['ok'])
            self.assertEqual(failed['goal_status'], 'unknown')
            self.assertEqual(t.inspect(self.path)['matching_samples'], 0)
            again = self.sample(peer.address)
            self.assertEqual((again['goal_status'], again['matching_samples']), ('stabilizing', 1))

    def test_generation_change_invalidates_without_silently_rebinding(self):
        with Peer(manifest=e.Manifest(8, 'df', 'dfhack'), raw=raw_capture(110)) as peer:
            self.seed(address=peer.address)
            result = self.sample(peer.address)
        self.assertEqual(result['goal_status'], 'invalidated')
        self.assertEqual(result['source']['manifest']['generation'], 7)
        with patch.object(e, 'MapClient', side_effect=AssertionError('native access')):
            self.assertEqual(t.sample(self.path)['goal_status'], 'invalidated')

    def test_deadline_is_not_renewed_on_reopen(self):
        with Peer(raw=raw_capture(501)) as peer:
            self.seed(address=peer.address)
            self.assertEqual(self.sample(peer.address)['goal_status'], 'expired')
        self.assertEqual(t.inspect(self.path)['goal']['deadline_tick'], 500)

    def test_repeat_creation_and_mixed_profiles_cannot_start_native_work(self):
        self.seed()
        original = self.path.read_bytes()
        with patch.object(e, 'MapClient', side_effect=AssertionError('native access')):
            with self.assertRaises(FileExistsError):
                self.begin('127.0.0.1:5000')
            for extra in ('DFMCP_DIG_ALLOW_DESIGNATE', 'DFMCP_ADMITTED_BRIDGE_PROTOCOL', 'DFMCP_DIG_TOKEN'):
                with patch.dict(os.environ, {**self.env('127.0.0.1:5000'), extra: '1'}, clear=True):
                    with self.assertRaises(e.Rejected):
                        t.sample(self.path)
        self.assertEqual(self.path.read_bytes(), original)

    def test_read_intent_sync_failure_prevents_connection(self):
        real_sync = os.fsync
        for failure in (1, 2, 3, 4):
            self.path = self.root / f'pre-sync-{failure}.jsonl'
            self.seed()
            count = []
            def sync(fd):
                count.append(fd)
                if len(count) == failure:
                    raise OSError('injected sync failure')
                real_sync(fd)
            with patch.object(e, 'MapClient', side_effect=AssertionError('native access')):
                with patch.object(t.os, 'fsync', side_effect=sync), self.assertRaises(OSError):
                    self.sample('127.0.0.1:5000')
            self.assertEqual(len(count), failure)
            before = t.inspect(self.path)
            self.assertFalse(before['floor_goal_satisfied_at_sample'])
            self.assertEqual(before['pending_read'], failure >= 3)

    def test_terminal_file_and_directory_sync_failures_cannot_acknowledge(self):
        real_sync = os.fsync
        for failure in (5, 6):
            self.path = self.root / f'terminal-sync-{failure}.jsonl'
            count = []
            def sync(fd):
                count.append(stat.S_ISDIR(os.fstat(fd).st_mode))
                if len(count) == failure:
                    raise OSError('injected terminal sync failure')
                real_sync(fd)
            with Peer(raw=raw_capture(110)) as peer:
                self.seed(address=peer.address)
                with patch.object(t.os, 'fsync', side_effect=sync), self.assertRaises(OSError):
                    self.sample(peer.address)
            self.assertEqual(count, ([False, True] * 3)[:failure])
            # A complete frame can survive an uncertain sync. Offline inspection
            # verifies historical bytes, not the failed call's durability.
            recovered = t.inspect(self.path)
            self.assertEqual(recovered['goal_status'], 'satisfied')
            self.assertTrue(recovered['evidence_is_historical'])

    def test_torn_sample_is_retained_without_repair_or_further_native_reads(self):
        real_write = os.write
        writing = []
        def write(fd, raw):
            if writing or b'"kind":"sample"' in bytes(raw):
                writing.append(fd)
                if len(writing) == 1:
                    return real_write(fd, raw[:31])
                raise OSError('torn sample')
            return real_write(fd, raw)
        with Peer(raw=raw_capture(110)) as peer:
            self.seed(address=peer.address)
            with patch.object(t.os, 'write', side_effect=write), self.assertRaises(OSError):
                self.sample(peer.address)
        original = self.path.read_bytes()
        with patch.object(e, 'MapClient', side_effect=AssertionError('native access')):
            for operation in (t.inspect, t.sample, t.cancel):
                with self.assertRaises((e.Rejected, ValueError)):
                    operation(self.path)
        self.assertEqual(self.path.read_bytes(), original)

    def test_partial_header_and_unrelated_empty_files_are_not_initialized(self):
        self.path.write_bytes(b'')
        self.path.chmod(0o600)
        for content in (b'', b'{"event":', b'{}\n'):
            self.path.write_bytes(content)
            with self.assertRaises((e.Rejected, ValueError)):
                t.inspect(self.path)
            self.assertEqual(self.path.read_bytes(), content)

    def test_journal_no_follow_links_modes_and_special_files(self):
        self.seed()
        original = self.root / 'original'
        self.path.rename(original)
        self.path.symlink_to(original)
        with self.assertRaises((e.Rejected, OSError)):
            t.inspect(self.path)
        self.path.unlink()
        os.link(original, self.path)
        with self.assertRaises(e.Rejected):
            t.inspect(self.path)
        self.path.unlink()
        os.mkfifo(self.path, 0o600)
        with self.assertRaises(e.Rejected):
            t.inspect(self.path)
        self.path.unlink()
        original.rename(self.path)
        for mode in (0o400, 0o640, 0o666):
            self.path.chmod(mode)
            with self.assertRaises(e.Rejected):
                t.inspect(self.path)
        self.path.chmod(0o600)
        self.root.chmod(0o750)
        with self.assertRaises(e.Rejected):
            t.inspect(self.path)
        self.root.chmod(0o700)
        alias = self.root / 'alias'
        alias.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(OSError):
            t.inspect(alias / self.path.name)

    def test_real_cross_process_exclusive_lock(self):
        self.seed()
        command = [sys.executable, t.__file__, 'inspect', '--journal', str(self.path)]
        with t.open_journal(self.path, t.Budget(10000)):
            result = subprocess.run(command, env={}, capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 2, result.stderr)
        result = subprocess.run(command, env={}, capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_same_length_corruption_and_inode_substitution_fence_live_journal(self):
        original = self.seed()
        with t.open_journal(self.path, t.Budget(10000), writable=True) as journal:
            self.path.write_bytes(original.replace(b'region1', b'region2', 1))
            with self.assertRaises(e.Rejected):
                journal.append({'kind': 'read_started'})
            self.assertTrue(journal.fenced)
        self.path.write_bytes(original)
        with t.open_journal(self.path, t.Budget(10000), writable=True) as journal:
            self.path.rename(self.root / 'saved')
            self.path.write_bytes(original)
            self.path.chmod(0o600)
            with self.assertRaises(e.Rejected):
                journal.verify()
            self.assertTrue(journal.fenced)

    def test_rehashed_invalid_events_do_not_forge_goal_completion(self):
        raw = self.seed()
        history = t.replay(raw)
        for event in ({'kind': 'satisfied'}, {'kind': 'sample', 'sample': t.sample_value(capture(120))},
                      {'kind': 'read_started', 'status': 'satisfied'}, {'kind': 'cancel', 'undo': True}):
            with self.assertRaises(e.Rejected):
                t.replay(raw + t.frame(event, history.events, history.head))
        with t.open_journal(self.path, t.Budget(10000), writable=True) as journal:
            journal.append({'kind': 'cancel'})
        cancelled = self.path.read_bytes()
        history = t.replay(cancelled)
        with self.assertRaises(e.Rejected):
            t.replay(cancelled + t.frame({'kind': 'read_started'}, history.events, history.head))

    def test_all_byte_corruptions_and_non_boundary_prefixes_fail(self):
        self.seed()
        with t.open_journal(self.path, t.Budget(10000), writable=True) as journal:
            journal.append({'kind': 'read_started'})
            journal.append({'kind': 'sample', 'sample': t.sample_value(capture(110))})
        raw = self.path.read_bytes()
        boundaries = set()
        offset = 0
        for line in raw.splitlines(keepends=True):
            offset += len(line)
            boundaries.add(offset)
        for size in range(len(raw)):
            if size in boundaries:
                t.replay(raw[:size])
            else:
                with self.assertRaises((e.Rejected, ValueError, KeyError, TypeError)):
                    t.replay(raw[:size])
        for index in range(len(raw)):
            changed = bytearray(raw)
            changed[index] ^= 1
            with self.assertRaises((e.Rejected, ValueError, KeyError, TypeError)):
                t.replay(bytes(changed))

    def test_retention_bound_refuses_reads_but_allows_local_cancellation(self):
        with Peer(raw=raw_capture(105, cells=[visible(2)] * 4)) as peer:
            self.seed(address=peer.address)
            with patch.object(t, 'MAX_READS', 1):
                self.sample(peer.address)
                with patch.object(e, 'MapClient', side_effect=AssertionError('native access')):
                    with self.assertRaises(e.Rejected):
                        self.sample(peer.address)
                    result = t.cancel(self.path)
                    self.assertEqual(result['goal_status'], 'cancelled')
                    self.assertFalse(result['native_effect_obligations_changed'])

    def test_worst_case_header_and_output_are_bounded(self):
        region = e.Region((32759, 32759, 32766), (8, 8, 1))
        source = e.Manifest(2**64 - 2, '\1' * 128, '\1' * 128)
        folder = '\1' * 512
        raw = raw_capture(e.MAX_TICK - 100, region=region, folder=folder, site=2**31 - 1,
                          dimensions=(32768, 32768, 32768),
                          cells=[visible(tiletype=2**32 - 1, walkable=2**32 - 1,
                                         temperature1=65535, temperature2=65535)] * 64)
        value = e.decode_capture(raw, source, region)
        g = e.Goal(region, folder, 2**31 - 1, e.MAX_TICK)
        with t.open_journal(self.path, t.Budget(10000), writable=True, create=True) as journal:
            journal.append({'kind': 'begin', 'format': 'dfmcp.excavation-goal/1', 'nonce': 'ab' * 32,
                'endpoint': '127.0.0.1:65535', 'goal': g.json(), 'sample': t.sample_value(value)})
            self.assertLess(len(journal.raw), t.MAX_FRAME)
        self.assertLess(len(t.encode_result(t.inspect(self.path))), t.MAX_OUTPUT)

    def test_actual_subprocess_cli_start_sample_and_offline_inspect(self):
        with Peer(scenarios=[(raw_capture(100), MANIFEST, None), (raw_capture(110), MANIFEST, None)]) as peer:
            env = self.env(peer.address)
            common = [sys.executable, t.__file__]
            command = common + ['start', '--journal', str(self.path), '--x', '15', '--y', '15', '--z', '2',
                '--width', '2', '--height', '2', '--world-folder', 'region1', '--site', '1', '--max-game-ticks', '400']
            result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            started = json.loads(result.stdout)
            self.assertEqual(started['goal_status'], 'stabilizing')
            self.assertEqual(started['agent_turn']['schema'], 'dfmcp.agent_turn/1')
            self.assertEqual(started['agent_turn']['phase'], 'bootstrap')
            self.assertEqual(len(started['agent_turn']['active_work']['obligations']), 1)
            self.assertIsNone(started['agent_turn']['anchor'])
            result = subprocess.run(common + ['sample', '--journal', str(self.path)],
                                    env=env, capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            completed = json.loads(result.stdout)
            self.assertEqual(completed['goal_status'], 'satisfied')
            self.assertEqual(completed['agent_turn']['phase'], 'verify')
            self.assertEqual(completed['agent_turn']['active_work']['obligations'], [])
            self.assertFalse(completed['agent_turn']['active_work']['native_effect_inventory_verified'])
            self.assertNotIn(TOKEN.decode(), result.stdout)
        result = subprocess.run(common + ['inspect', '--journal', str(self.path)],
                                env={}, capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(json.loads(result.stdout)['native_reads_attempted'], 0)

    def test_cancel_stops_monitor_only_and_is_idempotent_without_environment(self):
        original = self.seed()
        with patch.dict(os.environ, {}, clear=True), patch.object(e, 'MapClient', side_effect=AssertionError('native access')):
            first = t.cancel(self.path)
            after = self.path.read_bytes()
            self.assertEqual(t.cancel(self.path)['journal_head'], first['journal_head'])
        self.assertEqual(first['goal_status'], 'cancelled')
        self.assertFalse(first['floor_goal_satisfied_at_sample'])
        self.assertTrue(after.startswith(original))
        self.assertEqual(self.path.read_bytes(), after)

    def test_endpoint_change_and_exhausted_deadline_refuse_before_native_work(self):
        self.seed()
        original = self.path.read_bytes()
        with patch.object(e, 'MapClient', side_effect=AssertionError('native access')):
            with self.assertRaises(e.Rejected):
                self.sample('127.0.0.1:5001')
            with self.assertRaises(e.Rejected):
                t.sample(self.path, timeout_ms=0)
        self.assertEqual(self.path.read_bytes(), original)

    def test_cli_failure_never_prints_credentials_or_claims_success(self):
        self.seed()
        out = io.StringIO()
        with patch.dict(os.environ, {'DFMCP_MAP_TOKEN': 'private-secret'}, clear=True), redirect_stdout(out):
            status = t.main(['sample', '--journal', str(self.path)])
        self.assertEqual(status, 2)
        self.assertNotIn('private-secret', out.getvalue())
        value = json.loads(out.getvalue())
        self.assertFalse(value['ok'])
        self.assertFalse(value['retry_designation_permitted'])
        self.assertEqual(value['goal_status'], 'unknown')
        self.assertEqual(value['agent_turn']['schema'], 'dfmcp.agent_turn/1')
        self.assertFalse(value['agent_turn']['active_work']['inventory_verified'])
        self.assertIsNone(value['agent_turn']['request_id'])


if __name__ == '__main__':
    unittest.main(verbosity=2)
