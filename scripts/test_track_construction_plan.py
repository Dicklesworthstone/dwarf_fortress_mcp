"""Exercise the complete furnishing-plan CLI through files, processes and TCP.

The joined native peer implements the declared test wire protocol; these checks
do not establish live-game, native-plugin or physical power-loss qualification.
Beads: df-dfhack-bridge-plane-c-pic.4 / df-dfhack-bridge-plane-c-pic.5.
"""
from __future__ import annotations

from dataclasses import replace
import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import construction_monitor_rpc as native
import construction_monitor_store as custody
import construction_plan as core
import construction_plan_rpc as rpc
import construction_plan_store as store
import track_construction_plan as cli
from build_placement_wire import Rejected, Record, canonical
from test_construction_plan import make_goal, make_receipt, operations_for, sample_for, TICK
from test_construction_plan_rpc import NativePeer
from test_construction_receipt import VECTORS

ROOT = Path(__file__).resolve().parents[1]
ADDRESS = ('127.0.0.1', 5000)
ENVIRONMENT = {native.OPT_IN: '1', native.BUILD_TOKEN: 'b' * 32,
               native.OPERATIONS_TOKEN: 'o' * 32}


def bundle(receipts):
    return {'schema': 'dfmcp.construction-plan-receipts/1',
            'receipts': [{'canonical_record_hex': raw.hex()} for raw in receipts]}


def render(state):
    return cli.output(cli.packet('sample', state, storage_acknowledged=True))


