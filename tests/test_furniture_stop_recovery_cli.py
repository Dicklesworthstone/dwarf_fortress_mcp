"""Real CLI recovery after a stopped writer/placement crash, without re-dispatch."""
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import unittest
from unittest.mock import patch

from test_furniture_stop_recovery import StopFixture
import furniture_batch as batch
import build_placement_rpc as rpc
import build_placement_store as storage

ROOT = Path(__file__).resolve().parents[1]


class StopRecoveryCliTests(StopFixture, unittest.TestCase):
    def cli(self, operation, name=None, env=None):
        args = [sys.executable, '-W', 'error', str(ROOT / 'scripts/furniture_batch.py'), operation,
                '--directory', str(self.root), '--batch-id', self.batch_id]
        if name is not None: args += ['--step', name]
        environment = {**os.environ, 'PYTHONDONTWRITEBYTECODE': '1', rpc.PLACE: '0'} if env is None else env
        return subprocess.run(args, capture_output=True, env=environment, timeout=15)

    def local(self, operation='inspect', name=None):
        args = [operation, '--directory', str(self.root), '--batch-id', self.batch_id]
        if name is not None: args += ['--step', name]
        outputs = []
        with patch.object(batch, 'write_output', side_effect=lambda raw: outputs.append(raw) or 0):
            code = batch.main(args)
        self.assertEqual(len(outputs), 1)
        return code, json.loads(outputs[0])

    def test_sigkill_stop_writer_during_lost_commit_recovers_the_same_key_at_cli(self):
        self.initialize()
        results = []
        # Execute the actual signal writer, not a replacement marker producer.
        # Kill only this owned test subprocess after its first real marker byte.
        code = '''import os, signal, sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
import request_furniture_batch_stop as stop
original = os.write
def interrupted(fd, data):
    if os.readlink('/proc/self/fd/' + str(fd)).endswith('/stop.json'):
        original(fd, data[:1]); os.fsync(fd)
        os.kill(os.getpid(), signal.SIGKILL)
    return original(fd, data)
os.write = interrupted
raise SystemExit(stop.main(['--directory', sys.argv[2], '--batch-id', sys.argv[3]]))
'''
        def crash(name, fields):
            if name == 'CommitPlacement':
                result = subprocess.run([sys.executable, '-W', 'error', '-c', code,
                    str(ROOT / 'scripts'), str(self.root), self.batch_id],
                    capture_output=True, timeout=10)
                results.append(result)
        self.peer.hook = crash
        self.peer.commit_mode = 'lost'
        proposed = batch.review(str(self.root), self.batch_id)
        with self.assertRaises(ValueError):
            batch.advance(str(self.root), self.batch_id, proposed['expected_plan'], proposed['confirm_review'])
        self.peer.hook = None
        self.assertEqual(len(results), 1)
        self.assertEqual(results[0].returncode, -signal.SIGKILL)
        self.assertEqual(results[0].stdout, b'')
        self.assertEqual(results[0].stderr, b'')
        self.assertEqual((self.root / 'stop.json').read_bytes(), b'{')
        self.assertEqual(self.peer.applied, 1)
        frozen = {p.name: p.read_bytes() for p in (self.root / 'batch.json', self.root / 'steps.jsonl')}
        before = len(self.peer.calls)
        result = self.cli('query', 's00')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, b'')
        out = json.loads(result.stdout)['result']
        self.assert_stopped(out, b'{')
        self.assertEqual(out['placed'], 1)
        self.assertEqual([n for n, _ in self.peer.calls[before:]], ['Handshake', 'QueryPlacement'])
        self.assertEqual(frozen, {name: (self.root / name).read_bytes() for name in frozen})
        self.assertEqual(self.peer.applied, 1)
        offline = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
        before = len(self.peer.calls)
        for operation in ('inspect', 'query', 'cancel'):
            again = self.cli(operation, 's00', offline)
            self.assertEqual(again.returncode, 0, again.stderr)
            self.assertFalse(json.loads(again.stdout)['result']['native_contacted'])
        self.assertEqual(len(self.peer.calls), before)
        self.assertEqual(self.cli('review').returncode, 2)

    def test_cli_cancels_only_original_preparation_while_stop_stays_damaged(self):
        self.initialize(); self.attempt(raw=b'{', boundary='PreparePlacement')
        process = self.cli('cancel', 's00')
        self.assertEqual(process.returncode, 0, process.stderr)
        out = json.loads(process.stdout)['result']
        self.assert_stopped(out, b'{')
        self.assertEqual(out['steps'][0]['phase'], 'cancelled')
        self.assertEqual(self.peer.applied, 0)
        self.assertNotIn('CommitPlacement', [name for name, _ in self.peer.calls])

    def test_orphan_intent_registration_recovery_does_not_prepare_or_commit(self):
        self.initialize()
        proposed = batch.review(str(self.root), self.batch_id)
        with patch.object(batch.Batch, 'register', side_effect=InterruptedError('crash before index registration')):
            with self.assertRaises(InterruptedError):
                batch.advance(str(self.root), self.batch_id, proposed['expected_plan'], proposed['confirm_review'])
        self.assertEqual((self.root / 'steps.jsonl').read_bytes(), batch.HEADER)
        self.marker(b'')
        child = next((self.root / 'effects').iterdir()); original = child.read_bytes()
        process = self.cli('query', 's00')
        self.assertEqual(process.returncode, 0, process.stderr)
        out = json.loads(process.stdout)['result']
        self.assert_stopped(out, b'')
        self.assertTrue(out['steps'][0]['registered'])
        self.assertEqual(out['pending_step'], 's00')
        self.assertEqual(child.read_bytes(), original)
        self.assertEqual(len((self.root / 'steps.jsonl').read_bytes().splitlines()), 2)
        self.assertEqual(self.peer.applied, 0)
        self.assertNotIn('PreparePlacement', [name for name, _ in self.peer.calls])

    def test_original_custody_is_held_through_the_single_output_write(self):
        self.initialize(); self.marker(b'{')
        outputs, blocked = [], []
        def write(raw):
            outputs.append(raw)
            competitor = self.cli('inspect')
            blocked.append(competitor.returncode)
            return 0
        with patch.object(batch, 'write_output', side_effect=write):
            code = batch.main(['inspect', '--directory', str(self.root), '--batch-id', self.batch_id])
        self.assertEqual(code, 0)
        self.assertEqual(blocked, [2])
        self.assertEqual(len(outputs), 1)
        self.assertTrue(json.loads(outputs[0])['result']['inventory_verified'])

    def test_change_during_final_serialization_prevents_a_stale_success(self):
        self.initialize(); self.marker(b'{')
        original = batch.encoded; calls = 0
        def changed(operation, out, *args):
            nonlocal calls
            raw = original(operation, out, *args)
            if 'stop_evidence' in out:
                calls += 1
                if calls == 2: self.marker(b'different')
            return raw
        with patch.object(batch, 'encoded', side_effect=changed): code, value = self.local()
        self.assertEqual(code, 2)
        self.assertFalse(value['ok'])
        self.assertFalse(value['result']['inventory_verified'])
        self.assertEqual(calls, 2)

    def test_change_after_history_inspection_is_detected_on_final_reopen(self):
        self.initialize(); self.marker(b'{')
        original = batch.inspect
        def changed(*args, **kwargs):
            out = original(*args, **kwargs)
            self.marker(b'new-marker')
            return out
        with patch.object(batch, 'inspect', side_effect=changed): code, value = self.local()
        self.assertEqual(code, 2)
        self.assertFalse(value['ok'])
        self.assertNotIn('stop_evidence', value['result'])

    def test_changed_marker_during_output_returns_failure_without_second_object(self):
        self.initialize(); self.marker(b'{')
        outputs = []
        def write(raw):
            outputs.append(raw)
            self.marker(b'changed-during-write')
            return 0
        with patch.object(batch, 'write_output', side_effect=write):
            code = batch.main(['inspect', '--directory', str(self.root), '--batch-id', self.batch_id])
        self.assertEqual(code, 2)
        self.assertEqual(len(outputs), 1)

    def test_short_output_does_not_repeat_recovery_or_emit_another_object(self):
        self.initialize(); self.attempt(raw=b'{')
        os.environ[rpc.PLACE] = '0'
        before = len(self.peer.calls); outputs = []
        with patch.object(batch, 'write_output', side_effect=lambda raw: outputs.append(raw) or 2):
            code = batch.main(['query', '--directory', str(self.root), '--batch-id', self.batch_id, '--step', 's00'])
        self.assertEqual(code, 2)
        self.assertEqual(len(outputs), 1)
        self.assertEqual([n for n, _ in self.peer.calls[before:]], ['Handshake', 'QueryPlacement'])
        self.assertEqual(self.peer.applied, 1)
        self.assertEqual(self.inspect()['placed'], 1)

    def test_revoked_query_authority_after_receipt_blocks_publication_not_history(self):
        self.initialize(); self.attempt(raw=b'{')
        os.environ[rpc.PLACE] = '0'
        original = batch.encoded; calls = 0
        def revoked(operation, out, *args):
            nonlocal calls
            raw = original(operation, out, *args)
            if 'stop_evidence' in out:
                calls += 1
                if calls == 2: os.environ[rpc.TOKEN] = 'revoked-current-token-with-enough-length'
            return raw
        with patch.object(batch, 'encoded', side_effect=revoked): code, value = self.local('query', 's00')
        self.assertEqual(code, 2)
        self.assertFalse(value['ok'])
        self.assertEqual(self.peer.applied, 1)
        self.assertEqual(self.inspect()['placed'], 1)

    def test_final_publication_does_not_renew_an_expired_operation_budget(self):
        self.initialize(); self.marker(b'{')
        original = batch._publish_stop_recovery
        def expired(operation, path, batch_id, out, budget):
            budget.deadline = 0
            return original(operation, path, batch_id, out, budget)
        with patch.object(batch, '_publish_stop_recovery', side_effect=expired): code, value = self.local()
        self.assertEqual(code, 2)
        self.assertFalse(value['ok'])

    def test_marker_replacement_after_reopen_cannot_inherit_its_pin(self):
        self.initialize(); self.marker(b'{')
        original = batch.encoded; calls = 0
        def replace(operation, out, *args):
            nonlocal calls
            raw = original(operation, out, *args)
            if 'stop_evidence' in out:
                calls += 1
                if calls == 2:
                    alternate = self.root.parent / 'replacement'
                    alternate.write_bytes(b'{'); alternate.chmod(0o600)
                    os.replace(alternate, self.root / 'stop.json')
            return raw
        with patch.object(batch, 'encoded', side_effect=replace): code, value = self.local()
        self.assertEqual(code, 2)
        self.assertFalse(value['ok'])

    def test_explicit_signal_retry_does_not_repair_the_recovered_marker(self):
        self.initialize(); self.attempt(raw=b'{')
        query = self.cli('query', 's00')
        self.assertEqual(query.returncode, 0, query.stderr)
        process = subprocess.run([sys.executable, str(ROOT / 'scripts/request_furniture_batch_stop.py'),
            '--directory', str(self.root), '--batch-id', self.batch_id], capture_output=True, timeout=10)
        self.assertEqual(process.returncode, 2)
        self.assertFalse(json.loads(process.stdout)['ok'])
        self.assertEqual((self.root / 'stop.json').read_bytes(), b'{')
        self.assertEqual(self.inspect()['placed'], 1)
        self.assertEqual(self.peer.applied, 1)


if __name__ == '__main__':
    unittest.main()
