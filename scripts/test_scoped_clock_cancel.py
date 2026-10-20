"""Explicit original-key cancellation through real files, CLI and a TCP peer."""
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import bounded_run_client as clock
import bounded_run_outcomes as storage
import recover_route_furniture_clock as recovery
from scoped_clock_cancel import CancelClient
from test_scoped_clock_recovery import Fixture, Peer, env


class CancellationTests(unittest.TestCase):
    def test_exact_cancel_stops_or_reports_pending_without_starting_another_clock(self):
        for final in (False, True):
            for phase in ('stopped', 'stopping', 'refused'):
                with tempfile.TemporaryDirectory() as directory, Peer((phase,), cancel=True) as peer:
                    f = Fixture(directory, peer.address, final=final); peer.intent = f.intent
                    before = f.snapshot()
                    with patch.dict(os.environ, env(peer.address), clear=True), f.owner() as owner:
                        out = owner.cancel()
                    self.assertEqual(peer.calls, ['Handshake', 'CancelRun'])
                    self.assertTrue(out['clock_cancel_dispatched'])
                    self.assertFalse(out['clock_start_dispatched'])
                    self.assertFalse(out['placement_dispatched'])
                    self.assertTrue(out['current_pause_unproved'])
                    self.assertFalse(out['original_batch_custody_verified'])
                    self.assertEqual(out['recovery_required'], phase == 'stopping')
                    self.assertEqual(out['clock_result']['storage_acknowledged_this_call'], phase != 'stopping')
                    self.assertEqual({n: f.snapshot()[n] for n in before}, before)
                    self.assertEqual(f.outcome.exists(), phase != 'stopping')

    def test_terminal_cancellation_is_offline_and_receipt_bytes_remain_immutable(self):
        with tempfile.TemporaryDirectory() as directory:
            f = Fixture(directory, final=True); f.saved()
            before = f.snapshot()
            with patch.dict(os.environ, {}, clear=True), f.owner() as owner:
                with patch.object(socket, 'socket', side_effect=AssertionError('native call')):
                    out = owner.cancel()
            self.assertFalse(out['clock_cancel_dispatched'])
            self.assertEqual(before, f.snapshot())

    def test_lost_cancel_reply_has_no_retry_or_automatic_query(self):
        with tempfile.TemporaryDirectory() as directory, Peer(('drop',), cancel=True) as peer:
            f = Fixture(directory, peer.address); peer.intent = f.intent
            before = f.snapshot()
            with patch.dict(os.environ, env(peer.address), clear=True), self.assertRaises(ValueError), f.owner() as owner:
                owner.cancel()
            self.assertEqual(peer.calls, ['Handshake', 'CancelRun'])
            self.assertEqual(before, f.snapshot())

    def test_cancellation_connection_has_one_attempt_even_after_success(self):
        with tempfile.TemporaryDirectory() as directory, Peer(cancel=True) as peer:
            f = Fixture(directory, peer.address); peer.intent = f.intent
            with patch.dict(os.environ, env(peer.address), clear=True), f.owner() as owner:
                authority = recovery.Authority.load()
                with CancelClient(authority, owner.budget, owner.check, f.intent) as client:
                    client.call('CancelRun', clock.intent_fields(f.intent, True))
                    self.assertTrue(client.cancel_attempted)
                    with self.assertRaises(ValueError):
                        client.call('CancelRun', clock.intent_fields(f.intent, True))
            self.assertEqual(peer.calls, ['Handshake', 'CancelRun'])

    def test_cancellation_cannot_query_observe_prepare_or_commit(self):
        for operation in ('QueryRun', 'ObserveRun', 'PrepareRun', 'CommitRun'):
            with tempfile.TemporaryDirectory() as directory, Peer(cancel=True) as peer:
                f = Fixture(directory, peer.address); peer.intent = f.intent
                with patch.dict(os.environ, env(peer.address), clear=True), f.owner() as owner:
                    with CancelClient(recovery.Authority.load(), owner.budget, owner.check, f.intent) as client:
                        with self.assertRaises(ValueError):
                            client.call(operation, clock.intent_fields(f.intent, True))
                self.assertEqual(peer.calls, ['Handshake'])

    def test_key_plan_and_prepare_token_must_be_the_original_exact_tuple(self):
        for field, value in ((5, b'other'), (9, b'x' * 32), (10, b'x' * 16)):
            with tempfile.TemporaryDirectory() as directory, Peer(cancel=True) as peer:
                f = Fixture(directory, peer.address); peer.intent = f.intent
                with patch.dict(os.environ, env(peer.address), clear=True), f.owner() as owner:
                    with CancelClient(recovery.Authority.load(), owner.budget, owner.check, f.intent) as client:
                        changed = clock.intent_fields(f.intent, True); changed[field] = value
                        with self.assertRaises(ValueError): client.call('CancelRun', changed)
                self.assertEqual(peer.calls, ['Handshake'])

    def test_changed_or_regressed_native_generation_refuses_before_cancel(self):
        for generation in (10, 12):
            with tempfile.TemporaryDirectory() as directory, Peer(generation=generation, cancel=True) as peer:
                f = Fixture(directory, peer.address); peer.intent = f.intent
                with patch.dict(os.environ, env(peer.address), clear=True), self.assertRaises(ValueError), f.owner() as owner:
                    owner.cancel()
                self.assertEqual(peer.calls, ['Handshake'])
                self.assertFalse(f.outcome.exists())

    def test_contradictory_prepared_or_running_reply_does_not_acknowledge_stop(self):
        for phase in ('prepared', 'running'):
            with tempfile.TemporaryDirectory() as directory, Peer((phase,), cancel=True) as peer:
                f = Fixture(directory, peer.address); peer.intent = f.intent
                with patch.dict(os.environ, env(peer.address), clear=True), self.assertRaises(ValueError), f.owner() as owner:
                    owner.cancel()
                self.assertEqual(peer.calls, ['Handshake', 'CancelRun'])
                self.assertFalse(f.outcome.exists())

    def test_cancellation_sync_failure_preserves_original_evidence_without_ack(self):
        with tempfile.TemporaryDirectory() as directory, Peer(cancel=True) as peer:
            f = Fixture(directory, peer.address); peer.intent = f.intent
            before = f.snapshot()
            with patch.dict(os.environ, env(peer.address), clear=True), self.assertRaises(OSError), f.owner() as owner:
                with patch.object(storage.os, 'fsync', side_effect=OSError('sync failed')):
                    owner.cancel()
            self.assertEqual(peer.calls, ['Handshake', 'CancelRun'])
            self.assertEqual({n: f.snapshot()[n] for n in before}, before)
            self.assertTrue(f.outcome.exists())
            with f.owner() as owner:
                out = owner.inspect()
            self.assertFalse(out['clock_result']['storage_acknowledged_this_call'])

    def test_revocation_after_cancel_reply_cannot_publish_a_positive_result(self):
        def change(name, fields):
            if name == 'CancelRun': os.environ['DFMCP_RUN_TOKEN'] = 'z' * 32
        with tempfile.TemporaryDirectory() as directory, Peer(cancel=True, change=change) as peer:
            f = Fixture(directory, peer.address); peer.intent = f.intent
            with patch.dict(os.environ, env(peer.address), clear=True), self.assertRaises(ValueError), f.owner() as owner:
                owner.cancel()
            self.assertFalse(f.outcome.exists())
            self.assertEqual(peer.calls, ['Handshake', 'CancelRun'])

    def test_actual_cancel_cli_and_invalid_wait_do_not_open_a_batch(self):
        with tempfile.TemporaryDirectory() as directory, Peer(cancel=True) as peer:
            f = Fixture(directory, peer.address, final=True); peer.intent = f.intent
            command = [sys.executable, '-B', recovery.__file__]
            denied = subprocess.run(command + f.args('cancel', '--wait-ms', '10'), env=env(peer.address),
                                    capture_output=True, timeout=10)
            self.assertEqual(denied.returncode, 2)
            self.assertEqual(peer.calls, [])
            self.assertFalse(json.loads(denied.stdout)['ok'])
            result = subprocess.run(command + f.args('cancel'), env=env(peer.address),
                                    capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            packet = json.loads(result.stdout)
            self.assertTrue(packet['result']['clock_cancel_dispatched'])
            self.assertFalse(packet['result']['clock_start_dispatched'])
            self.assertEqual(peer.calls, ['Handshake', 'CancelRun'])
            self.assertEqual(len(result.stdout.splitlines()), 1)


if __name__ == '__main__':
    unittest.main()