class PrivatePlan(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='dfmcp-construction-plan-cli-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.root.chmod(0o700)
        self.path = str(self.root / 'furnishing.construction-plan')
        self.receipts_file = self.root / 'original-placements.json'
        self.goal = make_goal()
        self.write_bundle(self.goal.receipts)
        self.last_stdout = b''

    def write_bundle(self, receipts):
        self.receipts_file.write_bytes(canonical(bundle(receipts)))
        self.receipts_file.chmod(0o600)

    def owner(self, writable=False, create=False):
        return store.Journal(self.path, rpc.Budget(10000), writable=writable,
                             create=(self.goal, ADDRESS) if create else None)

    def saved(self, *, terminal=False):
        with self.owner(writable=True, create=True) as journal:
            journal.start_read()
            journal.accept(sample_for(self.goal), render)
            if terminal:
                journal.start_read()
                journal.accept(sample_for(self.goal, tick=TICK + 2), render)
        return Path(self.path).read_bytes()

    def start_args(self):
        return ['start', '--journal', self.path, '--receipts-file', str(self.receipts_file),
                '--deadline-tick', str(self.goal.deadline)]

    def command(self, arguments, *, process=False):
        if process:
            result = subprocess.run([sys.executable, str(ROOT / 'scripts/track_construction_plan.py'),
                                     *arguments], capture_output=True, timeout=10, env=dict(os.environ))
            self.assertEqual(result.stderr, b'', result.stderr)
            self.last_stdout = result.stdout
            code = result.returncode
        else:
            stdout, stderr = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                code = cli.main(arguments)
            self.assertEqual(stderr.getvalue(), '')
            self.last_stdout = stdout.getvalue().encode('ascii')
        self.assertLessEqual(len(self.last_stdout), 65536)
        self.assertEqual(self.last_stdout.count(b'\n'), 1)
        value = json.loads(self.last_stdout)
        self.assertEqual(value['schema'], 'dfmcp.construction-plan-monitor-result/1')
        self.assertEqual(value['ok'], code == 0)
        self.assertIsNone(value['agent_turn']['anchor'])
        self.assertFalse(value['agent_turn']['briefing']['runtime_admitted'])
        self.assertFalse(value['agent_turn']['briefing']['mutation_admissible'])
        self.assertFalse(value['result']['placement_effects_discharged'])
        self.assertFalse(value['result']['retry_placement_permitted'])
        self.assertFalse(value['result']['current_usability_proven'])
        self.assertNotIn(b'b' * 32, self.last_stdout)
        self.assertNotIn(b'o' * 32, self.last_stdout)
        return code, value


class PlanWorkflowTests(PrivatePlan):
    def test_mixed_completion_resets_the_whole_streak_across_processes(self):
        # Each member succeeds at some point, but that is not a joint result.
        captures = [operations_for(self.goal),
                    operations_for(self.goal, tick=TICK + 2, statuses={0: 'item_unverified'}),
                    operations_for(self.goal, tick=TICK + 3),
                    operations_for(self.goal, tick=TICK + 4, statuses={1: 'item_unverified'}),
                    operations_for(self.goal, tick=TICK + 5),
                    operations_for(self.goal, tick=TICK + 6)]
        original = self.receipts_file.read_bytes()
        with NativePeer(self.goal.receipts, captures) as peer, peer.environment():
            results = [self.command(self.start_args())]
            results.extend(self.command(['sample', '--journal', self.path], process=True)
                           for _ in captures[1:])
            self.assertEqual([code for code, _ in results], [0] * 6, results)
            progress = [value['result']['progress'] for _, value in results]
            self.assertEqual([p['streak'] for p in progress], [1, 0, 1, 0, 1, 2])
            self.assertTrue(all(p['phase'] != 'satisfied' for p in progress[:-1]))
            self.assertEqual(progress[-1]['phase'], 'satisfied')
            for _, value in results:
                self.assertEqual(value['result']['identity']['goal_digest'], self.goal.digest)
                self.assertEqual(value['result']['identity']['target_count'], 2)
                self.assertEqual(len(value['result']['targets']), 2)
                self.assertTrue(value['result']['native_contacted'])
                self.assertTrue(value['result']['storage_acknowledged_this_call'])
                self.assertEqual(value['result']['goal']['deadline_tick'], self.goal.deadline)
            self.assertEqual((peer.connections, peer.reads, peer.releases, peer.queries), (6, 6, 6, 24))
        with self.owner() as journal:
            self.assertEqual(journal.state.progress.observations, 6)
            self.assertEqual(journal.state.progress.phase, 'satisfied')
        self.assertEqual(self.receipts_file.read_bytes(), original)

    def test_lost_last_receipt_retains_unknown_read_then_restarts_joint_stability(self):
        final_key = self.goal.goals[-1].record.plan.key.encode('ascii')
        def hook(tag, fields, reply, peer):
            peer.lose = ('after_receipt' if peer.connections == 2
                         and tag == 'after_receipt' and fields[10] == final_key else None)
            return reply
        captures = [operations_for(self.goal, tick=TICK + offset) for offset in range(1, 5)]
        with NativePeer(self.goal.receipts, captures, hook=hook) as peer, peer.environment():
            code, value = self.command(self.start_args())
            self.assertEqual(code, 0, value)
            code, refused = self.command(['sample', '--journal', self.path], process=True)
            self.assertEqual(code, 2, refused)
            self.assertIsNone(refused['result']['progress'])
            self.assertFalse(refused['result']['custody_verified'])
            self.assertTrue(refused['result']['native_connection_attempted'])
            self.assertTrue(refused['agent_turn']['active_work'][0]['read_outcome_unknown'])
            with self.owner() as journal:
                self.assertTrue(journal.state.progress.reading)
                self.assertEqual(journal.state.progress.observations, 1)
            code, resumed = self.command(['sample', '--journal', self.path], process=True)
            self.assertEqual(code, 0, resumed)
            self.assertEqual(resumed['result']['progress']['streak'], 1)
            self.assertEqual(resumed['result']['progress']['interrupted_reads'], 1)
            self.assertNotEqual(resumed['result']['progress']['phase'], 'satisfied')
            code, finished = self.command(['sample', '--journal', self.path], process=True)
            self.assertEqual(code, 0, finished)
            self.assertEqual(finished['result']['progress']['phase'], 'satisfied')
            self.assertEqual(finished['result']['goal']['deadline_tick'], self.goal.deadline)
            self.assertEqual((peer.connections, peer.reads, peer.queries), (4, 4, 16))

    def test_goal_and_read_intent_file_and_parent_sync_before_first_socket(self):
        actual_sync, synced = os.fsync, []
        def sync(fd):
            synced.append(fd)
            return actual_sync(fd)
        actual_socket = native.socket.socket
        def connect(*args, **kwargs):
            self.assertEqual(len(synced), 4)
            state = store.replay(Path(self.path).read_bytes(), rpc.Budget(1000))
            self.assertTrue(state.progress.reading)
            self.assertEqual(state.progress.observations, 0)
            self.assertEqual(state.goal.receipts, self.goal.receipts)
            return actual_socket(*args, **kwargs)
        with NativePeer(self.goal.receipts) as peer, peer.environment():
            with patch.object(custody.os, 'fsync', side_effect=sync), \
                    patch.object(native.socket, 'socket', side_effect=connect):
                code, value = self.command(self.start_args())
            self.assertEqual(code, 0, value)
            self.assertEqual(len(synced), 6)
            self.assertEqual(peer.connections, 1)

    def test_any_pre_read_sync_failure_prevents_native_contact(self):
        actual = os.fsync
        for failure in range(1, 5):
            self.path = str(self.root / f'sync-{failure}.construction-plan')
            calls = 0
            def sync(fd):
                nonlocal calls
                calls += 1
                if calls == failure:
                    raise OSError('injected durable intent synchronization failure')
                return actual(fd)
            with self.subTest(failure=failure), patch.dict(os.environ, ENVIRONMENT, clear=True), \
                    patch.object(custody.os, 'fsync', side_effect=sync), \
                    patch.object(native.socket, 'socket', side_effect=AssertionError('socket before durable intent')):
                code, value = self.command(self.start_args())
            self.assertEqual(code, 2)
            self.assertFalse(value['result']['native_connection_attempted'])
            self.assertFalse(value['result']['storage_acknowledged_this_call'])
            self.assertFalse(value['result']['custody_verified'])
            # Visible complete bytes remain available without claiming that
            # the failed caller acknowledged their durability.
            with self.owner() as journal:
                self.assertEqual(journal.state.goal.receipts, self.goal.receipts)
                self.assertEqual(journal.state.progress.observations, 0)
                self.assertEqual(journal.state.progress.reading, failure > 2)

    def test_terminal_inspect_sample_and_cancel_are_offline_and_immutable(self):
        original = self.saved(terminal=True)
        with patch.dict(os.environ, {}, clear=True), \
                patch.object(native.socket, 'socket', side_effect=AssertionError('terminal native read')), \
                patch.object(custody.os, 'fsync', side_effect=AssertionError('terminal journal write')):
            for operation in ('inspect', 'sample', 'cancel', 'sample'):
                code, value = self.command([operation, '--journal', self.path])
                self.assertEqual(code, 0, value)
                self.assertEqual(value['result']['progress']['phase'], 'satisfied')
                self.assertFalse(value['result']['native_contacted'])
                self.assertFalse(value['result']['storage_acknowledged_this_call'])
                self.assertTrue(value['result']['custody_verified'])
                self.assertEqual(value['agent_turn']['active_work'], [])
        self.assertEqual(Path(self.path).read_bytes(), original)

    def test_missing_one_original_receipt_cannot_publish_and_offline_cancel_keeps_effects(self):
        final_key = self.goal.goals[-1].record.plan.key.encode('ascii')
        def missing(tag, fields, reply, peer):
            if tag == 'before_receipt' and fields[10] == final_key:
                reply.pop(10, None)
            return reply
        original = self.receipts_file.read_bytes()
        with NativePeer(self.goal.receipts, hook=missing) as peer, peer.environment():
            code, value = self.command(self.start_args())
            self.assertEqual(code, 2, value)
            self.assertEqual((peer.queries, peer.reads, peer.releases), (2, 0, 0))
        with self.owner() as journal:
            self.assertTrue(journal.state.progress.reading)
            self.assertEqual(journal.state.progress.observations, 0)
            self.assertFalse(journal.state.progress.terminal)
        with patch.dict(os.environ, {}, clear=True), \
                patch.object(native.socket, 'socket', side_effect=AssertionError('cancel touched native')):
            code, value = self.command(['cancel', '--journal', self.path])
            self.assertEqual(code, 0, value)
            self.assertEqual(value['result']['progress']['phase'], 'cancelled')
            self.assertTrue(value['result']['storage_acknowledged_this_call'])
            retained = Path(self.path).read_bytes()
            for operation in ('cancel', 'sample', 'inspect'):
                code, value = self.command([operation, '--journal', self.path])
                self.assertEqual(code, 0, value)
                self.assertEqual(value['result']['progress']['phase'], 'cancelled')
                self.assertFalse(value['result']['storage_acknowledged_this_call'])
        self.assertEqual(Path(self.path).read_bytes(), retained)
        self.assertEqual(self.receipts_file.read_bytes(), original)

    def test_retained_endpoint_and_policy_cannot_be_substituted(self):
        original = self.saved()
        environment = {**ENVIRONMENT, native.ENDPOINT: '127.0.0.1:5001'}
        extras = [[], ['--deadline-tick', str(TICK + 999)], ['--interval-ticks', '2'],
                  ['--stable-samples', '3'], ['--stable-span-ticks', '9'],
                  ['--max-gap-ticks', '20'], ['--max-observations', '42'],
                  ['--receipts-file', str(self.receipts_file)], ['--unknown', 'do-not-echo-secret']]
        with patch.dict(os.environ, environment, clear=True), \
                patch.object(native.socket, 'socket', side_effect=AssertionError('retargeted native access')):
            for extra in extras:
                with self.subTest(extra=extra):
                    code, value = self.command(['sample', '--journal', self.path, *extra])
                    self.assertEqual(code, 2, value)
                    self.assertNotIn(b'do-not-echo-secret', self.last_stdout)
                    self.assertFalse(value['result']['native_connection_attempted'])
                    self.assertEqual(Path(self.path).read_bytes(), original)

    def test_changed_import_file_does_not_change_retained_selection(self):
        captures = [operations_for(self.goal), operations_for(self.goal, tick=TICK + 2)]
        with NativePeer(self.goal.receipts, captures) as peer, peer.environment():
            code, first = self.command(self.start_args())
            self.assertEqual(code, 0, first)
            self.write_bundle((make_receipt(9),))
            original = Path(self.path).read_bytes()
            with patch.object(native.socket, 'socket', side_effect=AssertionError('selection replacement contacted native')):
                code, refused = self.command(self.start_args())
                self.assertEqual(code, 2, refused)
                self.assertEqual(Path(self.path).read_bytes(), original)
                code, refused = self.command(['sample', '--journal', self.path,
                                               '--receipts-file', str(self.receipts_file)])
                self.assertEqual(code, 2, refused)
                self.assertEqual(Path(self.path).read_bytes(), original)
            code, value = self.command(['sample', '--journal', self.path], process=True)
            self.assertEqual(code, 0, value)
            self.assertEqual(value['result']['progress']['phase'], 'satisfied')
            self.assertEqual(value['result']['identity']['goal_digest'], self.goal.digest)
            self.assertEqual(value['result']['targets'], first['result']['targets'])
            self.assertEqual(peer.queries, 8)

    def test_result_reservation_failure_retains_unknown_read_without_sample(self):
        actual = cli.output
        def fail_success(value):
            progress = value['result']['progress']
            if value['ok'] and progress is not None and progress['observations']:
                raise Rejected('injected complete response reservation failure')
            return actual(value)
        with NativePeer(self.goal.receipts) as peer, peer.environment(), \
                patch.object(cli, 'output', side_effect=fail_success):
            code, value = self.command(self.start_args())
            self.assertEqual(code, 2, value)
            self.assertEqual((peer.reads, peer.queries), (1, 4))
            self.assertFalse(value['result']['storage_acknowledged_this_call'])
            self.assertFalse(value['result']['custody_verified'])
        with self.owner() as journal:
            self.assertTrue(journal.state.progress.reading)
            self.assertEqual(journal.state.progress.observations, 0)

    def test_complete_32_target_response_and_single_capture_with_maximum_keys(self):
        receipts = []
        for index in range(32):
            record = Record.decode(make_receipt(index, kind=index % 3 + 1))
            record = replace(record, plan=replace(record.plan, key='k' * 125 + f'{index:03d}'))
            receipts.append(record.raw)
        self.goal = core.Goal(tuple(receipts), TICK + 100)
        self.write_bundle(tuple(reversed(self.goal.receipts)))
        with NativePeer(self.goal.receipts) as peer, peer.environment():
            code, value = self.command(self.start_args(), process=True)
            self.assertEqual(code, 0, value)
            self.assertEqual(value['result']['identity']['target_count'], 32)
            rows = value['result']['targets']
            self.assertEqual(len(rows), 32)
            self.assertEqual([row['placement_key'] for row in rows],
                             [child.record.plan.key for child in self.goal.goals])
            self.assertEqual({row['kind'] for row in rows}, {'bed', 'chair', 'table'})
            self.assertEqual(len({row['building_id'] for row in rows}), 32)
            self.assertEqual(len({row['item_id'] for row in rows}), 32)
            self.assertEqual(value['result']['progress']['streak'], 1)
            self.assertEqual(value['agent_turn']['coverage']['selected_receipt_count'], 32)
            self.assertTrue(value['agent_turn']['coverage']['selection_complete'])
            self.assertEqual(value['agent_turn']['budget']['output_bytes_limit'], 65536)
            self.assertEqual((peer.connections, peer.reads, peer.releases, peer.queries), (1, 1, 1, 64))
        with self.owner() as journal:
            self.assertEqual(journal.state.goal.receipts, self.goal.receipts)
            self.assertEqual(journal.state.progress.observations, 1)
            reserved = cli.reserve('start', journal.state, native_contacted=True,
                                   storage_acknowledged=True)
            self.assertLessEqual(len(reserved), cli.MAX_OUTPUT)
            self.assertGreaterEqual(len(reserved), len(self.last_stdout))
            envelope = json.loads(reserved)
            self.assertEqual(len(envelope['result']['targets']), 32)
            self.assertEqual(envelope['result']['journal'],
                             {'frames': store.MAX_FRAMES, 'bytes': store.MAX_FILE, 'head': 'f' * 64})


class PlanImportTests(PrivatePlan):
    def test_strict_complete_bundle_schema_rows_and_canonical_original_receipts(self):
        self.write_bundle(tuple(reversed(self.goal.receipts)))
        receipts = cli.load_receipts(str(self.receipts_file), rpc.Budget(1000))
        self.assertEqual(receipts, tuple(reversed(self.goal.receipts)))
        self.assertEqual(core.Goal(receipts, self.goal.deadline), self.goal)
        row = bundle(self.goal.receipts)['receipts'][0]
        schema = cli.RECEIPT_SCHEMA
        invalid = [b'not-json', b'\xff', canonical({}), canonical({'schema': schema}),
                   canonical({'schema': schema, 'receipts': []}),
                   canonical({'schema': schema, 'receipts': [row] * 33}),
                   canonical({'schema': schema, 'receipts': True}),
                   canonical({'schema': 'unknown/1', 'receipts': [row]}),
                   canonical({**bundle(self.goal.receipts), 'authority': True}),
                   canonical({'schema': schema, 'receipts': [None]}),
                   canonical({'schema': schema, 'receipts': [{}]}),
                   canonical({'schema': schema, 'receipts': [{**row, 'status': 'placed'}]}),
                   canonical({'schema': schema, 'receipts': [{'canonical_record_hex': True}]}),
                   canonical({'schema': schema, 'receipts': [{'canonical_record_hex': row['canonical_record_hex'].upper()}]}),
                   canonical({'schema': schema, 'receipts': [{'canonical_record_hex': row['canonical_record_hex'][:-2]}]}),
                   canonical({'schema': schema, 'receipts': [{'canonical_record_hex': VECTORS['prepared']}]}),
                   b'{"schema":"' + schema.encode() + b'","schema":"' + schema.encode() + b'","receipts":[]}',
                   b'{"schema":"' + schema.encode() + b'","receipts":[{"canonical_record_hex":"00","canonical_record_hex":"00"}]}']
        for content in invalid:
            with self.subTest(content=content[:100]):
                self.receipts_file.write_bytes(content)
                with self.assertRaises((ValueError, TypeError)):
                    cli.load_receipts(str(self.receipts_file), rpc.Budget(1000))

    def test_duplicate_or_conflicting_selection_is_refused_before_journal_or_socket(self):
        first = Record.decode(self.goal.receipts[0])
        alias = replace(first, plan=replace(first.plan, key='different-key')).raw
        for index, receipts in enumerate(((self.goal.receipts[0],) * 2,
                                          (self.goal.receipts[0], alias))):
            self.path = str(self.root / f'refused-{index}.construction-plan')
            self.write_bundle(receipts)
            with patch.dict(os.environ, ENVIRONMENT, clear=True), \
                    patch.object(native.socket, 'socket', side_effect=AssertionError('invalid selection contacted native')):
                code, value = self.command(self.start_args())
                self.assertEqual(code, 2, value)
            self.assertFalse(Path(self.path).exists())
            self.assertFalse(value['result']['native_connection_attempted'])

    def test_private_import_rejects_noncanonical_custody_and_oversized_inputs(self):
        for mode in (0o400, 0o640, 0o660):
            self.receipts_file.chmod(mode)
            with self.subTest(mode=mode), self.assertRaises(Rejected):
                cli.load_receipts(str(self.receipts_file), rpc.Budget(1000))
        self.receipts_file.chmod(0o600)
        self.root.chmod(0o500)
        with self.assertRaises(Rejected):
            cli.load_receipts(str(self.receipts_file), rpc.Budget(1000))
        self.root.chmod(0o700)
        alias = self.root / 'alias'
        alias.symlink_to(self.receipts_file)
        with self.assertRaises((OSError, Rejected)):
            cli.load_receipts(str(alias), rpc.Budget(1000))
        hardlink = self.root / 'hardlink'
        os.link(self.receipts_file, hardlink)
        with self.assertRaises(Rejected):
            cli.load_receipts(str(self.receipts_file), rpc.Budget(1000))
        hardlink.unlink()
        original = self.receipts_file.read_bytes()
        self.receipts_file.write_bytes(original + b' ' * (cli.MAX_RECEIPT_INPUT + 1 - len(original)))
        with self.assertRaises(Rejected):
            cli.load_receipts(str(self.receipts_file), rpc.Budget(1000))


if __name__ == '__main__':
    unittest.main(verbosity=2)
