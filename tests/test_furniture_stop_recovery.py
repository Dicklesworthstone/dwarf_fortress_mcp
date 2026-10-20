"""Interrupted stop publication cannot strand exact original placement evidence.

Full production Python modules, real private journals and joined synthetic TCP
sources. No native DFHack or live-game qualification is implied.
"""
from contextlib import ExitStack
import hashlib
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import furniture_batch as batch
import build_placement_rpc as rpc
import build_placement_store as storage
from furniture_plan import FurniturePlan, Step, canonical
import request_furniture_batch_stop as signal
from request_dig_blueprint_stop import Budget as StopBudget
from furniture_recovery_peer import Peer


class StopFixture:
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        directory = self.stack.enter_context(tempfile.TemporaryDirectory())
        self.root = Path(directory) / 'batch'
        self.root.mkdir(mode=0o700)
        self.peer = Peer()
        environment = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
        self.stack.enter_context(patch.dict(os.environ, {**environment, **self.peer.environment()}, clear=True))
        self.stack.enter_context(self.peer)  # Join before restoring credentials.

    def initialize(self, count=2):
        self.plan = FurniturePlan(tuple(Step(f's{i:02}', 'chair', i + 1,
            (10 + i % 8 * 3, 10 + i // 8 * 3, 2), (f's{i-1:02}',) if i else ()) for i in range(count)))
        self.initial = batch.initialize(str(self.root), self.plan, 'recovery-fixture', 3)
        self.batch_id = self.initial['batch_id']
        return self.initial

    def marker(self, raw):
        path = self.root / 'stop.json'
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        try:
            if raw:
                storage.write_all(fd, raw)
            os.fsync(fd)
        finally:
            os.close(fd)
        return path

    def attempt(self, mode='lost', raw=None, boundary='CommitPlacement'):
        proposed = batch.review(str(self.root), self.batch_id)
        self.peer.commit_mode = mode
        if raw is not None:
            self.peer.hook = lambda name, fields: self.marker(raw) if name == boundary else None
        try:
            if mode == 'lost' or raw is not None:
                with self.assertRaises(ValueError):
                    batch.advance(str(self.root), self.batch_id, proposed['expected_plan'], proposed['confirm_review'])
            else:
                return batch.advance(str(self.root), self.batch_id, proposed['expected_plan'], proposed['confirm_review'])
        finally:
            self.peer.hook = None

    def inspect(self, name=None):
        return batch.inspect(str(self.root), self.batch_id, name)

    def query(self, name='s00', cancel=False, budget=None):
        os.environ[rpc.PLACE] = '0'
        return batch.recover(str(self.root), self.batch_id, name, cancel, _budget=budget)

    def assert_stopped(self, state, raw):
        self.assertTrue(state['stopped'])
        self.assertFalse(state['advance_allowed'])
        self.assertTrue(state['inventory_verified'])
        self.assertEqual(state['plan'], self.plan.json())
        self.assertEqual(state['stop_evidence'], {
            'status': 'unverified_marker', 'marker_bytes': len(raw),
            'marker_sha256': hashlib.sha256(raw).hexdigest(), 'marker_validated': False,
            'new_work_fenced': True, 'recovery_only': True, 'marker_modified': False,
            'native_quiescence_proven': False})
        self.assertEqual((self.root / 'stop.json').read_bytes(), raw)
        self.assertFalse(state['construction_completion_proven'])



class StopRecoveryTests(StopFixture, unittest.TestCase):
    def test_lost_commit_and_empty_stop_recover_without_another_placement(self):
        self.initialize()
        self.attempt(raw=b'')
        journal = next((self.root / 'effects').iterdir())
        original = journal.read_bytes()
        identity = (self.root / 'stop.json').stat().st_ino
        self.assertEqual(self.peer.applied, 1)
        self.assert_stopped(self.inspect(), b'')
        start = len(self.peer.calls)
        result = self.query()
        self.assert_stopped(result, b'')
        self.assertEqual(result['placed'], 1)
        self.assertEqual(result['next_step'], 's01')
        self.assertEqual([n for n, _ in self.peer.calls[start:]], ['Handshake', 'QueryPlacement'])
        self.assertTrue(journal.read_bytes().startswith(original))
        self.assertEqual(storage.replay(journal.read_bytes()).terminal.phase, 'placed')
        self.assertEqual((self.root / 'stop.json').stat().st_ino, identity)
        self.assertEqual(self.peer.applied, 1)
        with self.assertRaises(ValueError):
            batch.review(str(self.root), self.batch_id)

    def test_all_truncated_marker_prefixes_are_recovery_only_and_byte_pinned(self):
        self.initialize()
        full = batch.seal({'schema': 'dfmcp.furniture-batch-stop/1', 'batch_id': self.batch_id})
        start = len(self.peer.calls)
        digests = set()
        for size in range(len(full)):
            raw = full[:size]
            self.marker(raw)
            with self.subTest(size=size):
                state = self.inspect()
                self.assert_stopped(state, raw)
                digests.add(state['inventory_digest'])
        self.assertEqual(len(digests), len(full))
        self.assertEqual(len(self.peer.calls), start)
        self.marker(full)
        state = self.inspect()
        self.assertTrue(state['stopped'])
        self.assertNotIn('stop_evidence', state)

    def test_arbitrary_invalid_bytes_are_never_parsed_into_positive_stop_evidence(self):
        self.initialize()
        cases = [b'\xff\x00', b'[' * batch.MAX_FILE, b'null', b'{"value":true}',
                 batch.seal({'schema': 'dfmcp.furniture-batch-stop/1', 'batch_id': '0' * 64}),
                 batch.seal({'schema': 'other', 'batch_id': self.batch_id})]
        for raw in cases:
            with self.subTest(raw=raw[:24]):
                self.marker(raw)
                self.assert_stopped(self.inspect(), raw)
                with self.assertRaises(ValueError):
                    batch.stop(str(self.root), self.batch_id)
        self.marker(b'x' * (batch.MAX_FILE + 1))
        with self.assertRaises(ValueError):
            self.inspect()

    def test_stop_during_prepare_can_cancel_original_preparation_with_control_disabled(self):
        self.initialize()
        self.attempt(raw=b'{', boundary='PreparePlacement')
        self.assertEqual(self.peer.applied, 0)
        original_key = next(iter(self.peer.records))
        self.assertEqual(self.peer.records[original_key].phase, 'prepared')
        start = len(self.peer.calls)
        out = self.query(cancel=True)
        self.assert_stopped(out, b'{')
        self.assertEqual(out['steps'][0]['phase'], 'cancelled')
        self.assertEqual(self.peer.calls[start:], [('Handshake', ''), ('CancelPlacement', original_key)])
        self.assertEqual(self.peer.applied, 0)

    def test_retained_indeterminate_can_be_reconciled_but_not_cancelled_away(self):
        self.initialize()
        self.attempt(mode='indeterminate')
        self.marker(b'{')
        journal = next((self.root / 'effects').iterdir())
        uncertain = journal.read_bytes()
        calls = len(self.peer.calls)
        out = self.query(cancel=True)
        self.assert_stopped(out, b'{')
        self.assertEqual(out['pending_step'], 's00')
        self.assertFalse(out['native_contacted'])
        self.assertEqual(len(self.peer.calls), calls)
        self.assertEqual(journal.read_bytes(), uncertain)
        out = self.query()
        self.assertEqual(out['placed'], 1)
        state = storage.replay(journal.read_bytes())
        self.assertEqual(state.reconciled_from.phase, 'indeterminate')
        self.assertTrue(journal.read_bytes().startswith(uncertain))
        self.assertEqual(self.peer.applied, 1)

    def test_resolved_receipt_recovery_stays_offline_with_invalid_stop(self):
        self.initialize()
        self.attempt(mode='placed')
        self.marker(b'broken')
        calls = len(self.peer.calls)
        with patch.dict(os.environ, {}, clear=True):
            out = batch.recover(str(self.root), self.batch_id, 's00')
            selected = self.inspect('s00')
        self.assert_stopped(out, b'broken')
        self.assertFalse(out['native_contacted'])
        self.assertEqual(out['placed'], 1)
        self.assertIn('canonical_record_hex', selected['receipt'])
        self.assertEqual(len(self.peer.calls), calls)

    def test_missing_native_receipt_cannot_resolve_uncertainty(self):
        self.initialize()
        self.attempt(mode='indeterminate')
        self.marker(b'broken')
        self.peer.query_mode = 'absent'
        journal = next((self.root / 'effects').iterdir())
        prior = journal.read_bytes()
        result = self.query()
        self.assert_stopped(result, b'broken')
        self.assertEqual(result['pending_step'], 's00')
        self.assertEqual(journal.read_bytes(), prior)

    def test_reset_or_forged_receipt_cannot_discharge_the_original_attempt(self):
        for mode in ('malformed', 'wrong_plan', 'refused', 'lost', 'generation'):
            with self.subTest(mode=mode):
                # Each independent source/custody fixture is disposed, never repaired.
                with tempfile.TemporaryDirectory() as tmp:
                    self.root = Path(tmp) / 'batch'; self.root.mkdir(mode=0o700)
                    self.peer.records.clear(); self.peer.generation = 7
                    self.peer.query_mode = 'resolve'
                    self.initialize(); self.attempt(mode='indeterminate'); self.marker(b'{')
                    journal = next((self.root / 'effects').iterdir()); prior = journal.read_bytes()
                    self.peer.query_mode = 'resolve' if mode == 'generation' else mode
                    if mode == 'generation': self.peer.generation += 1
                    with self.assertRaises(ValueError): self.query()
                    self.assertEqual(journal.read_bytes(), prior)
                    self.assertEqual((self.root / 'stop.json').read_bytes(), b'{')
                    os.environ[rpc.PLACE] = '1'

    def test_recovery_rejects_other_metadata_corruption_before_native_contact(self):
        self.initialize(); self.attempt(raw=b'{')
        for name in ('batch.json', 'steps.jsonl'):
            path = self.root / name; original = path.read_bytes()
            for raw in (b'', b'{'):
                path.write_bytes(raw)
                calls = len(self.peer.calls)
                with self.subTest(name=name, raw=raw), self.assertRaises(ValueError): self.query()
                self.assertEqual(len(self.peer.calls), calls)
            path.write_bytes(original)
        journal = next((self.root / 'effects').iterdir()); prior = journal.read_bytes()
        journal.write_bytes(prior[:-1])
        with self.assertRaises(ValueError): self.query()
        self.assertEqual(journal.read_bytes(), prior[:-1])

    def test_wrong_batch_or_unstarted_key_does_not_contact_native(self):
        self.initialize(); self.attempt(raw=b'{')
        calls = len(self.peer.calls)
        for identity, name in (('0' * 64, 's00'), (self.batch_id, 's01'), (self.batch_id, 'foreign')):
            with self.subTest(identity=identity, name=name), self.assertRaises(ValueError):
                batch.recover(str(self.root), identity, name)
        self.assertEqual(len(self.peer.calls), calls)

    def test_all_32_original_slots_remain_visible_and_unexecutable(self):
        self.initialize(32); self.marker(b'')
        out = self.inspect()
        self.assert_stopped(out, b'')
        self.assertEqual(out['total'], 32)
        self.assertEqual(len(out['steps']), 32)
        calls = len(self.peer.calls)
        for function in (lambda: batch.review(str(self.root), self.batch_id),
                         lambda: batch.advance(str(self.root), self.batch_id, '0' * 64, '0' * 64)):
            with self.assertRaises(ValueError): function()
        self.assertEqual(len(self.peer.calls), calls)
        with batch.Batch(str(self.root), rpc.Budget(10000), _recovery=True) as owner:
            with self.assertRaises(ValueError): batch.select_next(owner, self.batch_id)

    def test_damaged_marker_is_not_a_bypass_for_file_identity_and_privacy(self):
        self.initialize(); marker = self.marker(b'{')
        marker.chmod(0o644)
        with self.assertRaises(ValueError): self.inspect()
        marker.chmod(0o600)
        other = self.root.parent / 'other'
        os.link(marker, other)
        with self.assertRaises(ValueError): self.inspect()
        other.unlink(); marker.unlink(); marker.symlink_to(self.root / 'batch.json')
        with self.assertRaises((ValueError, OSError)): self.inspect()
        marker.unlink(); marker.mkdir()
        with self.assertRaises((ValueError, OSError)): self.inspect()

    def test_recovery_empty_permission_is_closed_to_stop_file_reading(self):
        self.initialize()
        with batch.root_lock(str(self.root), rpc.Budget(10000)) as fd:
            for name, writable, initial in (('batch.json', False, None), ('steps.jsonl', False, None),
                                            ('stop.json', True, None), ('stop.json', False, b'x')):
                with self.assertRaises(ValueError):
                    batch.File(fd, name, rpc.Budget(10000), writable, initial, recovery_stop=True)
        for invalid in (1, None, 'true'):
            with self.assertRaises(ValueError):
                batch.Batch(str(self.root), rpc.Budget(10000), _recovery=invalid)

    def test_marker_is_pinned_through_native_recovery_and_changed_bytes_refuse_output(self):
        self.initialize(); self.attempt(raw=b'{')
        self.peer.hook = lambda name, fields: self.marker(b'changed') if name == 'QueryPlacement' else None
        with self.assertRaises(ValueError): self.query()
        self.peer.hook = None
        self.assertEqual(self.peer.applied, 1)
        self.assertEqual((self.root / 'stop.json').read_bytes(), b'changed')
        # A valid returned receipt may already be retained; it never re-enables work.
        result = self.inspect()
        self.assertFalse(result['advance_allowed'])

    def test_native_recovery_still_requires_current_query_authority(self):
        self.initialize(); self.attempt(raw=b'{')
        calls = len(self.peer.calls)
        with patch.dict(os.environ, {}, clear=True), self.assertRaises(ValueError):
            batch.recover(str(self.root), self.batch_id, 's00')
        self.assertEqual(len(self.peer.calls), calls)
        self.assertEqual(self.inspect()['pending_step'], 's00')

    def test_recovery_uses_original_deadline_and_closes_all_descriptors_on_interruption(self):
        self.initialize(); self.marker(b'{')
        class CountingBudget(rpc.Budget):
            def __init__(self, stop=None):
                super().__init__(10000); self.calls = 0; self.stop = stop
            def remaining(self):
                self.calls += 1
                if self.calls == self.stop: raise InterruptedError('injected checkpoint interruption')
                return super().remaining()
        count = CountingBudget()
        batch.inspect(str(self.root), self.batch_id, _budget=count)
        before = len(os.listdir('/proc/self/fd'))
        for stop in range(1, count.calls + 1):
            with self.subTest(stop=stop), self.assertRaises(InterruptedError):
                batch.inspect(str(self.root), self.batch_id, _budget=CountingBudget(stop))
            self.assertEqual(len(os.listdir('/proc/self/fd')), before)
        exhausted = rpc.Budget(10000); exhausted.deadline = 0
        with self.assertRaises(ValueError): self.query(budget=exhausted)

    def test_actual_stop_signal_interrupts_locked_owner_and_retains_original_receipts(self):
        self.initialize()
        def stop(name, fields):
            if name == 'CommitPlacement':
                result = signal.request_stop(self.root, self.batch_id, StopBudget())
                self.assertTrue(result['ok'])
        self.peer.hook = stop
        proposed = batch.review(str(self.root), self.batch_id)
        with self.assertRaises(ValueError):
            batch.advance(str(self.root), self.batch_id, proposed['expected_plan'], proposed['confirm_review'])
        self.peer.hook = None
        out = self.inspect('s00')
        self.assertTrue(out['stopped'])
        self.assertEqual(out['placed'], 1)
        self.assertNotIn('stop_evidence', out)
        self.assertEqual(self.peer.applied, 1)
        self.assertIn('canonical_record_hex', out['receipt'])

    def test_agent_turn_distinguishes_verified_history_from_unverified_stop(self):
        self.initialize(); self.marker(b'{')
        out = self.inspect()
        packet = batch.packet('inspect', out)
        self.assertIsNone(packet['agent_turn']['anchor'])
        import json
        encoded = json.loads(batch.encoded('inspect', out))
        self.assertTrue(encoded['ok'])
        self.assertEqual(encoded['agent_turn']['attention'][0]['kind'], 'unverified_stop_marker')
        self.assertFalse(encoded['agent_turn']['attention'][0]['marker_validated'])
        self.assertNotIn(b'"marker_raw"', batch.encoded('inspect', out))


if __name__ == '__main__':
    unittest.main()
