"""Real POSIX custody, subprocess and foreground lifecycle tests; no live game."""
from __future__ import annotations

from contextlib import redirect_stdout
from dataclasses import replace
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

import excavation_run_client as cli
import excavation_run_rpc as rpc
import excavation_run_store as s
import excavation_run_wire as w
from test_excavation_run_client import NativeDouble, REGION, SPEC, SECRET, capture, plan, record, stopped

MANIFEST = rpc.Manifest(41, 'fake-df', 'fake-dfhack')
SCRIPTS = Path(__file__).resolve().parent


def reply(raw, generation=41):
    return rpc.Reply(replace(MANIFEST, generation=generation), False, 1, record=w.Record.decode(raw))


def fresh_server(**kwargs):
    user_hook = kwargs.pop('reply_hook', None)
    server = NativeDouble(**kwargs)
    def hook(name, fields, response):
        if name == 'QueryRun' and 'PrepareRun' not in server.calls:
            response.pop(10, None)
            response[11] = response[12] = 0
        return user_hook(name, fields, response) if user_hook else response
    server.reply_hook = hook
    return server


class StoreHelpers(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix='dfmcp-excavation-client-'))
        self.root.chmod(0o700)

    def owner(self, writable=True):
        return s.RunDirectory(str(self.root), rpc.Budget(10000), writable)

    def create(self, key='golden', address=('127.0.0.1', 5000), terminal=None):
        with self.owner() as owner:
            journal = owner.create(plan(key), MANIFEST, address)
            if terminal is not None:
                journal.retain(reply(terminal))
        return self.root / s.filename(key)

    def invoke(self, *args):
        output = io.StringIO()
        with redirect_stdout(output):
            code = cli.main(list(args))
        return code, json.loads(output.getvalue())

    def start_args(self, key='golden'):
        return ('start', '--directory', str(self.root), '--key', key, '--region', *map(str, REGION.values()),
                '--spec', *map(str, SPEC.values()), '--expected-plan', plan(key).digest.hex())


