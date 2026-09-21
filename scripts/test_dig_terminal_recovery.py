#!/usr/bin/env python3
"""Actual CLI/socket/file recovery tests; no live DFHack or power-loss claim."""
from __future__ import annotations

import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch

import dig_designation_client as d
from test_dig_designation_client import FakeGame, Peer, REGION, TOKEN, VECTORS, intent, outcome, rehash


class TerminalRecoveryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='dfmcp-dig-terminal-')
        self.root = Path(self.temp.name).resolve()
        os.chmod(self.root, 0o700)
        self.path = self.root / 'intent.json'

    def tearDown(self):
        self.temp.cleanup()

    def create(self, value=None):
        with d.capsule(self.path, value or intent()) as owner:
            return self.path.parent / d.terminal_name(owner)

    def start(self, client):
        value = intent(client.address)
        return d.start(client, self.path, 'dig-001', REGION, False, value['witness'], value['plan_digest'])

    def command(self, name, env=None, no_native=False):
        out = io.StringIO()
        with contextlib.ExitStack() as stack:
            stack.enter_context(patch.dict(os.environ, env or {}, clear=True))
            stack.enter_context(contextlib.redirect_stdout(out))
            if no_native:
                stack.enter_context(patch.object(d, 'Client', side_effect=AssertionError('unexpected native connection')))
            status = d.main([name, '--record', str(self.path)])
        value = json.loads(out.getvalue())
        self.assertNotIn(TOKEN.decode(), out.getvalue())
        self.assertFalse(value['retry_commit_permitted'])
        self.assertFalse(value['excavation_completion_proven'])
        self.assertLess(len(out.getvalue()), d.MAX_OUTPUT)
        return status, value

    def test_success_survives_reopen_without_credentials_or_native_access(self):
        game = FakeGame()
        with Peer(game) as peer, d.Client(peer.address, TOKEN, 3000) as client:
            result = self.start(client)
        original = self.path.read_bytes()
        self.assertTrue(result['terminal_receipt_retained'])
        for name in ('inspect', 'query', 'cancel'):
            status, loaded = self.command(name, no_native=True)
            self.assertEqual(status, 0)
            self.assertEqual(loaded['effect'], result['effect'])
            self.assertEqual(loaded['native_calls'], 0)
            self.assertEqual(loaded['evidence_source'], 'retained_terminal_receipt')
            self.assertEqual(loaded['terminal_receipt_sha256'], result['terminal_receipt_sha256'])
            self.assertFalse(loaded['commit_attempted_this_call'])
            self.assertTrue(loaded['effect']['current_terrain_unproved'])
        self.assertEqual(self.path.read_bytes(), original)
        self.assertEqual(game.calls.count('CommitDesignation'), 1)

    def test_lost_reply_query_publishes_proof_without_recommitting(self):
        game = FakeGame(); game.commit_mode = 'lost'
        with Peer(game, connections=2) as peer:
            with d.Client(peer.address, TOKEN, 3000) as client:
                with self.assertRaises(d.Rejected):
                    self.start(client)
            status, before = self.command('inspect', no_native=True)
            self.assertEqual(status, 0); self.assertEqual(before['effect_status'], 'unknown')
            original = self.path.read_bytes()
            env = {'DFMCP_ALLOW_UNADMITTED_DIG_V1_16': '1', 'DFMCP_DIG_TOKEN': TOKEN.decode()}
            status, result = self.command('query', env)
            self.assertEqual(status, 0); self.assertEqual(result['effect_status'], 'designated')
            self.assertTrue(result['terminal_receipt_retained'])
        status, loaded = self.command('inspect', no_native=True)
        self.assertEqual(status, 0); self.assertEqual(loaded['effect'], result['effect'])
        self.assertEqual(self.path.read_bytes(), original)
        self.assertEqual(game.calls.count('CommitDesignation'), 1)
        self.assertEqual(game.calls[-2:], ['Handshake', 'QueryDesignation'])

    def test_cancel_persists_only_proved_pre_dispatch_refusal(self):
        game = FakeGame(); game.record = VECTORS['prepared']
        with Peer(game) as peer:
            self.create(intent(peer.address))
            env = {'DFMCP_ALLOW_UNADMITTED_DIG_V1_16': '1', 'DFMCP_DIG_TOKEN': TOKEN.decode(),
                   'DFMCP_DIG_ALLOW_DESIGNATE': '1'}
            status, result = self.command('cancel', env)
            self.assertEqual(status, 0)
            self.assertEqual(result['effect']['reason'], 'cancelled_before_dispatch')
        status, loaded = self.command('inspect', no_native=True)
        self.assertEqual(status, 0); self.assertEqual(loaded['effect_status'], 'refused')
        self.assertNotIn('CommitDesignation', game.calls)

    def test_prepared_unknown_and_missing_records_never_become_terminal(self):
        receipt_path = self.create()
        with d.capsule(self.path) as owner:
            for raw in (VECTORS['prepared'], outcome(1)):
                native = {'manifest': owner.intent['manifest'], 'effect_raw': raw, 'effect': d.effect(raw, owner.intent)}
                self.assertFalse(d.finish_recovery(owner, native, False)['terminal_receipt_retained'])
                with self.assertRaises(d.Rejected):
                    d.terminal_receipt(owner, raw)
            missing = d.finish_recovery(owner, {'manifest': owner.intent['manifest']}, False)
            self.assertEqual(missing['effect_status'], 'unknown')
        self.assertFalse(receipt_path.exists())
        self.assertEqual(self.command('inspect', no_native=True)[1]['effect_status'], 'unknown')

    def test_file_and_parent_sync_failures_cannot_acknowledge_but_can_recover_complete_proof(self):
        real_sync = os.fsync
        for failure in (7, 8):  # intent, registry header and registration precede terminal proof
            directory = self.root / f'sync-{failure}'
            directory.mkdir(mode=0o700)
            self.path = directory / 'intent.json'
            calls = []
            def sync(fd):
                calls.append(stat.S_ISDIR(os.fstat(fd).st_mode))
                if len(calls) == failure:
                    raise OSError('injected terminal sync failure')
                real_sync(fd)
            game = FakeGame()
            with Peer(game) as peer, d.Client(peer.address, TOKEN, 3000) as client:
                with patch.object(d.os, 'fsync', side_effect=sync), self.assertRaises(OSError):
                    self.start(client)
            self.assertEqual(game.calls.count('CommitDesignation'), 1)
            self.assertEqual(calls, ([False, True] * 4)[:failure])
            # Reopen verifies and re-syncs, rather than inferring that the failed
            # invocation durably acknowledged its otherwise complete proof.
            observed = []
            def resync(fd):
                observed.append(stat.S_ISDIR(os.fstat(fd).st_mode)); real_sync(fd)
            with patch.object(d.os, 'fsync', side_effect=resync):
                status, loaded = self.command('inspect', no_native=True)
            self.assertEqual(status, 0); self.assertEqual(loaded['effect_status'], 'designated')
            self.assertEqual(observed, [False, True])

    def test_offline_sync_failure_is_not_a_successful_durable_acknowledgement(self):
        self.create()
        with d.capsule(self.path) as owner:
            d.terminal_receipt(owner, VECTORS['designated'])
        with patch.object(d.os, 'fsync', side_effect=OSError('offline sync failed')):
            status, loaded = self.command('inspect', no_native=True)
        self.assertEqual(status, 2); self.assertEqual(loaded['effect_status'], 'unknown')

    def test_partial_write_is_retained_and_never_repaired_or_redispatched(self):
        real_write = os.write
        writes = []
        def write(fd, data):
            name = os.readlink(f'/proc/self/fd/{fd}')
            if '/.dfmcp-dig-terminal-' in name:
                writes.append(fd)
                if len(writes) == 1:
                    return real_write(fd, data[:29])
                raise OSError('partial terminal write')
            return real_write(fd, data)
        game = FakeGame()
        with Peer(game) as peer, d.Client(peer.address, TOKEN, 3000) as client:
            with patch.object(d.os, 'write', side_effect=write), self.assertRaises(OSError):
                self.start(client)
        with d.capsule(self.path) as owner:
            damaged = self.root / d.terminal_name(owner)
        original = damaged.read_bytes()
        self.assertEqual(len(original), 29)
        for operation in ('inspect', 'query', 'cancel'):
            status, _ = self.command(operation, no_native=True)
            self.assertEqual(status, 2)
        self.assertEqual(damaged.read_bytes(), original)
        self.assertEqual(game.calls.count('CommitDesignation'), 1)

    def test_identical_receipt_is_idempotent_and_conflicts_do_not_overwrite(self):
        receipt_path = self.create()
        with d.capsule(self.path) as owner:
            first = d.terminal_receipt(owner, VECTORS['designated'])
            inode = receipt_path.stat().st_ino; original = receipt_path.read_bytes()
            self.assertEqual(d.terminal_receipt(owner, VECTORS['designated']), first)
            for raw in (VECTORS['cancelled'], outcome(4, 1)):
                with self.assertRaises(d.Rejected):
                    d.terminal_receipt(owner, raw)
            self.assertEqual(d.terminal_receipt(owner), first)
        self.assertEqual(receipt_path.stat().st_ino, inode)
        self.assertEqual(receipt_path.read_bytes(), original)

    def test_outer_rehash_cannot_forge_native_evidence_or_transplant_intent(self):
        self.create()
        with d.capsule(self.path) as owner:
            valid = d.terminal_payload(owner, VECTORS['designated'])
            for offset in (16, 52, 85, 117, 135, 139, 140, 171, 206):
                bad = bytearray(VECTORS['designated']); bad[offset] ^= 1
                native = rehash(bytes(bad))
                payload = json.loads(valid)['receipt']; payload['effect_hex'] = native.hex()
                data = d.canonical({'receipt': payload, 'sha256': hashlib.sha256(d.canonical(payload)).hexdigest()}) + b'\n'
                with self.assertRaises(d.Rejected):
                    d.verify_terminal(owner, data)
            for raw in (VECTORS['prepared'], outcome(1)):
                payload = json.loads(valid)['receipt']; payload['effect_hex'] = raw.hex()
                data = d.canonical({'receipt': payload, 'sha256': hashlib.sha256(d.canonical(payload)).hexdigest()}) + b'\n'
                with self.assertRaises(d.Rejected):
                    d.verify_terminal(owner, data)
        self.path = self.root / 'other.json'
        self.create(intent('127.0.0.1:5001'))
        with d.capsule(self.path) as owner, self.assertRaises(d.Rejected):
            d.verify_terminal(owner, valid)

    def test_complete_canonical_envelope_is_required(self):
        self.create()
        with d.capsule(self.path) as owner:
            good = d.terminal_payload(owner, VECTORS['designated'])
            for data in (b'', b' ' * (d.MAX_TERMINAL_RECEIPT + 1), good[:-1], good + b'\n',
                         good.replace(b'"receipt":', b'"extra":null,"receipt":'),
                         good.replace(b'"receipt":', b'"receipt":null,"receipt":')):
                with self.assertRaises((d.Rejected, ValueError)):
                    d.verify_terminal(owner, data)
            for size in range(len(good)):
                with self.assertRaises((d.Rejected, ValueError)):
                    d.verify_terminal(owner, good[:size])
            self.assertLess(len(good), d.MAX_TERMINAL_RECEIPT)

    def test_receipt_symlink_hardlink_modes_and_fifo_are_rejected(self):
        receipt_path = self.create()
        with d.capsule(self.path) as owner:
            data = d.terminal_payload(owner, VECTORS['designated'])
            for mode in (0o400, 0o640, 0o666):
                receipt_path.write_bytes(data); receipt_path.chmod(mode)
                with self.assertRaises(d.Rejected):
                    d.terminal_receipt(owner)
                receipt_path.unlink()
            target = self.root / 'elsewhere'; target.write_bytes(data); target.chmod(0o600)
            receipt_path.symlink_to(target)
            with self.assertRaises(OSError):
                d.terminal_receipt(owner)
            receipt_path.unlink(); os.link(target, receipt_path)
            with self.assertRaises(d.Rejected):
                d.terminal_receipt(owner)
            receipt_path.unlink(); os.mkfifo(receipt_path, 0o600)
            with self.assertRaises(d.Rejected):
                d.terminal_receipt(owner)
            self.assertEqual(target.read_bytes(), data)

    def test_receipt_substitution_during_sync_is_rejected(self):
        receipt_path = self.create()
        with d.capsule(self.path) as owner:
            d.terminal_receipt(owner, VECTORS['designated'])
            original = receipt_path.read_bytes(); real_sync = os.fsync; count = []
            def substitute(fd):
                real_sync(fd); count.append(fd)
                if len(count) == 1:
                    receipt_path.rename(self.root / 'old')
                    receipt_path.write_bytes(original); receipt_path.chmod(0o600)
            with patch.object(d.os, 'fsync', side_effect=substitute), self.assertRaises(d.Rejected):
                d.terminal_receipt(owner)

    def test_regressed_or_mismatched_native_replies_cannot_erase_terminal_history(self):
        self.create()
        with d.capsule(self.path) as owner:
            d.terminal_receipt(owner, VECTORS['designated'])
            for raw in (VECTORS['prepared'], outcome(1), None):
                reply = {'manifest': owner.intent['manifest']}
                if raw is not None:
                    reply.update(effect_raw=raw, effect=d.effect(raw, owner.intent))
                with self.assertRaises(d.Rejected):
                    d.finish_recovery(owner, reply, False)
            wrong = {'manifest': {**owner.intent['manifest'], 'generation': 8},
                     'effect_raw': VECTORS['designated'], 'effect': d.effect(VECTORS['designated'], owner.intent)}
            with self.assertRaises(d.Rejected):
                d.finish_recovery(owner, wrong, False)
            self.assertEqual(d.terminal_receipt(owner)['effect']['state'], 'designated')

    def test_source_change_during_recovery_does_not_create_terminal_proof(self):
        game = FakeGame(); game.source_generation = 8
        with Peer(game) as peer:
            receipt_path = self.create(intent(peer.address))
            env = {'DFMCP_ALLOW_UNADMITTED_DIG_V1_16': '1', 'DFMCP_DIG_TOKEN': TOKEN.decode()}
            status, loaded = self.command('query', env)
            self.assertEqual(status, 2); self.assertEqual(loaded['effect_status'], 'unknown')
        self.assertFalse(receipt_path.exists())
        self.assertEqual(game.calls, ['Handshake'])


if __name__ == '__main__':
    unittest.main(verbosity=2)
