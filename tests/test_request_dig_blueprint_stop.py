"""Real private-file/process tests for identity-only original-batch revocation.

These do not qualify DFHack, room geometry, or the native executor. Synthetic
manifest bodies exercise the existing identity and stop encoding independently.
"""
from __future__ import annotations

import fcntl
import hashlib
import json
import multiprocessing
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import request_dig_blueprint_stop as stop


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True,
                      allow_nan=False).encode('ascii')


def envelope(value):
    return canonical({'value': value, 'sha256': hashlib.sha256(canonical(value)).hexdigest()}) + b'\n'


def hold_owner(root, ready, release):
    directory = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
    manifest = os.open(root / 'blueprint.json', os.O_RDONLY)
    try:
        fcntl.flock(directory, fcntl.LOCK_EX | fcntl.LOCK_NB)
        fcntl.flock(manifest, fcntl.LOCK_EX | fcntl.LOCK_NB)
        ready.set()
        if not release.wait(10):
            raise RuntimeError('test owner was not released')
    finally:
        os.close(manifest)
        os.close(directory)


class StopTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve() / 'batch'
        self.root.mkdir(mode=0o700)
        (self.root / 'effects').mkdir(mode=0o700)
        effect = (self.root / 'effects').stat()
        self.value = {'format': stop.LEGACY_FORMAT, 'effects_identity': [effect.st_dev, effect.st_ino],
                      'blueprint': {'schema': 'dfmcp.excavation-blueprint/1', 'parts': []},
                      'opaque_original_intent': 'not a terrain or native admission fixture'}
        self.write_manifest()

    def write(self, name, raw):
        path = self.root / name
        path.write_bytes(raw)
        path.chmod(0o600)
        return path

    def write_manifest(self):
        raw = envelope(self.value)
        self.write('blueprint.json', raw)
        domain = b'dfmcp-dig-blueprint-batch/2' if self.value['format'] == stop.ROOM_FORMAT else b'dfmcp-dig-blueprint-batch/1'
        self.batch_id = hashlib.sha256(domain + b'\0' + canonical(self.value)).hexdigest()

    def request(self):
        return stop.request_stop(self.root, self.batch_id, stop.Budget())

    def refused(self):
        with self.assertRaises((ValueError, OSError)):
            self.request()

    def test_exact_existing_marker_and_no_native_authority(self):
        with patch('socket.socket', side_effect=AssertionError('unexpected native contact')):
            result = self.request()
        expected = envelope({'format': 'dfmcp.dig-blueprint-stop/1', 'batch_id': self.batch_id})
        self.assertEqual((self.root / 'stopped.json').read_bytes(), expected)
        self.assertEqual(result['stop_record_sha256'], hashlib.sha256(expected).hexdigest())
        self.assertTrue(result['future_step_admission_revoked'])
        self.assertTrue(result['in_flight_effect_may_complete'])
        self.assertTrue(result['cooperative_only'])
        for field in ('native_quiescence_proven', 'game_paused_proven', 'native_effects_cancelled',
                      'retry_commit_permitted', 'excavation_completion_proven', 'production_admitted'):
            self.assertIs(result[field], False)
        self.assertEqual(result['native_calls'], 0)

    def test_repeat_preserves_inode_bytes_and_result_and_resyncs(self):
        first = self.request()
        path = self.root / 'stopped.json'
        before = path.stat()
        original_sync = os.fsync
        with patch.object(stop.os, 'fsync', wraps=original_sync) as sync:
            self.assertEqual(first, self.request())
        self.assertEqual(sync.call_count, 2)
        self.assertEqual((path.stat().st_ino, path.stat().st_mtime_ns), (before.st_ino, before.st_mtime_ns))

    def test_request_succeeds_with_live_process_holding_both_owner_locks(self):
        ctx = multiprocessing.get_context('spawn')
        ready, release = ctx.Event(), ctx.Event()
        child = ctx.Process(target=hold_owner, args=(self.root, ready, release))
        child.start()
        try:
            self.assertTrue(ready.wait(5))
            result = subprocess.run([sys.executable, stop.__file__, '--directory', str(self.root),
                                     '--batch-id', self.batch_id], capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(json.loads(result.stdout)['future_step_admission_revoked'])
            # The request did not steal or release the running owner's locks.
            for name, flags in ((self.root, os.O_RDONLY | os.O_DIRECTORY),
                                (self.root / 'blueprint.json', os.O_RDONLY)):
                fd = os.open(name, flags)
                try:
                    with self.assertRaises(BlockingIOError):
                        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                finally:
                    os.close(fd)
        finally:
            release.set()
            child.join(5)
            if child.is_alive():
                child.kill()
                child.join()
        self.assertEqual(child.exitcode, 0)

    def test_wrong_batch_is_refused_before_creating_marker(self):
        with self.assertRaises(ValueError):
            stop.request_stop(self.root, '0' * 64, stop.Budget())
        self.assertFalse((self.root / 'stopped.json').exists())

    def test_room_identity_uses_original_v2_domain(self):
        self.value['format'] = stop.ROOM_FORMAT
        self.value['room_handoff'] = {'retained_bytes': 'synthetic identity fixture'}
        self.write_manifest()
        self.assertEqual(self.request()['batch_id'], self.batch_id)

    def test_room_bound_does_not_widen_legacy_bound(self):
        self.value['padding'] = 'x' * 66000
        self.write_manifest()
        self.refused()
        self.assertFalse((self.root / 'stopped.json').exists())
        self.value['format'] = stop.ROOM_FORMAT
        self.write_manifest()
        self.assertTrue(self.request()['ok'])

    def test_unsupported_format_is_refused(self):
        self.value['format'] = 'dfmcp.dig-blueprint-batch/3'
        self.write_manifest()
        self.refused()

    def test_checksum_noncanonical_duplicate_and_deep_json_refused(self):
        valid = envelope(self.value)
        inputs = [valid.replace(b'"sha256":"', b'"sha256":"0', 1), b' ' + valid,
                  b'{"value":{},"value":{},"sha256":"x"}\n', b'[' * 15 + b'0' + b']' * 15]
        for raw in inputs:
            with self.subTest(raw=raw[:30]):
                self.write('blueprint.json', raw)
                self.refused()
                self.assertFalse((self.root / 'stopped.json').exists())

    def test_sparse_oversized_manifest_refused(self):
        with (self.root / 'blueprint.json').open('r+b') as file:
            file.truncate(stop.ROOM_MAX_MANIFEST + 1)
        self.refused()
        self.assertFalse((self.root / 'stopped.json').exists())

    def test_effect_identity_rejects_booleans(self):
        self.value['effects_identity'] = [True, False]
        self.write_manifest()
        self.refused()

    def test_effect_directory_replacement_is_refused(self):
        (self.root / 'effects').rename(self.root.parent / 'old-effects')
        (self.root / 'effects').mkdir(mode=0o700)
        self.refused()
        self.assertFalse((self.root / 'stopped.json').exists())

    def test_nonprivate_directory_manifest_and_effects_refused(self):
        for path, wrong, right in ((self.root, 0o750, 0o700),
                                   (self.root / 'blueprint.json', 0o400, 0o600),
                                   (self.root / 'effects', 0o755, 0o700)):
            with self.subTest(path=path):
                path.chmod(wrong)
                try:
                    self.refused()
                finally:
                    path.chmod(right)
        self.assertFalse((self.root / 'stopped.json').exists())

    def test_symlink_directory_and_manifest_refused(self):
        alias = self.root.parent / 'alias'
        alias.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(OSError):
            stop.request_stop(alias, self.batch_id, stop.Budget())
        original = self.root / 'blueprint.json'
        target = self.root.parent / 'manifest'
        original.rename(target)
        original.symlink_to(target)
        self.refused()
        self.assertFalse((self.root / 'stopped.json').exists())

    def test_hardlinked_manifest_refused(self):
        os.link(self.root / 'blueprint.json', self.root.parent / 'alias')
        self.refused()
        self.assertFalse((self.root / 'stopped.json').exists())

    def test_unexpected_root_entry_refused(self):
        self.write('not-an-original-artifact', b'x')
        self.refused()

    def test_partial_empty_conflicting_oversized_markers_are_never_repaired(self):
        for raw in (b'', b'{', envelope({'format': stop.STOP_FORMAT, 'batch_id': '0' * 64}), b'x' * 1025):
            with self.subTest(size=len(raw)):
                path = self.write('stopped.json', raw)
                inode = path.stat().st_ino
                self.refused()
                self.assertEqual(path.read_bytes(), raw)
                self.assertEqual(path.stat().st_ino, inode)

    def test_symlink_and_hardlinked_stop_markers_refused_without_touching_target(self):
        target = self.root.parent / 'other'
        target.write_bytes(b'never overwrite')
        target.chmod(0o600)
        path = self.root / 'stopped.json'
        for link in ('symbolic', 'hard'):
            if link == 'symbolic':
                path.symlink_to(target)
            else:
                os.link(target, path)
            self.refused()
            self.assertEqual(target.read_bytes(), b'never overwrite')
            path.unlink()

    def test_fifo_stop_marker_is_refused_without_blocking(self):
        os.mkfifo(self.root / 'stopped.json', 0o600)
        self.refused()

    def test_competing_publisher_lock_refuses_without_waiting(self):
        self.request()
        fd = os.open(self.root / 'stopped.json', os.O_RDONLY)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(BlockingIOError):
                self.request()
        finally:
            os.close(fd)
        self.assertTrue(self.request()['ok'])

    def test_partial_write_failure_preserves_the_exact_torn_marker(self):
        real_write = os.write
        def fail(fd, data):
            real_write(fd, data[:7])
            raise OSError('injected write loss')
        with patch.object(stop.os, 'write', side_effect=fail):
            self.refused()
        path = self.root / 'stopped.json'
        raw = path.read_bytes()
        self.assertEqual(len(raw), 7)
        self.refused()
        self.assertEqual(path.read_bytes(), raw)

    def test_lost_file_and_directory_sync_ack_can_be_reverified_without_rewrite(self):
        for fail_at in (1, 2):
            with self.subTest(fail_at=fail_at):
                calls = 0
                real_sync = os.fsync
                def fail(fd):
                    nonlocal calls
                    calls += 1
                    if calls == fail_at:
                        raise OSError('injected sync loss')
                    real_sync(fd)
                with patch.object(stop.os, 'fsync', side_effect=fail):
                    self.refused()
                path = self.root / 'stopped.json'
                raw, inode = path.read_bytes(), path.stat().st_ino
                self.assertTrue(self.request()['ok'])
                self.assertEqual((path.read_bytes(), path.stat().st_ino), (raw, inode))

    def test_directory_replacement_during_write_cannot_acknowledge_new_directory(self):
        real_write = os.write
        moved = self.root.parent / 'moved'
        def replace(fd, data):
            self.root.rename(moved)
            self.root.mkdir(mode=0o700)
            return real_write(fd, data)
        with patch.object(stop.os, 'write', side_effect=replace):
            self.refused()
        self.assertFalse((self.root / 'stopped.json').exists())
        self.assertTrue((moved / 'stopped.json').exists())

    def test_manifest_change_after_stop_create_fences_without_repair(self):
        real_open = os.open
        def replace(path, flags, *args, **kwargs):
            fd = real_open(path, flags, *args, **kwargs)
            if path == 'stopped.json' and flags & os.O_CREAT:
                self.write('blueprint.json', b'changed original manifest')
            return fd
        with patch.object(stop.os, 'open', side_effect=replace):
            self.refused()
        self.assertEqual((self.root / 'stopped.json').read_bytes(), b'')

    def test_expired_budget_has_no_local_or_native_effect(self):
        budget = stop.Budget()
        budget.deadline = 0
        with self.assertRaises(ValueError):
            stop.request_stop(self.root, self.batch_id, budget)
        self.assertFalse((self.root / 'stopped.json').exists())

    def test_budget_loss_after_write_keeps_marker_without_success_ack(self):
        budget = stop.Budget()
        real_write = os.write
        def expire(fd, data):
            count = real_write(fd, data)
            budget.deadline = 0
            return count
        with patch.object(stop.os, 'write', side_effect=expire), self.assertRaises(ValueError):
            stop.request_stop(self.root, self.batch_id, budget)
        self.assertTrue(self.request()['ok'])

    def test_cli_offline_despite_unrelated_profiles_and_no_credentials(self):
        env = dict(os.environ, DFMCP_UNRELATED_SECRET='must-not-leak')
        result = subprocess.run([sys.executable, stop.__file__, '--directory', str(self.root),
                                 '--batch-id', self.batch_id], env=env, capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.splitlines()), 1)
        self.assertNotIn(b'must-not-leak', result.stdout + result.stderr)
        self.assertEqual(json.loads(result.stdout)['native_calls'], 0)

    def test_cli_invalid_arguments_returns_one_machine_error(self):
        result = subprocess.run([sys.executable, stop.__file__, '--unknown'], capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 2)
        self.assertEqual(len(result.stdout.splitlines()), 1)
        self.assertEqual(json.loads(result.stdout)['error'], 'STOP_REQUEST_REFUSED')
        self.assertEqual(result.stderr, b'')

    def test_short_output_never_repeats_write_or_undoes_stop(self):
        class Short:
            writes = 0
            def write(self, raw):
                self.writes += 1
                return len(raw) - 1
            def flush(self):
                raise AssertionError('short write must terminate')
        stream = Short()
        with patch.object(stop.sys, 'stdout', type('Output', (), {'buffer': stream})()):
            code = stop.main(['--directory', str(self.root), '--batch-id', self.batch_id])
        self.assertEqual(code, 2)
        self.assertEqual(stream.writes, 1)
        self.assertTrue(self.request()['ok'])

    def test_sigkill_during_publication_retains_torn_stop(self):
        code = '''import os, signal, sys
sys.path.insert(0, sys.argv[1])
import request_dig_blueprint_stop as s
real = os.write
def die(fd, data):
    real(fd, data[:9])
    os.kill(os.getpid(), signal.SIGKILL)
s.os.write = die
s.main(['--directory', sys.argv[2], '--batch-id', sys.argv[3]])
'''
        result = subprocess.run([sys.executable, '-c', code, str(Path(stop.__file__).parent),
                                 str(self.root), self.batch_id], capture_output=True, timeout=5)
        self.assertEqual(result.returncode, -9)
        raw = (self.root / 'stopped.json').read_bytes()
        self.assertEqual(len(raw), 9)
        self.refused()
        self.assertEqual((self.root / 'stopped.json').read_bytes(), raw)


if __name__ == '__main__':
    unittest.main()