class StoreTests(StoreHelpers):
    def test_complete_journal_replay_and_terminal_idempotence(self):
        with self.owner() as owner:
            journal = owner.create(plan(), MANIFEST, ('127.0.0.1', 5000))
            journal.retain(reply(record()), prepared=True)
            journal.append('dispatch', {'plan_digest': plan().digest.hex()})
            self.assertTrue(journal.retain(reply(stopped())))
            original = journal.raw
            self.assertFalse(journal.retain(reply(stopped())))
            self.assertEqual(journal.raw, original)
            self.assertEqual(journal.state.frames, 4)
        with self.owner(False) as owner:
            state = owner.get('golden').state
            self.assertTrue(state.dispatched)
            self.assertFalse(state.pending)
            self.assertEqual(state.terminal.trigger, 'floor_observed')
            self.assertFalse(state.view()['storage_acknowledged_this_call'])

    def test_every_incomplete_prefix_and_one_byte_corruption(self):
        path = self.create(terminal=stopped())
        raw = path.read_bytes()
        boundaries = {i + 1 for i, byte in enumerate(raw) if byte == 10}
        for offset in range(len(raw)):
            if offset in boundaries:
                self.assertTrue(s.replay(raw[:offset]).pending)
            else:
                with self.subTest(prefix=offset), self.assertRaises((ValueError, TypeError, KeyError)):
                    s.replay(raw[:offset])
            bad = raw[:offset] + bytes([raw[offset] ^ 128]) + raw[offset + 1:]
            with self.subTest(byte=offset), self.assertRaises((ValueError, TypeError, KeyError)):
                s.replay(bad)

    def test_rehashed_transition_reordering_and_false_receipts(self):
        first = s.frame_bytes(None, 'intent', s.intent(plan(), MANIFEST, ('127.0.0.1', 5000)))
        state = s.replay(first)
        for kind, payload in (('dispatch', {'plan_digest': plan().digest.hex()}),
                              ('prepared', {'record_hex': stopped().hex(), 'manifest': MANIFEST.view()}),
                              ('terminal', {'record_hex': record().hex(), 'manifest': MANIFEST.view()}),
                              ('terminal', {'record_hex': stopped('wrong').hex(), 'manifest': MANIFEST.view()})):
            with self.subTest(kind=kind), self.assertRaises(w.Rejected):
                s.frame_bytes(state, kind, payload)
        with self.owner() as owner:
            journal = owner.create(plan(), MANIFEST, ('127.0.0.1', 5000))
            journal.retain(reply(record()), prepared=True)
            journal.append('dispatch', {'plan_digest': plan().digest.hex()})
            original = journal.raw
            with self.assertRaises(w.Rejected):
                journal.append('dispatch', {'plan_digest': plan().digest.hex()})
            self.assertEqual(journal.raw, original)

    def test_conflicting_terminal_and_public_field_substitution(self):
        with self.owner() as owner:
            journal = owner.create(plan(), MANIFEST, ('127.0.0.1', 5000))
            fake = replace(reply(stopped()), record=replace(w.Record.decode(stopped()), pause_verified=False))
            with self.assertRaises(w.Rejected):
                journal.retain(fake)
            journal.retain(reply(stopped()))
            before = journal.raw
            with self.assertRaises(w.Rejected):
                journal.retain(reply(record(phase=3, reason=2)))
            self.assertEqual(journal.raw, before)

    def test_pending_unknown_and_source_lost_block_new_keys(self):
        for name, terminal in (('unknown', None), ('source_lost', record(phase=5, reason=7))):
            root = self.root / name
            root.mkdir(mode=0o700)
            with s.RunDirectory(str(root), rpc.Budget(10000), True) as owner:
                journal = owner.create(plan(), MANIFEST, ('127.0.0.1', 5000))
                if terminal:
                    journal.retain(reply(terminal))
                with self.assertRaises(w.Rejected):
                    owner.ready('another')
                self.assertEqual(owner.inventory()['pending'], 1)
                self.assertEqual(owner.inventory()['operator_attention'], int(terminal is not None))

    def test_terminal_history_allows_new_key_not_old_key(self):
        self.create(terminal=stopped())
        with self.owner() as owner:
            owner.ready('new')
            with self.assertRaises(w.Rejected):
                owner.ready('golden')
            owner.create(plan('new'), MANIFEST, ('127.0.0.1', 5000))
            self.assertEqual(owner.inventory()['total'], 2)

    def test_inventory_pending_first_complete_counts_and_stale_pages(self):
        with self.owner() as owner:
            for number in range(4):
                key = f'a{number}'
                j = owner.create(plan(key), MANIFEST, ('127.0.0.1', 5000))
                j.retain(reply(stopped(key)))
            j = owner.create(plan('z-pending'), MANIFEST, ('127.0.0.1', 5000))
            first = owner.inventory(2)
            self.assertEqual((first['total'], first['pending']), (5, 1))
            self.assertEqual(first['rows'][0]['key'], 'z-pending')
            second = owner.inventory(2, first['continuation'])
            self.assertEqual((second['total'], second['pending']), (5, 1))
            with self.assertRaises(w.Rejected):
                owner.inventory(3, first['continuation'])
            j.retain(reply(stopped('z-pending')))
            with self.assertRaises(w.Rejected):
                owner.inventory(2, first['continuation'])

    def test_modes_symlink_hardlink_fifo_and_parent_substitution(self):
        path = self.create()
        path.chmod(0o400)
        with self.assertRaises(w.Rejected):
            self.owner()
        path.chmod(0o600)
        os.link(path, self.root / 'other.exrun')
        with self.assertRaises(w.Rejected):
            self.owner()
        for kind in ('symlink', 'fifo'):
            root = self.root / kind
            root.mkdir(mode=0o700)
            target = root / 'golden.exrun'
            if kind == 'symlink':
                target.symlink_to(path)
            else:
                os.mkfifo(target, 0o600)
            with self.assertRaises((OSError, w.Rejected)):
                s.RunDirectory(str(root), rpc.Budget(1000), True)
        root = self.root / 'replace'
        root.mkdir(mode=0o700)
        with s.RunDirectory(str(root), rpc.Budget(1000), True) as owner:
            root.rename(self.root / 'moved')
            root.mkdir(mode=0o700)
            with self.assertRaises(w.Rejected):
                owner.check()

    def test_same_size_changes_and_new_membership_fence_owner(self):
        path = self.create()
        with self.owner() as owner:
            original = path.read_bytes()
            path.write_bytes(original.replace(b'fake-df', b'fuke-df'))
            with self.assertRaises(w.Rejected):
                owner.check()
        other = self.root / 'other'
        other.mkdir(mode=0o700)
        with s.RunDirectory(str(other), rpc.Budget(1000), True) as owner:
            (other / 'extra.exrun').write_bytes(b'bad')
            with self.assertRaises(w.Rejected):
                owner.ready('golden')

    def test_directory_lock_and_offline_subprocess_inspection(self):
        self.create(terminal=stopped())
        args = [sys.executable, str(SCRIPTS / 'excavation_run_client.py'), 'inspect', '--directory', str(self.root), '--key', 'golden']
        with self.owner() as owner:
            blocked = subprocess.run(args, capture_output=True, text=True, timeout=5, env={})
            self.assertEqual(blocked.returncode, 2)
            owner.check()
        done = subprocess.run(args, capture_output=True, text=True, timeout=5, env={})
        self.assertEqual(done.returncode, 0, done.stdout)
        self.assertFalse(json.loads(done.stdout)['result']['native_contacted'])
        self.assertEqual(json.loads(done.stdout)['result']['effect']['trigger'], 'floor_observed')

    def test_output_and_bounds_before_effects(self):
        with self.assertRaises(w.Rejected):
            cli.bounded_output({'x': 'x' * 65536})
        with self.owner() as owner:
            for limit in (0, 65, True):
                with self.assertRaises(w.Rejected):
                    owner.inventory(limit)
            with self.assertRaises(w.Rejected):
                owner.create(plan(), replace(MANIFEST, generation=42), ('127.0.0.1', 5000))
            self.assertEqual(owner.names(), [])

    def test_short_writes_are_completed_and_partial_failures_preserved(self):
        real_write = os.write
        with self.owner() as owner, patch.object(s.os, 'write', side_effect=lambda fd, b: real_write(fd, b[:7])):
            journal = owner.create(plan(), MANIFEST, ('127.0.0.1', 5000))
            self.assertEqual(journal.state.plan, plan())
        root = self.root / 'partial'
        root.mkdir(mode=0o700)
        calls = 0
        def broken(fd, raw):
            nonlocal calls
            calls += 1
            if calls == 1:
                return real_write(fd, raw[:20])
            raise OSError('injected partial write')
        with s.RunDirectory(str(root), rpc.Budget(1000), True) as owner:
            with patch.object(s.os, 'write', side_effect=broken), self.assertRaises(OSError):
                owner.create(plan(), MANIFEST, ('127.0.0.1', 5000))
            self.assertTrue(owner.fenced)
        self.assertEqual((root / 'golden.exrun').stat().st_size, 20)
        with self.assertRaises(w.Rejected):
            s.RunDirectory(str(root), rpc.Budget(1000), True)

    def test_readonly_owner_cannot_publish_and_expired_owner_cannot_create(self):
        with self.owner(False) as owner:
            with self.assertRaises(w.Rejected):
                owner.create(plan(), MANIFEST, ('127.0.0.1', 5000))
        with self.owner() as owner:
            owner.budget.deadline = 0
            with self.assertRaises(w.Rejected):
                owner.create(plan(), MANIFEST, ('127.0.0.1', 5000))
        self.assertEqual(list(self.root.iterdir()), [])


