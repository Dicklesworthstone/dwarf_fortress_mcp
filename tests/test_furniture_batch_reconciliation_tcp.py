"""Real original-batch imports, private custody and TCP; synthetic native source.

Beads: df-dfhack-bridge-plane-c-pic.4/.5. No extracted function bodies or
substituted owner/client/receipt implementations are used in these tests.
"""
from __future__ import annotations

from pathlib import Path
import os
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1] / 'scripts'
sys.path.insert(0, str(SCRIPTS))
import furniture_batch as batch
import build_placement_client as placement
import build_placement_rpc as rpc
import build_placement_store as store
from furniture_plan import FurniturePlan, Step
from furniture_recovery_peer import Peer, placed


class BatchReconciliationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = str(Path(self.directory.name).resolve())
        self.peer = self.enterContext(Peer())
        clean = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
        self.env = self.enterContext(patch.dict(os.environ, {**clean, **self.peer.environment()}, clear=True))
        self.plan = FurniturePlan(tuple(Step(f'item{i:02d}', ('bed', 'chair', 'table')[i % 3],
            i + 1, (20 + 3 * i, 20, 2)) for i in range(3)))
        self.initial = batch.initialize(self.path, self.plan, 'recovery-fixture', 3)
        self.id = self.initial['batch_id']
        self.first = self.plan.ordered[0]

    def advance(self, mode='indeterminate'):
        review = batch.review(self.path, self.id)
        self.peer.commit_mode = mode
        return batch.advance(self.path, self.id, review['expected_plan'], review['confirm_review'])

    def journal(self):
        with batch.Batch(self.path, rpc.Budget(10000)) as owner:
            journal = owner.effects.get(owner.key(self.first))
            return journal.name, journal.raw, journal.state

    def test_query_resolves_original_uncertainty_without_placement_permission(self):
        out = self.advance()
        self.assertEqual(out['status'], 'pending_recovery')
        name, raw, state = self.journal()
        self.assertEqual(state.terminal.phase, 'indeterminate')
        calls = len(self.peer.calls)
        os.environ[rpc.PLACE] = '0'
        out = batch.recover(self.path, self.id, self.first.name)
        self.assertEqual(out['native_query_status'], 'placed')
        self.assertEqual(out['status'], 'ready')
        self.assertEqual(out['next_step'], self.plan.ordered[1].name)
        self.assertEqual(out['plan'], self.plan.json())
        self.assertEqual(self.peer.applied, 1)
        self.assertEqual([c[0] for c in self.peer.calls[calls:]], ['Handshake', 'QueryPlacement'])
        after_name, after_raw, after = self.journal()
        self.assertEqual(after_name, name)
        self.assertTrue(after_raw.startswith(raw))
        self.assertEqual(after.reconciled_from, state.terminal)
        self.assertEqual(after.terminal, placed(state.plan))
        self.assertEqual(after.frames, 5)
        self.assertFalse(after.pending)

    def test_indeterminate_cancellation_stays_offline_and_pending(self):
        self.advance()
        name, raw, _ = self.journal()
        before = list(self.peer.calls)
        with patch.dict(os.environ, {}, clear=True):
            out = batch.recover(self.path, self.id, self.first.name, cancel=True)
        self.assertEqual(out['status'], 'pending_recovery')
        self.assertFalse(out['native_contacted'])
        self.assertEqual(self.peer.calls, before)
        self.assertEqual(Path(self.path, 'effects', name).read_bytes(), raw)

    def test_resolved_queries_and_cancellation_are_offline_after_restart(self):
        self.advance(mode='placed')
        _, raw, _ = self.journal()
        before = list(self.peer.calls)
        with patch.dict(os.environ, {}, clear=True):
            for cancel in (False, True, False):
                out = batch.recover(self.path, self.id, self.first.name, cancel=cancel)
                self.assertFalse(out['native_contacted'])
                self.assertEqual(out['status'], 'ready')
        self.assertEqual(self.peer.calls, before)
        self.assertEqual(self.journal()[1], raw)

    def test_absence_never_erases_retained_uncertainty(self):
        self.advance()
        _, raw, _ = self.journal()
        self.peer.query_mode = 'absent'
        out = batch.recover(self.path, self.id, self.first.name)
        self.assertEqual(out['status'], 'pending_recovery')
        self.assertFalse(out['advance_allowed'])
        self.assertEqual(out['native_query_status'], 'indeterminate')
        self.assertEqual(self.journal()[1], raw)

    def test_repeated_uncertain_reply_is_not_completion(self):
        self.advance()
        _, raw, _ = self.journal()
        self.peer.query_mode = 'unchanged'
        out = batch.recover(self.path, self.id, self.first.name)
        self.assertEqual(out['status'], 'pending_recovery')
        self.assertEqual(self.journal()[1], raw)
        self.assertEqual(self.peer.applied, 1)

    def test_query_recovers_lost_commit_acknowledgment_only_once(self):
        with self.assertRaises(ValueError):
            self.advance(mode='lost')
        _, raw, state = self.journal()
        self.assertIsNone(state.terminal)
        self.assertTrue(state.dispatched)
        os.environ[rpc.PLACE] = '0'
        out = batch.recover(self.path, self.id, self.first.name)
        self.assertEqual(out['status'], 'ready')
        self.assertTrue(self.journal()[1].startswith(raw))
        calls = len(self.peer.calls)
        batch.recover(self.path, self.id, self.first.name)
        self.assertEqual(len(self.peer.calls), calls)
        self.assertEqual(self.peer.applied, 1)

    def test_stopped_batch_can_reconcile_but_cannot_resume(self):
        self.advance()
        batch.stop(self.path, self.id)
        os.environ[rpc.PLACE] = '0'
        out = batch.recover(self.path, self.id, self.first.name)
        self.assertTrue(out['stopped'])
        self.assertEqual(out['status'], 'ready')
        self.assertFalse(out['advance_allowed'])
        with self.assertRaises(ValueError):
            batch.review(self.path, self.id)
        self.assertEqual(self.peer.applied, 1)

    def test_missing_authority_refuses_before_network_and_preserves_journal(self):
        self.advance()
        _, raw, _ = self.journal()
        before = self.peer.connections
        with patch.dict(os.environ, {}, clear=True), self.assertRaises(ValueError):
            batch.recover(self.path, self.id, self.first.name)
        self.assertEqual(self.peer.connections, before)
        self.assertEqual(self.journal()[1], raw)

    def test_changed_incarnation_cannot_resolve_original_uncertain_receipt(self):
        self.advance()
        _, raw, _ = self.journal()
        self.peer.generation += 1
        with self.assertRaises(ValueError):
            batch.recover(self.path, self.id, self.first.name)
        self.assertEqual(self.journal()[1], raw)
        self.assertEqual(self.peer.applied, 1)

    def test_wrong_plan_malformed_and_contradictory_receipts_preserve_pending(self):
        self.advance()
        _, raw, _ = self.journal()
        for mode in ('wrong_plan', 'malformed', 'refused', 'lost'):
            with self.subTest(mode=mode):
                self.peer.query_mode = mode
                with self.assertRaises(ValueError):
                    batch.recover(self.path, self.id, self.first.name)
                self.assertEqual(self.journal()[1], raw)
        self.assertEqual(self.peer.applied, 1)

    def test_sync_loss_preserves_append_and_reopen_does_not_query_again(self):
        self.advance()
        _, original, _ = self.journal()
        real_sync = os.fsync
        def fail_after_sync(fd):
            real_sync(fd)
            raise OSError('synthetic lost synchronization acknowledgment')
        with patch.object(store.os, 'fsync', fail_after_sync), self.assertRaises(OSError):
            batch.recover(self.path, self.id, self.first.name)
        _, raw, state = self.journal()
        self.assertTrue(raw.startswith(original))
        self.assertEqual(state.terminal.phase, 'placed')
        calls = len(self.peer.calls)
        with patch.dict(os.environ, {}, clear=True):
            out = batch.recover(self.path, self.id, self.first.name)
        self.assertEqual(len(self.peer.calls), calls)
        self.assertEqual(out['status'], 'ready')
        self.assertEqual(self.peer.applied, 1)

    def test_cli_query_uses_real_imports_files_and_wire(self):
        self.advance()
        env = {**os.environ, rpc.PLACE: '0', 'PYTHONDONTWRITEBYTECODE': '1'}
        process = subprocess.run([sys.executable, '-W', 'error', str(SCRIPTS / 'furniture_batch.py'),
            'query', '--directory', self.path, '--batch-id', self.id, '--step', self.first.name],
            env=env, capture_output=True, timeout=15)
        self.assertEqual(process.returncode, 0, process.stderr or process.stdout)
        import json
        reply = json.loads(process.stdout)
        self.assertEqual(reply['result']['native_query_status'], 'placed')
        self.assertEqual(reply['result']['plan'], self.plan.json())
        self.assertNotIn(self.peer.environment()[rpc.TOKEN].encode(), process.stdout)
        self.assertEqual(self.peer.applied, 1)


    def test_revocation_during_query_cannot_publish_recovery(self):
        self.advance()
        _, raw, _ = self.journal()
        original_token = os.environ[rpc.TOKEN]
        def revoke(name, fields):
            if name == 'QueryPlacement':
                os.environ[rpc.TOKEN] = 'replacement-token-cannot-inherit-authority'
        self.peer.hook = revoke
        try:
            with self.assertRaises(ValueError):
                batch.recover(self.path, self.id, self.first.name)
        finally:
            self.peer.hook = None
            os.environ[rpc.TOKEN] = original_token
        self.assertEqual(self.journal()[1], raw)
        self.assertEqual(self.peer.applied, 1)

    def test_revocation_during_result_rendering_retains_proof_but_refuses_output(self):
        self.advance()
        original_encoded = batch.encoded
        original_token = os.environ[rpc.TOKEN]
        def revoke(operation, out, ok=True):
            raw = original_encoded(operation, out, ok)
            if operation == 'query' and out.get('native_contacted'):
                os.environ[rpc.TOKEN] = 'replacement-token-cannot-inherit-authority'
            return raw
        try:
            with patch.object(batch, 'encoded', revoke), self.assertRaises(ValueError):
                batch.recover(self.path, self.id, self.first.name)
        finally:
            os.environ[rpc.TOKEN] = original_token
        self.assertEqual(self.journal()[2].terminal.phase, 'placed')
        before = list(self.peer.calls)
        with patch.dict(os.environ, {}, clear=True):
            self.assertEqual(batch.recover(self.path, self.id, self.first.name)['status'], 'ready')
        self.assertEqual(self.peer.calls, before)
        self.assertEqual(self.peer.applied, 1)

    def test_query_cannot_renew_exhausted_native_call_allowance(self):
        self.advance()
        _, raw, _ = self.journal()
        budget = rpc.Budget(10000, max_calls=7)
        calls = len(self.peer.calls)
        with self.assertRaisesRegex(ValueError, 'allowance exhausted'):
            batch.recover(self.path, self.id, self.first.name, _budget=budget)
        self.assertEqual(budget.calls_left, 0)
        self.assertEqual([name for name, _ in self.peer.calls[calls:]], ['Handshake'])
        self.assertEqual(self.journal()[1], raw)

    def test_exact_item_module_import_does_not_require_room_planner(self):
        code = ("import furniture_batch, sys; "
                "assert not any(name.startswith(('room_', 'terrain_')) for name in sys.modules); "
                "assert 'furniture_handoff' not in sys.modules; "
                "assert callable(furniture_batch.recover)")
        result = subprocess.run([sys.executable, '-W', 'error', '-c', code],
            env={**os.environ, 'PYTHONPATH': str(SCRIPTS)}, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == '__main__':
    unittest.main()
