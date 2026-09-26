"""Execute real POSIX custody, interruption recovery and the construction CLI.

Foreground native I/O uses the same explicit joined peer as transport tests.
These are process/filesystem/loopback tests, not physical power-loss or live DF.
"""
from __future__ import annotations

from dataclasses import replace
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import construction_monitor_store as s
import construction_monitor_rpc as rpc
import construction_receipt as c
import track_construction as cli
from build_placement_wire import Rejected, Record, canonical
from test_construction_receipt import RECEIPT, TICK, goal, sample, operations, item, building
from test_construction_monitor_rpc import NativePeer

ROOT = Path(__file__).resolve().parents[1]
ADDRESS = ('127.0.0.1', 5000)
RENDER = lambda value: cli.output(cli.packet('sample', value, storage_acknowledged=True))


class PrivateDirectory(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='dfmcp-construction-custody-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.root.chmod(0o700)
        self.path = str(self.root / 'bed.construction')
        self.receipt = self.root / 'placed.receipt'
        self.receipt.write_bytes(RECEIPT)
        self.receipt.chmod(0o600)

    def owner(self, writable=True, create=False):
        return s.Journal(self.path, rpc.Budget(10000), writable=writable,
                         create=(goal(), ADDRESS) if create else None)

    def saved(self, ready=False):
        with self.owner(create=True) as journal:
            journal.start_read()
            journal.accept(sample(), RENDER)
            if ready:
                journal.start_read()
                journal.accept(sample(tick=TICK + 2), RENDER)
        return Path(self.path).read_bytes()

    def command(self, arguments):
        stdout = io.StringIO()
        with contextlib.redirect_stdout(stdout):
            result = cli.main(arguments)
        return result, json.loads(stdout.getvalue())

    def start_args(self):
        return ['start', '--journal', self.path, '--receipt-file', str(self.receipt),
                '--deadline-tick', str(TICK + 100)]


class CustodyTests(PrivateDirectory):
    def test_goal_and_sample_publication_then_immutable_offline_replay(self):
        raw = self.saved(ready=True)
        state = s.replay(raw, rpc.Budget(10000))
        self.assertEqual((state.frames, state.progress.phase, state.progress.streak), (5, 'satisfied', 2))
        with self.owner(False) as journal:
            self.assertEqual(journal.state, state)
            self.assertFalse(journal.read_owned)
            self.assertEqual(journal.view()['journal_head'], raw[-32:].hex())
            self.assertFalse(journal.cancel(RENDER))
            with self.assertRaises(Rejected):
                journal.start_read()
            with self.assertRaises(Rejected):
                journal.accept(sample(tick=TICK + 3), RENDER)
        self.assertEqual(Path(self.path).read_bytes(), raw)

    def test_every_torn_prefix_and_corrupt_byte_is_refused(self):
        raw = self.saved()
        complete = set()
        at = len(s.MAGIC)
        while at < len(raw):
            size, _, _ = s.HEADER.unpack(raw[at:at + s.HEADER.size])
            at += s.HEADER.size + size + 32
            complete.add(at)
        for end in range(len(raw)):
            if end in complete:
                self.assertFalse(s.replay(raw[:end], rpc.Budget(1000)).progress.terminal)
            else:
                with self.subTest(prefix=end), self.assertRaises((ValueError, TypeError)):
                    s.replay(raw[:end], rpc.Budget(1000))
        for at in range(len(raw)):
            corrupted = raw[:at] + bytes([raw[at] ^ 128]) + raw[at + 1:]
            with self.subTest(corrupt=at), self.assertRaises((ValueError, TypeError)):
                s.replay(corrupted, rpc.Budget(1000))
        with self.assertRaises(Rejected):
            s.replay(raw + b'\0', rpc.Budget(1000))

    def test_rehashed_impossible_transitions_and_wrong_receipts(self):
        with self.owner(create=True) as journal:
            raw = Path(self.path).read_bytes()
            for kind, payload in (('sample', sample().encode()), ('goal', goal().encode()),
                                  ('cancel', b'extra'), ('read_started', b'extra')):
                forged = raw + s.frame(journal.state, kind, payload)
                with self.subTest(kind=kind), self.assertRaises(Rejected):
                    s.replay(forged, rpc.Budget(1000))
            journal.start_read()
            state = journal.state
            forged_sample = replace(sample(), after_record=RECEIPT[:-1] + b'\0')
            with self.assertRaises(Rejected):
                journal.accept(forged_sample, RENDER)
            self.assertTrue(journal.fenced)
            self.assertEqual(journal.state, state)
        with self.owner() as journal:
            self.assertTrue(journal.state.progress.reading)
            journal.cancel(RENDER)
        raw = Path(self.path).read_bytes()
        with self.assertRaises(Rejected):
            s.replay(raw + s.frame(journal.state, 'read_started', b''), rpc.Budget(1000))

    def test_reopened_read_cannot_publish_and_interrupt_resets_streak(self):
        with self.owner(create=True) as journal:
            journal.start_read()
            journal.accept(sample(), RENDER)
            journal.start_read()  # The process ends without a native sample.
        with self.owner() as journal:
            self.assertEqual(journal.state.progress.streak, 1)
            self.assertTrue(journal.state.progress.reading)
            with self.assertRaises(Rejected):
                journal.accept(sample(tick=TICK + 2), RENDER)
            journal.start_read()
            self.assertEqual(journal.state.progress.streak, 0)
            self.assertEqual(journal.state.progress.interruptions, 1)
            journal.accept(sample(tick=TICK + 2), RENDER)
            self.assertEqual(journal.state.progress.streak, 1)
            self.assertFalse(journal.state.progress.terminal)
            journal.start_read()
            journal.accept(sample(tick=TICK + 3), RENDER)
            self.assertEqual(journal.state.progress.phase, 'satisfied')

    def test_failed_result_reservation_never_publishes_sample(self):
        with self.owner(create=True) as journal:
            journal.start_read()
            raw = Path(self.path).read_bytes()
            def too_large(candidate):
                raise Rejected('output reservation failed')
            with self.assertRaises(Rejected):
                journal.accept(sample(), too_large)
            self.assertTrue(journal.fenced)
            self.assertEqual(Path(self.path).read_bytes(), raw)
        with self.owner(False) as journal:
            self.assertTrue(journal.state.progress.reading)
            self.assertEqual(journal.state.progress.observations, 0)

    def test_file_parent_sync_failure_preserves_uncertainty(self):
        for fail_number in (1, 2):
            self.path = str(self.root / f'fail-{fail_number}.construction')
            with self.owner(create=True) as journal:
                actual, calls = os.fsync, 0
                def fail(fd):
                    nonlocal calls
                    calls += 1
                    if calls == fail_number:
                        raise OSError('injected synchronization failure')
                    return actual(fd)
                with patch.object(s.os, 'fsync', side_effect=fail), self.assertRaises(OSError):
                    journal.start_read()
                self.assertTrue(journal.fenced)
                self.assertFalse(journal.read_owned)
            # Fully visible uncertain bytes are retained, not rewritten or
            # represented as having been acknowledged by the failed call.
            with self.owner(False) as journal:
                self.assertTrue(journal.state.progress.reading)
                self.assertEqual(journal.state.progress.observations, 0)

    def test_terminal_sync_failure_reopens_historical_evidence_without_another_read(self):
        with self.owner(create=True) as journal:
            journal.start_read()
            journal.accept(sample(), RENDER)
            journal.start_read()
            with patch.object(s.os, 'fsync', side_effect=OSError('receipt sync failure')), self.assertRaises(OSError):
                journal.accept(sample(tick=TICK + 2), RENDER)
            self.assertTrue(journal.fenced)
            self.assertNotEqual(journal.state.progress.phase, 'satisfied')
        with self.owner(False) as journal:
            self.assertEqual(journal.state.progress.phase, 'satisfied')
            self.assertFalse(cli.packet('inspect', journal.state)['result']['storage_acknowledged_this_call'])

    def test_partial_writes_retained_and_short_writes_finished(self):
        actual = os.write
        with patch.object(s.os, 'write', side_effect=lambda fd, raw: actual(fd, raw[:13])):
            with self.owner(create=True) as journal:
                journal.start_read()
                journal.accept(sample(), RENDER)
        with self.owner(False) as journal:
            self.assertEqual(journal.state.progress.observations, 1)
        self.path = str(self.root / 'partial.construction')
        calls = 0
        def fail(fd, raw):
            nonlocal calls
            calls += 1
            if calls == 1:
                return actual(fd, raw[:23])
            raise OSError('injected short write')
        with patch.object(s.os, 'write', side_effect=fail), self.assertRaises(OSError):
            self.owner(create=True)
        self.assertEqual(Path(self.path).stat().st_size, 23)
        with self.assertRaises(Rejected):
            self.owner()
        with self.assertRaises(FileExistsError):
            self.owner(create=True)
        self.assertEqual(Path(self.path).stat().st_size, 23)

    def test_modes_symlinks_hardlinks_special_files_and_directory_replacement(self):
        self.saved()
        path = Path(self.path)
        for mode in (0o400, 0o640, 0o660):
            path.chmod(mode)
            with self.assertRaises(Rejected):
                self.owner()
        path.chmod(0o600)
        link = self.root / 'hardlink'
        os.link(path, link)
        with self.assertRaises(Rejected):
            self.owner()
        link.unlink()
        alias = self.root / 'alias'
        alias.symlink_to(path)
        with self.assertRaises((OSError, Rejected)):
            s.Journal(str(alias), rpc.Budget(1000))
        fifo = self.root / 'fifo'
        os.mkfifo(fifo, 0o600)
        with self.assertRaises((OSError, Rejected)):
            s.Journal(str(fifo), rpc.Budget(1000))
        self.root.chmod(0o500)
        with self.assertRaises(Rejected):
            self.owner()
        self.root.chmod(0o700)
        directory = self.root / 'private'
        directory.mkdir(mode=0o700)
        self.path = str(directory / 'monitor')
        with self.owner(create=True) as journal:
            directory.rename(self.root / 'renamed')
            directory.mkdir(mode=0o700)
            with self.assertRaises(Rejected):
                journal.check()
            self.assertTrue(journal.fenced)

    def test_same_size_substitution_and_named_file_replacement_fence_owner(self):
        self.saved()
        with self.owner() as journal:
            path = Path(self.path)
            raw = bytearray(path.read_bytes())
            raw[100] ^= 1
            path.write_bytes(raw)
            with self.assertRaises(Rejected):
                journal.check()
        self.path = str(self.root / 'replacement')
        with self.owner(create=True) as journal:
            path = Path(self.path)
            raw = path.read_bytes()
            path.rename(self.root / 'old')
            path.write_bytes(raw)
            path.chmod(0o600)
            with self.assertRaises(Rejected):
                journal.check()

    def test_real_subprocess_lock_and_no_write_offline_inspection(self):
        self.saved(ready=True)
        command = [sys.executable, str(ROOT / 'scripts/track_construction.py'), 'inspect', '--journal', self.path]
        with self.owner() as journal:
            blocked = subprocess.run(command, capture_output=True, timeout=5, env={})
            self.assertEqual(blocked.returncode, 2)
            self.assertFalse(json.loads(blocked.stdout)['ok'])
            journal.check()
        raw = Path(self.path).read_bytes()
        completed = subprocess.run(command, capture_output=True, timeout=5, env={})
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(json.loads(completed.stdout)['result']['progress']['phase'], 'satisfied')
        self.assertEqual(Path(self.path).read_bytes(), raw)
        with self.owner(False) as journal, patch.object(s.os, 'fsync', side_effect=AssertionError('offline write')):
            journal.view()
            self.assertFalse(journal.cancel(RENDER))

    def test_capacity_and_budget_refuse_before_read_intent(self):
        with self.owner(create=True) as journal:
            original = Path(self.path).read_bytes()
            with patch.object(s, 'MAX_FILE', journal.length + 1024), self.assertRaises(Rejected):
                journal.start_read()
            self.assertEqual(Path(self.path).read_bytes(), original)
            journal.budget.deadline = 0
            with self.assertRaises(Rejected):
                journal.start_read()
            self.assertEqual(Path(self.path).read_bytes(), original)


class WorkflowTests(PrivateDirectory):
    def test_actual_cli_start_and_new_process_sample_then_offline_terminal(self):
        with NativePeer([operations(), operations(tick=TICK + 2)]) as peer, peer.environment():
            code, first = self.command(self.start_args())
            self.assertEqual(code, 0, first)
            self.assertEqual(first['result']['progress']['phase'], 'candidate')
            self.assertTrue(first['result']['storage_acknowledged_this_call'])
            command = [sys.executable, str(ROOT / 'scripts/track_construction.py'), 'sample', '--journal', self.path]
            second = subprocess.run(command, capture_output=True, timeout=5, env=dict(os.environ))
            self.assertEqual(second.returncode, 0, second.stderr)
            self.assertEqual(json.loads(second.stdout)['result']['progress']['phase'], 'satisfied')
            self.assertEqual(peer.connections, 2)
            self.assertEqual(peer.reads, 2)
        original = Path(self.path).read_bytes()
        with patch.dict(os.environ, {}, clear=True), patch.object(rpc.socket, 'socket', side_effect=AssertionError('terminal contacted native')):
            for operation in ('inspect', 'sample', 'cancel'):
                code, value = self.command([operation, '--journal', self.path])
                self.assertEqual(code, 0, value)
                self.assertEqual(value['result']['progress']['phase'], 'satisfied')
                self.assertFalse(value['result']['native_contacted'])
                self.assertFalse(value['result']['storage_acknowledged_this_call'])
        self.assertEqual(Path(self.path).read_bytes(), original)
        self.assertEqual(self.receipt.read_bytes(), RECEIPT)

    def test_socket_not_opened_until_read_intent_and_both_syncs(self):
        actual, calls = os.fsync, []
        def synced(fd):
            calls.append(fd)
            return actual(fd)
        original = rpc.socket.socket
        def open_socket(*args, **kwargs):
            self.assertEqual(len(calls), 4)  # goal file/dir, read-start file/dir
            state = s.replay(Path(self.path).read_bytes(), rpc.Budget(1000))
            self.assertTrue(state.progress.reading)
            self.assertEqual(state.progress.observations, 0)
            return original(*args, **kwargs)
        with NativePeer() as peer, peer.environment():
            with patch.object(s.os, 'fsync', side_effect=synced), patch.object(rpc.socket, 'socket', side_effect=open_socket):
                code, value = self.command(self.start_args())
            self.assertEqual(code, 0, value)
            self.assertEqual(len(calls), 6)

    def test_failed_pre_read_sync_never_connects_and_preserves_journal(self):
        environment = {rpc.OPT_IN: '1', rpc.BUILD_TOKEN: 'b' * 32, rpc.OPERATIONS_TOKEN: 'o' * 32}
        actual, calls = os.fsync, 0
        def fail(fd):
            nonlocal calls
            calls += 1
            if calls == 3:
                raise OSError('read-start sync failure')
            return actual(fd)
        with patch.dict(os.environ, environment, clear=True), patch.object(s.os, 'fsync', side_effect=fail), \
                patch.object(rpc.socket, 'socket', side_effect=AssertionError('premature socket')):
            code, value = self.command(self.start_args())
        self.assertEqual(code, 2)
        self.assertFalse(value['result']['native_contacted'])
        self.assertFalse(value['result']['custody_verified'])
        self.assertEqual(value['agent_turn']['active_work'][0]['placement_key'], 'golden')
        with self.owner(False) as journal:
            self.assertTrue(journal.state.progress.reading)

    def test_lost_trailing_receipt_recovery_resets_prior_streak(self):
        with NativePeer() as peer, peer.environment():
            code, value = self.command(self.start_args())
            self.assertEqual(code, 0, value)
            # Keep the original endpoint constant as a restarted native peer
            # would; easier here to switch the loss hook between connections.
        # A retained goal forbids endpoint substitution. Use one peer for all
        # four foreground calls in the full interruption case below.
        self.path = str(self.root / 'interruption')
        def hook(tag, fields, reply, peer):
            peer.lose = 'after_receipt' if peer.connections == 2 else None
            return reply
        with NativePeer([operations(), operations(tick=TICK + 2), operations(tick=TICK + 3), operations(tick=TICK + 4)], hook=hook) as peer, peer.environment():
            results = [self.command(self.start_args())]
            for _ in range(3):
                results.append(self.command(['sample', '--journal', self.path]))
            self.assertEqual([r[0] for r in results], [0, 2, 0, 0], results)
            self.assertEqual(results[2][1]['result']['progress']['streak'], 1)
            self.assertEqual(results[2][1]['result']['progress']['interrupted_reads'], 1)
            self.assertEqual(results[3][1]['result']['progress']['phase'], 'satisfied')
            self.assertEqual(results[3][1]['result']['goal']['deadline_tick'], TICK + 100)

    def test_missing_original_receipt_never_becomes_completion(self):
        with NativePeer(receipt=None) as peer, peer.environment():
            code, value = self.command(self.start_args())
            self.assertEqual(code, 2)
            self.assertEqual(peer.reads, 0)
        with self.owner(False) as journal:
            self.assertTrue(journal.state.progress.reading)
            self.assertEqual(journal.state.progress.observations, 0)
            self.assertFalse(journal.state.progress.terminal)
        with patch.dict(os.environ, {}, clear=True):
            code, value = self.command(['cancel', '--journal', self.path])
        self.assertEqual(code, 0, value)
        self.assertEqual(value['result']['progress']['phase'], 'cancelled')
        self.assertFalse(value['result']['placement_effect_discharged'])
        self.assertEqual(self.receipt.read_bytes(), RECEIPT)

    def test_retained_endpoint_policy_and_input_receipt_cannot_be_changed(self):
        self.saved()
        with patch.dict(os.environ, {rpc.OPT_IN: '1', rpc.BUILD_TOKEN: 'b' * 32, rpc.OPERATIONS_TOKEN: 'o' * 32,
                                    rpc.ENDPOINT: '127.0.0.1:5001'}, clear=True), \
                patch.object(rpc.socket, 'socket', side_effect=AssertionError('retargeted native')):
            original = Path(self.path).read_bytes()
            for extra in ([], ['--deadline-tick', str(TICK + 999)], ['--stable-samples', '2'],
                          ['--receipt-file', str(self.receipt)], ['--unknown-option', 'secret']):
                code, value = self.command(['sample', '--journal', self.path, *extra])
                self.assertEqual(code, 2)
                self.assertNotIn('secret', json.dumps(value))
            self.assertEqual(Path(self.path).read_bytes(), original)

    def test_strict_private_receipt_import_and_existing_goal_not_overwritten(self):
        self.receipt.write_bytes(canonical({'canonical_record_hex': RECEIPT.hex()}))
        self.assertEqual(cli.load_receipt(str(self.receipt), rpc.Budget(1000)), RECEIPT)
        for content in (b'{"canonical_record_hex":"00","canonical_record_hex":"00"}',
                        canonical({'canonical_record_hex': RECEIPT.hex(), 'authority': True}), b'bad receipt'):
            self.receipt.write_bytes(content)
            with self.assertRaises((ValueError, TypeError)):
                cli.load_receipt(str(self.receipt), rpc.Budget(1000))
        self.receipt.write_bytes(RECEIPT)
        self.receipt.chmod(0o400)
        with self.assertRaises(Rejected):
            cli.load_receipt(str(self.receipt), rpc.Budget(1000))
        self.receipt.chmod(0o600)
        raw = self.saved()
        with patch.dict(os.environ, {rpc.OPT_IN: '1', rpc.BUILD_TOKEN: 'b' * 32, rpc.OPERATIONS_TOKEN: 'o' * 32}, clear=True), \
                patch.object(rpc.socket, 'socket', side_effect=AssertionError('existing start reached native')):
            code, value = self.command(self.start_args())
        self.assertEqual(code, 2)
        self.assertEqual(Path(self.path).read_bytes(), raw)

    def test_actual_exported_goal_for_bed_chair_and_table_and_maximal_output(self):
        for kind, building_name in ((1, 'Bed'), (2, 'Chair'), (3, 'Table')):
            original = Record.decode(RECEIPT)
            before = replace(original.plan.before, selection=replace(original.plan.before.selection, kind=kind),
                             item=replace(original.plan.before.item, kind=kind, native_type=100 + kind))
            plan = replace(original.plan, key='k' * 128, before=before)
            insertion = replace(original.insertion, kind=kind)
            receipt = replace(original, plan=plan, after=before.expected_after(), insertion=insertion)
            g = c.Goal(receipt.raw, TICK + 100)
            payload = operations(buildings=(building(kind=building_name, native_type=kind),),
                                 items=(item(kind=building_name.upper(), native_type=100 + kind),))
            linked = replace(sample(raw=payload), before_record=receipt.raw, after_record=receipt.raw)
            state = c.advance(c.begin_read(c.Progress(g.digest)), g, linked, lambda: None)
            self.assertEqual(state.phase, 'candidate')
            packet = cli.packet('sample', s.State(g, ADDRESS, state), native_contacted=True, storage_acknowledged=True)
            self.assertLess(len(cli.output(packet)) + 512, cli.MAX_OUTPUT)
            self.assertIsNone(packet['agent_turn']['anchor'])
            self.assertEqual(packet['agent_turn']['active_work'][0]['placement_key'], 'k' * 128)


if __name__ == '__main__':
    unittest.main(verbosity=2)