class LifecycleTests(StoreHelpers):
    def test_confirmed_start_publishes_intent_and_dispatch_before_native_effect(self):
        observed = []
        def inspect(name, fields, response):
            if name in ('PrepareRun', 'CommitRun'):
                state = s.replay((self.root / 'golden.exrun').read_bytes())
                observed.append((name, state.prepared is not None, state.dispatched))
            return response
        with fresh_server(reply_hook=inspect) as server, server.environment():
            code, packet = self.invoke(*self.start_args())
        self.assertEqual(code, 0, packet)
        self.assertEqual(observed, [('PrepareRun', False, False), ('CommitRun', True, True)])
        self.assertEqual(server.effects, 1)
        self.assertEqual(packet['result']['effect_status'], 'running')
        self.assertTrue(packet['result']['pending'])
        self.assertEqual(server.calls, ['Handshake', 'ObserveRun', 'QueryRun', 'PrepareRun', 'CommitRun'])

    def test_lost_commit_then_new_process_query_and_offline_terminal(self):
        with fresh_server(lose_on='CommitRun', connections=2) as server, server.environment():
            code, packet = self.invoke(*self.start_args())
            self.assertEqual(code, 2)
            self.assertEqual(packet['effect_status'], 'unknown')
            env = dict(os.environ)
            env[rpc.CLOCK] = '0'
            completed = subprocess.run([sys.executable, str(SCRIPTS / 'excavation_run_client.py'), 'query',
                '--directory', str(self.root), '--key', 'golden'], capture_output=True, text=True, timeout=5, env=env)
            self.assertEqual(completed.returncode, 0, completed.stdout)
            self.assertTrue(json.loads(completed.stdout)['result']['storage_acknowledged_this_call'])
        self.assertEqual(server.effects, 1)
        with patch.dict(os.environ, {}, clear=True), patch.object(cli, 'Client', side_effect=AssertionError('offline contacted native')):
            code, packet = self.invoke('query', '--directory', str(self.root), '--key', 'golden')
        self.assertEqual(code, 0)
        self.assertFalse(packet['result']['native_contacted'])
        self.assertFalse(packet['result']['storage_acknowledged_this_call'])

    def test_killed_start_process_recovers_without_second_unpause(self):
        reached, release = threading.Event(), threading.Event()
        def hold(name, fields, response):
            if name == 'CommitRun':
                reached.set()
                if not release.wait(3):
                    raise AssertionError('parent did not finish crash injection')
            return response
        with fresh_server(connections=2, reply_hook=hold) as server, server.environment():
            child = subprocess.Popen([sys.executable, str(SCRIPTS / 'excavation_run_client.py'), *self.start_args()],
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                self.assertTrue(reached.wait(3))
                self.assertTrue(s.replay((self.root / 'golden.exrun').read_bytes()).dispatched)
                child.kill()
                child.communicate(timeout=3)
                release.set()
                code, packet = self.invoke('query', '--directory', str(self.root), '--key', 'golden')
                self.assertEqual(code, 0, packet)
                self.assertEqual(packet['result']['effect_status'], 'stopped')
                self.assertEqual(server.effects, 1)
            finally:
                release.set()
                if child.poll() is None:
                    child.kill()
                child.communicate(timeout=3)

    def test_stale_confirmation_and_existing_native_key_never_prepare(self):
        with fresh_server() as server, server.environment():
            args = list(self.start_args())
            args[-1] = '0' * 64
            code, _ = self.invoke(*args)
        self.assertEqual(code, 2)
        self.assertNotIn('PrepareRun', server.calls)
        self.assertEqual(list(self.root.iterdir()), [])
        with NativeDouble(query_raw=record()) as server, server.environment():
            code, _ = self.invoke(*self.start_args())
        self.assertEqual(code, 2)
        self.assertNotIn('PrepareRun', server.calls)
        self.assertEqual(list(self.root.iterdir()), [])

    def test_new_key_cannot_bypass_unresolved_or_corrupt_journal_before_connection(self):
        path = self.create()
        values = {rpc.OPT_IN: '1', rpc.TOKEN: SECRET.decode(), rpc.CLOCK: '1'}
        with patch.dict(os.environ, values, clear=True), patch.object(cli, 'Client', side_effect=AssertionError('should refuse before connect')):
            code, _ = self.invoke(*self.start_args('another'))
            self.assertEqual(code, 2)
            path.write_bytes(path.read_bytes()[:-1])
            code, _ = self.invoke('query', '--directory', str(self.root), '--key', 'golden')
            self.assertEqual(code, 2)

    def test_file_and_directory_sync_failures_prevent_commit(self):
        actual = os.fsync
        original_root = self.root
        for fail_at in range(1, 7):
            self.root = original_root / f'sync-{fail_at}'
            self.root.mkdir(mode=0o700)
            calls = 0
            def sync(fd):
                nonlocal calls
                calls += 1
                if calls == fail_at:
                    raise OSError('injected sync failure')
                actual(fd)
            with self.subTest(fail_at=fail_at), fresh_server() as server, server.environment(), patch.object(s.os, 'fsync', side_effect=sync):
                code, _ = self.invoke(*self.start_args())
            self.assertEqual(code, 2)
            self.assertEqual(server.effects, 0)
            self.assertNotIn('CommitRun', server.calls)
            self.assertEqual(calls, fail_at)
        self.root = original_root

    def test_clock_revocation_after_dispatch_sync_prevents_unpause(self):
        actual = os.fsync
        calls = 0
        def sync(fd):
            nonlocal calls
            actual(fd)
            calls += 1
            if calls == 6:
                os.environ[rpc.CLOCK] = '0'
        with fresh_server() as server, server.environment(), patch.object(s.os, 'fsync', side_effect=sync):
            code, _ = self.invoke(*self.start_args())
        self.assertEqual(code, 2)
        self.assertEqual(server.effects, 0)
        self.assertTrue(s.replay((self.root / 'golden.exrun').read_bytes()).dispatched)

    def test_expiry_after_dispatch_sync_prevents_unpause(self):
        actual = os.fsync
        with fresh_server() as server, server.environment(), self.owner() as owner:
            calls = 0
            def sync(fd):
                nonlocal calls
                actual(fd)
                calls += 1
                if calls == 6:
                    owner.budget.deadline = 0
            with patch.object(s.os, 'fsync', side_effect=sync), self.assertRaises(w.Rejected):
                cli.start(owner, rpc.Authority.load(True), REGION, SPEC, 'golden', plan().digest.hex())
            self.assertEqual(server.effects, 0)

    def test_absent_after_restart_keeps_pending_and_cannot_redispatch(self):
        with NativeDouble(query_raw=b'') as server, server.environment():
            self.create(address=server.address)
            code, packet = self.invoke('query', '--directory', str(self.root), '--key', 'golden')
        self.assertEqual(code, 0, packet)
        self.assertEqual(packet['result']['effect_status'], 'unknown_absent_native_record')
        self.assertTrue(packet['result']['pending'])
        self.assertEqual(server.effects, 0)
        with self.owner() as owner:
            with self.assertRaises(w.Rejected):
                owner.ready('another')

    def test_query_only_wait_and_cancel_once_with_revoked_clock(self):
        with NativeDouble(query_raw=record(phase=1)) as server, server.environment(clock='0'):
            self.create(address=server.address)
            code, packet = self.invoke('query', '--directory', str(self.root), '--key', 'golden',
                                       '--wait-ms', '1000', '--max-queries', '2')
        self.assertEqual(code, 0, packet)
        self.assertEqual(packet['result']['foreground_stop'], 'query_limit')
        self.assertEqual(server.calls, ['Handshake', 'QueryRun', 'QueryRun'])
        # Reuse the same journal with its original endpoint by opening a new
        # native double on that port is unnecessary: direct recovery checks the
        # endpoint mismatch before connecting, covered separately below.
        other = self.root / 'cancel'
        other.mkdir(mode=0o700)
        self.root = other
        with NativeDouble() as server, server.environment(clock='0'):
            self.create(address=server.address)
            code, packet = self.invoke('cancel', '--directory', str(self.root), '--key', 'golden')
        self.assertEqual(code, 0, packet)
        self.assertEqual(server.calls, ['Handshake', 'CancelRun'])
        self.assertTrue(packet['result']['effect']['historical_pause_verified'])

    def test_endpoint_and_software_mismatch_refuse_before_query(self):
        self.create()
        with patch.dict(os.environ, {rpc.OPT_IN: '1', rpc.TOKEN: SECRET.decode(), rpc.ENDPOINT: '127.0.0.1:5001'}, clear=True), patch.object(cli, 'Client', side_effect=AssertionError('wrong endpoint contacted')):
            code, _ = self.invoke('query', '--directory', str(self.root), '--key', 'golden')
        self.assertEqual(code, 2)
        other = self.root / 'software'
        other.mkdir(mode=0o700)
        self.root = other
        with NativeDouble(reply_hook=lambda n, f, r: {**r, 7: b'other-df'}) as server, server.environment():
            self.create(address=server.address)
            code, _ = self.invoke('query', '--directory', str(self.root), '--key', 'golden')
        self.assertEqual(code, 2)
        self.assertEqual(server.calls, ['Handshake'])

    def test_terminal_sync_failure_preserves_history_without_acknowledgement(self):
        with NativeDouble() as server, server.environment():
            path = self.create(address=server.address)
            with patch.object(s.os, 'fsync', side_effect=OSError('sync failed')):
                code, packet = self.invoke('query', '--directory', str(self.root), '--key', 'golden')
            self.assertEqual(code, 2)
            self.assertEqual(packet['effect_status'], 'unknown')
        self.assertTrue(s.replay(path.read_bytes()).terminal.terminal)
        with patch.dict(os.environ, {}, clear=True):
            code, packet = self.invoke('inspect', '--directory', str(self.root), '--key', 'golden')
        self.assertEqual(code, 0)
        self.assertFalse(packet['result']['storage_acknowledged_this_call'])

    def test_recovery_source_loss_is_retained_but_unresolved(self):
        with NativeDouble(query_raw=record(phase=5, reason=7)) as server, server.environment():
            self.create(address=server.address)
            code, packet = self.invoke('query', '--directory', str(self.root), '--key', 'golden')
        self.assertEqual(code, 0, packet)
        self.assertTrue(packet['result']['pending'])
        self.assertTrue(packet['result']['effect']['operator_attention_required'])

    def test_plan_command_returns_exact_confirmation_without_writes(self):
        with NativeDouble() as server, server.environment(clock=None):
            code, packet = self.invoke('plan', '--region', *map(str, REGION.values()),
                                       '--spec', *map(str, SPEC.values()))
        self.assertEqual(code, 0, packet)
        self.assertEqual(packet['result']['plan_digest'], plan().digest.hex())
        self.assertEqual(server.calls, ['Handshake', 'ObserveRun'])
        self.assertEqual(list(self.root.iterdir()), [])

    def test_invalid_cli_and_offline_inventory_do_not_connect(self):
        with patch.dict(os.environ, {}, clear=True), patch.object(cli, 'Client', side_effect=AssertionError('invalid/offline contacted native')):
            for args in (('start',), ('query', '--directory', str(self.root), '--key', 'golden', '--region', '0', '0', '0', '1', '1'),
                         ('inventory', '--directory', str(self.root), '--wait-ms', '1')):
                code, _ = self.invoke(*args)
                self.assertEqual(code, 2)
            code, packet = self.invoke('inventory', '--directory', str(self.root))
        self.assertEqual(code, 0)
        self.assertEqual(packet['result']['total'], 0)
        self.assertFalse(packet['result']['native_contacted'])



if __name__ == '__main__':
    unittest.main(verbosity=2)
