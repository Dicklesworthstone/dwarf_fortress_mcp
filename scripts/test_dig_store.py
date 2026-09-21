#!/usr/bin/env python3
"""Execute persistent store coordination; native traffic uses joined test peers."""
from __future__ import annotations
import contextlib
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

import dig_designation_client as d
import dig_designation_store as s
from test_dig_designation_client import FakeGame, Peer, REGION, TOKEN, VECTORS, intent, rehash


class StoreTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='dfmcp-dig-store-')
        self.root = Path(self.temp.name).resolve()
        self.root.chmod(0o700)

    def tearDown(self):
        self.temp.cleanup()

    def record(self, name='old.json', key='old', state=None, registered=False, address='127.0.0.1:5000'):
        value = d.build_intent(address, key, REGION, False, VECTORS['observation'], intent()['manifest'])
        path = self.root / name
        with d.capsule(path, value) as owner:
            if state:
                native = VECTORS['designated' if state == 'designated' else 'cancelled']
                native = native[:117] + bytes.fromhex(value['prepare_token']) + native[133:204] + d.key_bytes(key)
                d.terminal_receipt(owner, rehash(native))
            if registered:
                with s.open_store(d, self.root, True) as store:
                    store.register(owner)
        return path

    def start(self, client, name='new.json'):
        value = intent(client.address)
        return d.start(client, self.root / name, 'dig-001', REGION, False, value['witness'], value['plan_digest'])

    def test_legacy_unknown_is_registered_and_blocks_other_key_before_native_read(self):
        self.record()
        game = FakeGame()
        with Peer(game) as peer, d.Client(peer.address, TOKEN, 3000) as client:
            with self.assertRaises(d.Rejected):
                self.start(client)
        self.assertEqual(game.calls, ['Handshake'])
        self.assertFalse((self.root / 'new.json').exists())
        report = s.records_result(d, self.root)
        self.assertEqual(report['unresolved_records'], 1)
        self.assertTrue(report['records'][0]['registered'])
        self.assertFalse(report['mutation_authority_granted'])

    def test_lost_reply_fences_new_filename_after_reopen_then_query_releases_only_obligation(self):
        game = FakeGame(); game.commit_mode = 'lost'
        with Peer(game, connections=3) as peer:
            with d.Client(peer.address, TOKEN, 3000) as client:
                with self.assertRaises(d.Rejected):
                    self.start(client)
            with d.Client(peer.address, TOKEN, 3000) as client:
                with self.assertRaises(d.Rejected):
                    self.start(client, 'bypass.json')
            self.assertFalse((self.root / 'bypass.json').exists())
            with d.capsule(self.root / 'new.json') as owner, d.Client(peer.address, TOKEN, 3000) as client:
                d.finish_recovery(owner, client.query(owner.intent), False)
        self.assertEqual(game.calls.count('CommitDesignation'), 1)
        self.assertEqual(game.calls.count('PrepareDesignation'), 1)
        self.assertEqual(s.records_result(d, self.root)['unresolved_records'], 0)
        with s.open_store(d, self.root, True) as store:
            with self.assertRaises(d.Rejected):
                store.ready('bypass.json', 'dig-001')  # key reuse is not a new plan

    def test_terminal_history_allows_distinct_fresh_work_and_is_not_overwritten(self):
        old = self.record(state='refused', registered=True)
        original = old.read_bytes()
        game = FakeGame()
        with Peer(game) as peer, d.Client(peer.address, TOKEN, 3000) as client:
            self.assertEqual(self.start(client)['effect_status'], 'designated')
        report = s.records_result(d, self.root)
        self.assertEqual(report['total_records'], 2)
        self.assertEqual(report['unresolved_records'], 0)
        self.assertTrue(all(r['registered'] for r in report['records']))
        self.assertEqual({r['effect_status'] for r in report['records']}, {'refused', 'designated'})
        self.assertEqual(old.read_bytes(), original)
        self.assertEqual(game.calls.count('CommitDesignation'), 1)

    def test_registry_remembers_missing_or_substituted_intent(self):
        path = self.record(registered=True)
        original = path.read_bytes()
        for bad in (None, b'', original.replace(b'old', b'bad')):
            if path.exists():
                path.unlink()
            if bad is not None:
                path.write_bytes(bad); path.chmod(0o600)
            with self.assertRaises((d.Rejected, OSError, ValueError)):
                s.records_result(d, self.root)
            with s.open_store(d, self.root, True) as store, self.assertRaises((d.Rejected, OSError, ValueError)):
                store.ready('new.json', 'new')
        self.assertTrue((self.root / s.REGISTRY).exists())

    def test_header_and_entry_sync_failures_fence_all_preparation(self):
        real_sync = os.fsync
        for failure in (3, 4, 5, 6):
            with tempfile.TemporaryDirectory(dir=self.root) as directory:
                root = self.root; self.root = Path(directory); self.root.chmod(0o700)
                calls = []
                def sync(fd):
                    calls.append(fd)
                    if len(calls) == failure:
                        raise OSError('injected registry sync failure')
                    real_sync(fd)
                game = FakeGame()
                with Peer(game) as peer, d.Client(peer.address, TOKEN, 3000) as client:
                    with patch.object(d.os, 'fsync', side_effect=sync), self.assertRaises(OSError):
                        self.start(client)
                self.assertEqual(len(calls), failure)
                self.assertNotIn('PrepareDesignation', game.calls)
                self.assertNotIn('CommitDesignation', game.calls)
                self.assertTrue((self.root / 'new.json').exists())
                self.assertEqual(s.records_result(d, self.root)['unresolved_records'], 1)
                self.root = root

    def test_partial_registry_append_is_not_repaired_or_used_for_new_work(self):
        real_write = os.write; calls = []
        def write(fd, raw):
            if os.readlink(f'/proc/self/fd/{fd}').endswith(s.REGISTRY) and not bytes(raw).startswith(s.HEADER):
                calls.append(fd)
                if len(calls) == 1:
                    return real_write(fd, raw[:13])
                raise OSError('injected torn entry')
            return real_write(fd, raw)
        game = FakeGame()
        with Peer(game) as peer, d.Client(peer.address, TOKEN, 3000) as client:
            with patch.object(s.os, 'write', side_effect=write), self.assertRaises(OSError):
                self.start(client)
        before = (self.root / s.REGISTRY).read_bytes()
        self.assertEqual(len(before), len(s.HEADER) + 13)
        with self.assertRaises((d.Rejected, ValueError)):
            s.records_result(d, self.root)
        self.assertEqual((self.root / s.REGISTRY).read_bytes(), before)
        self.assertNotIn('PrepareDesignation', game.calls)

    def test_registry_change_after_prepare_prevents_commit(self):
        game = FakeGame()
        def corrupt():
            path = self.root / s.REGISTRY
            path.write_bytes(path.read_bytes() + b'x')
        game.after_prepare = corrupt
        with Peer(game) as peer, d.Client(peer.address, TOKEN, 3000) as client:
            with self.assertRaises(d.Rejected):
                self.start(client)
        self.assertEqual(game.calls.count('PrepareDesignation'), 1)
        self.assertNotIn('CommitDesignation', game.calls)

    def test_directory_lock_serializes_other_processes_and_releases_on_exit(self):
        code = '''from pathlib import Path
import sys
import dig_designation_client as d
import dig_designation_store as s
try:
    with s.open_store(d, Path(sys.argv[1])):
        pass
except BlockingIOError:
    sys.exit(78)
'''
        env = dict(os.environ, PYTHONPATH=str(Path(d.__file__).parent))
        with s.open_store(d, self.root):
            result = subprocess.run([sys.executable, '-c', code, str(self.root)], env=env, capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 78, result.stderr)
        result = subprocess.run([sys.executable, '-c', code, str(self.root)], env=env, capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_offline_pages_are_pinned_to_directory_registry_and_receipt_snapshot(self):
        for i in range(9):
            self.record(f'{i}.json', f'key-{i}')
        first = s.records_result(d, self.root)
        self.assertEqual(len(first['records']), 8)
        second = s.records_result(d, self.root, continuation=first['continuation'])
        self.assertEqual([r['name'] for r in second['records']], ['8.json'])
        self.assertIsNone(second['continuation'])
        self.assertEqual(first['unresolved_records'], 9)
        self.assertFalse(first['registry_present'])
        self.assertFalse((self.root / s.REGISTRY).exists())  # discovery is not adoption
        with d.capsule(self.root / '0.json') as owner:
            value = owner.intent; native = VECTORS['cancelled']
            raw = native[:117] + bytes.fromhex(value['prepare_token']) + native[133:204] + d.key_bytes(value['key'])
            d.terminal_receipt(owner, rehash(raw))
        with self.assertRaises(d.Rejected):
            s.records_result(d, self.root, continuation=first['continuation'])
        self.assertEqual(s.records_result(d, self.root)['unresolved_records'], 8)

    def test_cursor_shapes_and_cross_directory_reuse_fail_closed(self):
        for i in range(3):
            self.record(f'{i}.json', f'key-{i}')
        first = s.records_result(d, self.root, limit=1)
        token = json.loads(bytes.fromhex(first['continuation']))
        for change in ({'offset': True}, {'offset': -1}, {'offset': 3}, {'limit': True}, {'limit': 2},
                       {'head': '0' * 64}, {'extra': 1}):
            with self.assertRaises(d.Rejected):
                s.records_result(d, self.root, 1, d.canonical({**token, **change}).hex())
        with tempfile.TemporaryDirectory() as directory:
            other = Path(directory).resolve(); other.chmod(0o700)
            for path in self.root.iterdir():
                (other / path.name).write_bytes(path.read_bytes()); (other / path.name).chmod(0o600)
            with self.assertRaises(d.Rejected):
                s.records_result(d, other, 1, first['continuation'])
        for limit in (0, 9, True):
            with self.assertRaises(d.Rejected):
                s.records_result(d, self.root, limit)

    def test_foreign_entries_orphan_receipts_duplicates_and_full_store_refuse(self):
        extra = self.root / 'foreign'
        extra.mkdir(mode=0o700)
        with self.assertRaises((d.Rejected, OSError)):
            s.records_result(d, self.root)
        extra.rmdir()
        extra = self.root / ('.dfmcp-dig-terminal-' + '0' * 64 + '.json')
        extra.write_bytes(b'{}'); extra.chmod(0o600)
        with self.assertRaises(d.Rejected):
            s.records_result(d, self.root)
        extra.unlink()
        self.record()
        alias = self.root / 'alias.json'
        alias.write_bytes((self.root / 'old.json').read_bytes()); alias.chmod(0o600)
        with self.assertRaises(d.Rejected):
            s.records_result(d, self.root)
        alias.unlink()
        with patch.object(s, 'MAX_RECORDS', 1):
            with s.open_store(d, self.root, True) as store, self.assertRaises(d.Rejected):
                store.ready('new.json', 'new')
        with patch.object(s, 'MAX_ENTRIES', 1), self.assertRaises(d.Rejected):
            s.records_result(d, self.root)

    def test_registry_symlinks_hardlinks_and_nonprivate_modes_fail(self):
        self.record(registered=True)
        ledger = self.root / s.REGISTRY; raw = ledger.read_bytes()
        for mode in (0o400, 0o640, 0o666):
            ledger.chmod(mode)
            with self.assertRaises(d.Rejected):
                s.records_result(d, self.root)
        ledger.chmod(0o600)
        target = self.root / 'elsewhere'; ledger.rename(target)
        ledger.symlink_to(target)
        with self.assertRaises(OSError):
            s.records_result(d, self.root)
        ledger.unlink(); os.link(target, ledger)
        with self.assertRaises(d.Rejected):
            s.records_result(d, self.root)
        self.assertEqual(target.read_bytes(), raw)

    def test_all_registry_prefixes_and_chain_forgery_are_rejected(self):
        self.record(registered=True)
        with s.open_store(d, self.root) as store:
            raw = store.raw
            for size in range(len(s.HEADER)):
                with self.assertRaises((d.Rejected, ValueError)):
                    store.decode(raw[:size])
            for size in range(len(s.HEADER) + 1, len(raw)):
                with self.assertRaises((d.Rejected, ValueError)):
                    store.decode(raw[:size])
            entry = json.loads(raw.splitlines()[1])['entry']; entry['previous'] = '0' * 64
            bad = s.HEADER + d.canonical({'entry': entry, 'sha256': s.fingerprint(d.canonical(entry))}) + b'\n'
            with self.assertRaises(d.Rejected):
                store.decode(bad)

    def test_actual_records_cli_needs_no_environment_and_deadlines_bound_audit(self):
        self.record()
        command = [sys.executable, d.__file__, 'records', '--directory', str(self.root)]
        result = subprocess.run(command, env={}, capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)
        loaded = json.loads(result.stdout)
        self.assertEqual(loaded['native_calls'], 0); self.assertEqual(loaded['unresolved_records'], 1)
        self.assertFalse(loaded['retry_commit_permitted']); self.assertFalse(loaded['global_fence'])
        self.assertLess(len(result.stdout), d.MAX_OUTPUT)
        with patch.object(d, 'Client', side_effect=AssertionError('offline native access')):
            out = io.StringIO()
            with contextlib.redirect_stdout(out), patch.dict(os.environ, {}, clear=True):
                self.assertEqual(d.main(['records', '--directory', str(self.root)]), 0)
        with patch.object(s.time, 'monotonic', side_effect=[1, 2]), self.assertRaises(d.Rejected):
            s.records_result(d, self.root, timeout_ms=1)


if __name__ == '__main__':
    unittest.main(verbosity=2)
