"""Executed Python/POSIX/loopback tests, not DFHack, Rust or power-loss qualification."""
from contextlib import redirect_stdout
import copy
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import bounded_run_client as c
import bounded_run_outcomes as o
from test_bounded_run_client import NativeDouble, TOKEN, intent, reference_record


def result(phase='stopped', reason=None):
    code = c.PHASES.index(phase)
    reason = reason or {'prepared': 'none', 'running': 'none', 'stopping': 'tick_limit',
                        'stopped': 'tick_limit', 'refused': 'cancelled', 'source_lost': 'source_changed'}[phase]
    raw = reference_record(phase=code, reason=c.REASONS.index(reason), attempted=int(code in (1, 2, 3, 5)),
                           paused=int(code == 3), known=int(code in (1, 2, 3)))
    return {'manifest': {'generation': 41, 'df_version': 'fake-df', 'dfhack_version': 'fake-dfhack'},
            'record': c.decode_record(raw), 'owner_active': code in (1, 2), 'retained_records': 1}


class OutcomeTests(unittest.TestCase):
    def setUp(self):
        self.directory = Path(tempfile.mkdtemp(prefix='dfmcp-run-outcomes-'))
        self.directory.chmod(0o700)
        self.path = self.directory / 'run.json'
        self.side = Path(str(self.path) + o.SUFFIX)
        self.create()

    def create(self, address=('127.0.0.1', 5000)):
        with c.capsule(self.path, intent(address)):
            pass

    def cli(self, operation, **extra):
        output = io.StringIO()
        args = [operation, '--record', str(self.path)]
        for key, value in extra.items():
            args += ['--' + key.replace('_', '-'), str(value)]
        with redirect_stdout(output):
            status = c.main(args)
        return status, json.loads(output.getvalue())

    def test_terminal_roundtrip_retains_exact_receipt_and_original_intent(self):
        original = self.path.read_bytes()
        saved = o.retain(self.path, result())
        self.assertTrue(saved['storage_acknowledged_this_call'])
        self.assertEqual(self.path.read_bytes(), original)
        with patch.object(os, 'write', side_effect=AssertionError('offline write')), \
                patch.object(os, 'fsync', side_effect=AssertionError('offline sync')):
            reopened = o.inspect(self.path)
        self.assertEqual(reopened['effect_status'], 'historical_pause_verified')
        self.assertFalse(reopened['storage_acknowledged_this_call'])
        self.assertFalse(reopened['native_contacted'])
        self.assertFalse(reopened['retry_permitted'])
        self.assertFalse(reopened['goal_completion_proved'])
        self.assertTrue(reopened['current_pause_unproved'])
        raw = bytes.fromhex(json.loads(self.side.read_bytes())['outcome']['record_hex'])
        self.assertEqual(raw, reference_record(phase=3, reason=1, attempted=1, paused=1, known=1))

    def test_all_nonterminal_and_absent_results_stay_unresolved(self):
        for phase in ('prepared', 'running', 'stopping', None):
            native = result(phase or 'running')
            if phase is None:
                del native['record']
            view = o.retain(self.path, native)
            self.assertFalse(self.side.exists())
            self.assertFalse(view['terminal_record_retained'])
            self.assertFalse(view['retry_permitted'])
            self.assertEqual(view['native_record_status'], phase or 'absent')

    def test_source_lost_and_refused_are_not_successful_clock_advancement(self):
        for phase, status in (('source_lost', 'indeterminate_source_lost'), ('refused', 'historical_unpause_refused')):
            path = self.directory / (phase + '.json')
            with c.capsule(path, intent()):
                pass
            o.retain(path, result(phase))
            view = o.inspect(path)
            self.assertEqual(view['effect_status'], status)
            self.assertFalse(view['record']['pause_verified'])
            self.assertFalse(view['goal_completion_proved'])
            self.assertFalse(view['retry_permitted'])
            self.assertEqual(view['requires_operator_attention'], phase == 'source_lost')

    def test_every_byte_corruption_and_every_incomplete_prefix_refused_unchanged(self):
        o.retain(self.path, result())
        raw = self.side.read_bytes()
        for index in range(len(raw)):
            for bad in (raw[:index] + bytes([raw[index] ^ 1]) + raw[index + 1:], raw[:index]):
                self.side.write_bytes(bad)
                with self.assertRaises((ValueError, TypeError, KeyError)):
                    o.inspect(self.path)
                self.assertEqual(self.side.read_bytes(), bad)
        self.side.write_bytes(raw)
        self.assertTrue(o.inspect(self.path)['terminal_record_retained'])

    def test_rehashed_false_record_and_modified_derived_fields_are_refused(self):
        native = result()
        for field, value in (('observed_ticks_advanced', 999), ('pause_verified', False), ('phase', 'refused'),
                             ('observed_tick', True), ('current_pause_unproved', 1)):
            bad = copy.deepcopy(native)
            bad['record'][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                o.retain(self.path, bad)
        self.assertFalse(self.side.exists())

    def test_retained_receipt_cannot_move_to_another_intent(self):
        o.retain(self.path, result())
        o.retain(self.path, result())
        for index, field in enumerate(('endpoint', 'df_version', 'idempotency_key')):
            changed = intent()
            if field == 'idempotency_key':
                changed[field] = 'other'
                changed['prepare_token_hex'] = c.token_for('other', bytes.fromhex(changed['plan_digest_hex'])).hex()
            else:
                changed[field] = '127.0.0.1:5001' if field == 'endpoint' else 'other-df'
            path = self.directory / f'other-{index}.json'
            with c.capsule(path, changed):
                pass
            side = Path(str(path) + o.SUFFIX)
            side.write_bytes(self.side.read_bytes()); side.chmod(0o600)
            with self.assertRaises(ValueError):
                o.inspect(path)

    def test_idempotent_terminal_does_not_rewrite_and_conflict_is_preserved(self):
        o.retain(self.path, result())
        original = self.side.read_bytes()
        with patch.object(os, 'write', side_effect=AssertionError('duplicate write')), \
                patch.object(os, 'fsync', side_effect=AssertionError('duplicate sync')):
            self.assertFalse(o.retain(self.path, result())['storage_acknowledged_this_call'])
        for native in (result('running'), result('stopped', 'cancelled'), result('source_lost')):
            with self.assertRaises(ValueError):
                o.retain(self.path, native)
        self.assertEqual(self.side.read_bytes(), original)

    def test_file_and_directory_sync_failure_do_not_acknowledge_or_erase(self):
        actual = os.fsync
        for fail_at in (1, 2):
            path = self.directory / f'sync-{fail_at}.json'
            with c.capsule(path, intent()):
                pass
            calls = []
            def sync(fd):
                calls.append(fd)
                if len(calls) == fail_at:
                    raise OSError('injected storage failure')
                actual(fd)
            with patch.object(os, 'fsync', side_effect=sync), self.assertRaises(OSError):
                o.retain(path, result())
            self.assertEqual(len(calls), fail_at)
            recovered = o.inspect(path)
            self.assertTrue(recovered['terminal_record_retained'])
            self.assertFalse(recovered['storage_acknowledged_this_call'])

    def test_partial_write_is_preserved_and_blocks_future_native_work(self):
        actual = os.write
        calls = []
        def write(fd, data):
            calls.append(fd)
            if len(calls) == 1:
                return actual(fd, data[:31])
            raise OSError('torn write')
        with patch.object(os, 'write', side_effect=write), self.assertRaises(OSError):
            o.retain(self.path, result())
        raw = self.side.read_bytes()
        self.assertEqual(len(raw), 31)
        with patch.dict(os.environ, {'DFMCP_ALLOW_UNADMITTED_RUN_V1_13': '1'}, clear=True), \
                patch.object(c, 'Client', side_effect=AssertionError('corruption contacted native')):
            self.assertEqual(self.cli('query')[0], 2)
        self.assertEqual(self.side.read_bytes(), raw)

    def test_output_is_reserved_before_publication(self):
        with patch.object(o, 'MAX_OUTPUT', 1), self.assertRaises(ValueError):
            o.retain(self.path, result())
        self.assertFalse(self.side.exists())

    def test_private_files_and_symlink_ancestors_fail_closed(self):
        o.retain(self.path, result())
        self.side.chmod(0o400)
        with self.assertRaises(ValueError): o.inspect(self.path)
        self.side.chmod(0o600)
        hard = self.directory / 'hard'; os.link(self.side, hard)
        with self.assertRaises(ValueError): o.inspect(self.path)
        link = self.directory / 'alias'; link.symlink_to(self.directory, target_is_directory=True)
        with self.assertRaises(OSError): o.inspect(link / self.path.name)
        other = self.directory / 'fifo.json'
        with c.capsule(other, intent()): pass
        os.mkfifo(str(other) + o.SUFFIX, 0o600)
        with self.assertRaises(ValueError): o.inspect(other)

    def test_same_length_edit_and_path_replacement_are_detected(self):
        with self.assertRaises(ValueError):
            with o.OutcomeStore(self.path) as store:
                data = self.path.read_bytes()
                self.path.write_bytes(data.replace(b'fake-df', b'fake-xx', 1))
                store.retain(result())
        self.assertFalse(self.side.exists())
        path = self.directory / 'replacement.json'
        with c.capsule(path, intent()): pass
        with self.assertRaises(ValueError):
            with o.OutcomeStore(path):
                path.rename(self.directory / 'old.json')
                with c.capsule(path, intent()): pass

    def test_exclusive_recovery_lock_works_across_processes(self):
        code = ('import sys; from pathlib import Path; import bounded_run_outcomes as o; '
                'o.inspect(Path(sys.argv[1]))')
        env = {**os.environ, 'PYTHONPATH': str(Path(__file__).parent)}
        with o.OutcomeStore(self.path):
            done = subprocess.run([sys.executable, '-c', code, str(self.path)], env=env,
                                  capture_output=True, timeout=5)
            self.assertNotEqual(done.returncode, 0)
        done = subprocess.run([sys.executable, '-c', code, str(self.path)], env=env,
                              capture_output=True, timeout=5)
        self.assertEqual(done.returncode, 0, done.stderr)

    def test_offline_subprocess_needs_no_credentials_and_writes_nothing(self):
        o.retain(self.path, result())
        original = (self.path.stat().st_mtime_ns, self.side.stat().st_mtime_ns)
        done = subprocess.run([sys.executable, str(Path(c.__file__)), 'inspect', '--record', str(self.path)],
                              env={}, capture_output=True, timeout=5)
        self.assertEqual(done.returncode, 0, done.stderr)
        packet = json.loads(done.stdout)
        self.assertEqual(packet['result']['effect_status'], 'historical_pause_verified')
        self.assertNotIn(TOKEN, done.stdout)
        self.assertEqual(original, (self.path.stat().st_mtime_ns, self.side.stat().st_mtime_ns))

    def test_cli_lost_commit_query_restart_and_receipt_retention(self):
        self.path = self.directory / 'live.json'
        self.side = Path(str(self.path) + o.SUFFIX)
        with NativeDouble(connections=2, lose_commit=True) as server:
            env = {'DFMCP_ALLOW_UNADMITTED_RUN_V1_13': '1', 'DFMCP_RUN_TOKEN': TOKEN.decode(),
                   'DFMCP_RUN_ENDPOINT': f'{server.address[0]}:{server.address[1]}'}
            with patch.dict(os.environ, env, clear=True):
                status, _ = self.cli('start', key='test', ticks=10, wall_ms=1000)
                self.assertEqual(status, 2)
                self.assertTrue(self.path.exists()); self.assertFalse(self.side.exists())
                status, packet = self.cli('query')
                self.assertEqual(status, 0)
                self.assertTrue(packet['result']['storage_acknowledged_this_call'])
            self.assertEqual(server.effects, 1)
            self.assertEqual(server.calls.count('CommitRun'), 1)
            self.assertEqual(server.calls.count('QueryRun'), 1)
        with patch.dict(os.environ, {}, clear=True), patch.object(c, 'Client', side_effect=AssertionError('offline connect')):
            self.assertEqual(self.cli('inspect')[1]['result']['effect_status'], 'historical_pause_verified')

    def test_cancel_receipt_is_retained_and_repeat_does_not_dispatch(self):
        path = self.directory / 'cancel.json'; self.path = path
        with NativeDouble() as server:
            self.create(server.address)
            env = {'DFMCP_ALLOW_UNADMITTED_RUN_V1_13': '1', 'DFMCP_RUN_TOKEN': TOKEN.decode(),
                   'DFMCP_RUN_ENDPOINT': f'{server.address[0]}:{server.address[1]}'}
            with patch.dict(os.environ, env, clear=True):
                self.assertEqual(self.cli('cancel')[0], 0)
                with patch.object(c, 'Client', side_effect=AssertionError('repeat cancellation')):
                    status, packet = self.cli('cancel')
                    self.assertEqual(status, 0); self.assertFalse(packet['result']['native_contacted'])
            self.assertEqual(server.calls, ['Handshake', 'CancelRun'])
            self.assertEqual(server.effects, 0)

    def test_orphan_outcome_prevents_new_start_before_connection(self):
        self.path = self.directory / 'unused.json'
        side = Path(str(self.path) + o.SUFFIX); side.write_bytes(b'unknown'); side.chmod(0o600)
        with patch.dict(os.environ, {'DFMCP_ALLOW_UNADMITTED_RUN_V1_13': '1'}, clear=True), \
                patch.object(c, 'Client', side_effect=AssertionError('orphan evidence ignored')):
            self.assertEqual(self.cli('start', key='test', ticks=10, wall_ms=1000)[0], 2)
        self.assertFalse(self.path.exists())

    def test_failed_output_and_wrong_source_never_publish_a_receipt(self):
        for field, value in (('generation', 40), ('generation', True), ('df_version', 'other')):
            native = result(); native['manifest'][field] = value
            with self.assertRaises(ValueError): o.retain(self.path, native)
        self.assertFalse(self.side.exists())


if __name__ == '__main__':
    unittest.main(verbosity=2)
