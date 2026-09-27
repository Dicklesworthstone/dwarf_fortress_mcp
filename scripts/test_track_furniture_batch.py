"""Execute original furnishing batches through their receipt-linked completion CLI.

The joined peer retains the real placement client's generated records and serves
the operations profile on that same listener. No live DFHack, SDK build or
physical power-loss qualification is implied by these process/TCP/POSIX tests.
Beads: df-dfhack-bridge-plane-c-pic.4 / df-dfhack-bridge-plane-c-pic.5.
"""
from __future__ import annotations

from contextlib import contextmanager, redirect_stdout, redirect_stderr
from dataclasses import replace
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import build_placement_wire as wire
import construction_monitor_rpc as native
import construction_plan as condition
import furniture_batch as batch
import furniture_plan as plan_model
import track_furniture_batch as cli
from test_furniture_batch import Peer as PlacementPeer, environment as placement_environment, make_plan, proto
from test_construction_monitor_rpc import read_message
from test_construction_plan import operations_for

ROOT = Path(__file__).resolve().parents[1]


class Peer(PlacementPeer):
    """Existing native placement peer plus a strictly query-only monitor phase.

    Tests explicitly switch phases after original placements, keeping the same
    listener, native source, records and effects. The original placement handler
    runs unchanged; the monitoring handler rejects every effectful binding.
    """
    def __init__(self, plan):
        self.monitoring = False
        self.sample_tick = 901
        self.statuses = {}
        self.monitor_hook = None
        self.monitor_connections = self.monitor_reads = self.monitor_releases = self.monitor_queries = 0
        self.monitor_calls, self.monitor_bindings = [], []
        super().__init__(plan)

    def capture(self, selected):
        captured = super().capture(selected)
        # Independent Bed/Chair/Table native enum identities are consistent in
        # both the actual original receipts and the later complete item roster.
        return replace(captured, item=replace(captured.item, native_type=100 + selected.kind))

    def operations(self):
        records = sorted((record for record in self.records.values() if record.phase == 'placed'),
                         key=lambda record: record.insertion.building)
        goal = condition.Goal(tuple(record.raw for record in records), min(self.tick + 100, wire.MAX_TICK))
        by_item = {step.item: step.name for step in self.plan.steps}
        statuses = {index: self.statuses[by_item[record.insertion.item]]
                    for index, record in enumerate(records) if by_item[record.insertion.item] in self.statuses}
        return operations_for(goal, tick=self.sample_tick, statuses=statuses,
                              folder=self.folder, site=self.site)

    def connection(self, sock):
        if not self.monitoring:
            return super().connection(sock)
        self.monitor_connections += 1
        assert self.read(sock, 12) == b'DFHack?\n' + struct.pack('<i', 1)
        self.send(sock, b'DFHack!\n' + struct.pack('<i', 1))
        methods, queried, raw, released = {}, 0, None, False
        originals = sorted((record for record in self.records.values() if record.phase == 'placed'),
                           key=lambda record: record.insertion.building)
        while not self.closed.is_set():
            method, size = struct.unpack('<h2xi', self.read(sock, 8))
            assert 0 <= size <= 2048
            fields = read_message(self.read(sock, size))
            if method == 0:
                assert set(fields) == {1, 2, 3, 4}
                plugin = fields[4].decode('ascii')
                family = 'build' if plugin == native.PROFILES['build'][0] else 'operations'
                name = fields[1].decode('ascii')
                assert (family, name) in native.BINDINGS, 'monitor attempted an effectful native binding'
                assert plugin == native.PROFILES[family][0]
                assert fields[2] == (native.PROFILES[family][1] + '.Request').encode()
                assert fields[3] == (native.PROFILES[family][1] + '.Reply').encode()
                identifier = len(methods) + 2
                methods[identifier] = family, name
                self.monitor_bindings.append((family, name))
                response, tag = {1: identifier}, 'bind'
            else:
                family, operation = methods[method]
                self.calls.append(operation)
                self.monitor_calls.append((family, operation))
                assert fields[1] == (b't' * 32 if family == 'build' else b'o' * 32)
                assert len(fields[2]) == 32 and fields[3] == 1
                assert fields[4] == native.PROFILES[family][2]
                response = {1: 1, 2: 0, 3: fields[2], 4: 1, 5: fields[4],
                            6: self.generation if family == 'build' else 987,
                            7: self.df.encode(), 8: self.dfhack.encode()}
                tag = family + '_handshake'
                if family == 'build':
                    response.update({12: 0, 13: len(self.records)})
                    if operation == 'Handshake':
                        assert set(fields) == {1, 2, 3, 4}
                    else:
                        assert operation == 'QueryPlacement' and set(fields) == {1, 2, 3, 4, 10, 12}
                        assert queried < 2 * len(originals)
                        expected = originals[queried % len(originals)]
                        assert fields[10] == expected.plan.key.encode() and fields[12] == expected.plan.digest
                        tag = 'before_receipt' if queried < len(originals) else 'after_receipt'
                        assert raw is None if tag == 'before_receipt' else released
                        queried += 1
                        self.monitor_queries += 1
                        response[10] = expected.raw
                else:
                    assert tuple(fields[n] for n in (5, 6, 7, 8, 11)) == (4096, 4096, 65536, native.MAX_CAPTURE, native.PAGE)
                    if operation == 'Handshake':
                        assert set(fields) == set(range(1, 9)) | {11}
                    else:
                        assert operation == 'ReadObservation' and set(fields) == set(range(1, 13))
                        assert queried == len(originals)
                        token, offset, release = fields[9], fields[10], fields[12]
                        if release:
                            assert release == 1 and token == b't' * 16 and offset == 0 and raw is not None and not released
                            released = True
                            self.monitor_releases += 1
                            response[10], tag = token, 'release'
                        else:
                            assert not released
                            if not token:
                                assert offset == 0 and raw is None
                                raw = self.operations()
                            else:
                                assert token == b't' * 16 and raw is not None and offset > 0
                            part = raw[offset:offset + native.PAGE]
                            self.monitor_reads += 1
                            response.update({9: part, 10: b't' * 16, 11: offset, 12: len(raw),
                                             13: hashlib.sha256(raw).digest(),
                                             14: int(offset + len(part) == len(raw))})
                            tag = 'page'
            if self.monitor_hook:
                response = self.monitor_hook(tag, fields, response, self)
            encoded = proto(response)
            self.send(sock, struct.pack('<h2xi', -1, len(encoded)) + encoded)


@contextmanager
def monitor_environment(peer):
    environment = {key: value for key, value in os.environ.items() if not key.startswith('DFMCP_')}
    environment.update({native.OPT_IN: '1', native.ENDPOINT: f'{peer.address[0]}:{peer.address[1]}',
                        native.BUILD_TOKEN: 't' * 32, native.OPERATIONS_TOKEN: 'o' * 32})
    with patch.dict(os.environ, environment, clear=True):
        yield


class BatchMonitorTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='dfmcp-original-batch-completion-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.root.chmod(0o700)
        self.batch_path = self.root / 'batch'
        self.batch_path.mkdir(mode=0o700)
        self.journal = str(self.root / 'completion.monitor')
        self.plan = make_plan(3)
        self.peer = Peer(self.plan)
        self.addCleanup(lambda: self.peer.close())
        with placement_environment(self.peer):
            initial = batch.initialize(str(self.batch_path), self.plan, self.peer.folder, self.peer.site)
        self.batch_id = initial['batch_id']
        self.deadline = self.peer.tick + 100
        self.last_stdout = b''

    def advance(self, count=None):
        with placement_environment(self.peer):
            for _ in range(len(self.plan.steps) if count is None else count):
                review = batch.review(str(self.batch_path), self.batch_id)
                out = batch.advance(str(self.batch_path), self.batch_id,
                                    review['expected_plan'], review['confirm_review'])
        return out

    def original_bytes(self, path=None):
        source = self.batch_path if path is None else path
        return {str(file.relative_to(source)): file.read_bytes()
                for file in sorted(source.rglob('*')) if file.is_file()}

    def start_args(self, *, journal=None, batch_id=None):
        return ['start', '--journal', self.journal if journal is None else journal,
                '--batch', str(self.batch_path), '--batch-id', self.batch_id if batch_id is None else batch_id,
                '--deadline-tick', str(self.deadline)]

    def command(self, arguments, *, process=False):
        if process:
            result = subprocess.run([sys.executable, str(ROOT / 'scripts/track_furniture_batch.py'), *arguments],
                                    capture_output=True, timeout=70, env=dict(os.environ))
            self.assertEqual(result.stderr, b'', result.stderr)
            code, self.last_stdout = result.returncode, result.stdout
        else:
            stdout, stderr = io.StringIO(), io.StringIO()
            with redirect_stdout(stdout), redirect_stderr(stderr):
                code = cli.main(arguments)
            self.assertEqual(stderr.getvalue(), '')
            self.last_stdout = stdout.getvalue().encode('ascii')
        self.assertLessEqual(len(self.last_stdout), 65536)
        self.assertEqual(self.last_stdout.count(b'\n'), 1)
        value = json.loads(self.last_stdout)
        self.assertEqual(value['schema'], 'dfmcp.furniture-batch-monitor-result/1')
        self.assertEqual(value['ok'], code == 0)
        self.assertIsNone(value['agent_turn']['anchor'])
        self.assertFalse(value['agent_turn']['briefing']['runtime_admitted'])
        self.assertFalse(value['agent_turn']['briefing']['mutation_admissible'])
        self.assertFalse(value['result']['placement_effects_discharged'])
        self.assertFalse(value['result']['retry_placement_permitted'])
        self.assertFalse(value['result']['current_usability_proven'])
        self.assertNotIn(b't' * 32, self.last_stdout)
        self.assertNotIn(b'o' * 32, self.last_stdout)
        return code, value

    def start(self, *, process=False):
        self.peer.monitoring = True
        with monitor_environment(self.peer):
            return self.command(self.start_args(), process=process)

    def sample(self, *, process=False):
        with monitor_environment(self.peer):
            return self.command(['sample', '--journal', self.journal], process=process)

    def assert_plan(self, value, *, source_verified=True):
        result = value['result']
        self.assertEqual(result['requested_plan'], self.plan.json())
        self.assertEqual(result['origin']['batch_id'], self.batch_id)
        self.assertEqual(result['origin']['plan_digest'], self.plan.digest)
        self.assertEqual(result['origin']['requested_step_count'], len(self.plan.steps))
        self.assertEqual(result['origin']['source_custody_verified'], source_verified)
        self.assertEqual(result['original_placement_history_verified'], source_verified)
        self.assertEqual(len(result['targets']), len(self.plan.steps))
        expected = {step.name: (step.item, list(step.target), step.kind) for step in self.plan.steps}
        self.assertEqual({row['plan_step']: (row['item_id'], row['position'], row['kind'])
                          for row in result['targets']}, expected)

    def assert_only_monitor_reads(self, original_commits):
        self.assertEqual(self.peer.commits, original_commits)
        self.assertTrue(all(method in native.BINDINGS for method in self.peer.monitor_bindings))
        self.assertTrue(all(method in native.BINDINGS for method in self.peer.monitor_calls))
        self.assertEqual(self.peer.monitor_reads, self.peer.monitor_releases)

    def test_real_three_kind_batch_all_members_must_hold_together_across_processes(self):
        out = self.advance()
        self.assertEqual(out['status'], 'all_placed')
        self.assertFalse(out['construction_completion_proven'])
        original, commits = self.original_bytes(), list(self.peer.commits)
        code, first = self.start(process=True)
        self.assertEqual(code, 0, first)
        self.assert_plan(first)
        self.assertEqual(first['result']['progress']['streak'], 1)
        self.assertFalse(first['result']['complete_original_plan_sampled_condition'])
        self.peer.sample_tick += 1
        self.peer.statuses = {self.plan.ordered[-1].name: 'item_unverified'}
        code, pending = self.sample(process=True)
        self.assertEqual(code, 0, pending)
        self.assertEqual(pending['result']['progress']['streak'], 0)
        self.assertEqual(pending['result']['progress']['condition_met_count'], 2)
        self.assertFalse(pending['result']['complete_original_plan_sampled_condition'])
        self.peer.statuses = {}
        for streak in (1, 2):
            self.peer.sample_tick += 1
            code, value = self.sample(process=True)
            self.assertEqual(code, 0, value)
            self.assert_plan(value)
            self.assertEqual(value['result']['progress']['streak'], streak)
            self.assertEqual(value['result']['identity']['goal_digest'], first['result']['identity']['goal_digest'])
            self.assertEqual(value['result']['goal']['deadline_tick'], self.deadline)
            self.assertEqual(value['result']['complete_original_plan_sampled_condition'], streak == 2)
        self.assertEqual(value['result']['progress']['phase'], 'satisfied')
        self.assertEqual((self.peer.monitor_connections, self.peer.monitor_reads, self.peer.monitor_queries), (4, 4, 24))
        self.assertEqual(self.original_bytes(), original)
        self.assert_only_monitor_reads(commits)

    def test_offline_batch_stop_between_samples_does_not_retarget_completed_plan(self):
        self.advance()
        code, value = self.start()
        self.assertEqual(code, 0, value)
        original = self.original_bytes()
        with patch.dict(os.environ, {}, clear=True):
            stopped = batch.stop(str(self.batch_path), self.batch_id)
        self.assertTrue(stopped['stopped'])
        self.assertEqual({key: value for key, value in self.original_bytes().items() if key != 'stop.json'}, original)
        self.peer.sample_tick += 1
        code, value = self.sample(process=True)
        self.assertEqual(code, 0, value)
        self.assert_plan(value)
        self.assertTrue(value['result']['complete_original_plan_sampled_condition'])
        # A completed stopped batch can also seed a new, independently fixed
        # completion monitor; its stop marker does not erase original receipts.
        self.journal = str(self.root / 'after-stop.monitor')
        code, value = self.start()
        self.assertEqual(code, 0, value)
        self.assert_plan(value)
        self.assertFalse(value['result']['complete_original_plan_sampled_condition'])

    def test_wrong_batch_unplaced_and_unregistered_plans_refuse_before_journal_or_native(self):
        def refused(arguments):
            before = self.peer.connections
            with monitor_environment(self.peer), \
                    patch.object(native.socket, 'socket', side_effect=AssertionError('ineligible batch opened native socket')):
                code, value = self.command(arguments)
            self.assertEqual(code, 2, value)
            self.assertFalse(value['result']['native_connection_attempted'])
            self.assertEqual(self.peer.connections, before)
            self.assertFalse(Path(self.journal).exists())
        refused(self.start_args(batch_id='0' * 64))
        refused(self.start_args())
        self.advance(1)
        refused(self.start_args())
        self.advance(2)
        index = self.batch_path / 'steps.jsonl'
        lines = index.read_bytes().splitlines(keepends=True)
        index.write_bytes(b''.join(lines[:-1]))  # Complete original placed child, absent registration.
        original = self.original_bytes()
        refused(self.start_args())
        self.assertEqual(self.original_bytes(), original)

    def test_original_child_inode_replacement_blocks_inspection_and_sample(self):
        self.advance()
        code, value = self.start()
        self.assertEqual(code, 0, value)
        original_monitor = Path(self.journal).read_bytes()
        child = next((self.batch_path / 'effects').iterdir())
        old_identity, original = child.stat().st_ino, child.read_bytes()
        replacement = self.root / 'replacement'
        replacement.write_bytes(original)
        replacement.chmod(0o600)
        self.assertNotEqual(replacement.stat().st_ino, old_identity)
        os.replace(replacement, child)
        before = self.peer.monitor_queries
        with monitor_environment(self.peer), \
                patch.object(native.socket, 'socket', side_effect=AssertionError('substituted source contacted native')):
            for operation in ('inspect', 'sample'):
                code, value = self.command([operation, '--journal', self.journal])
                self.assertEqual(code, 2, value)
                self.assertFalse(value['result']['complete_original_plan_sampled_condition'])
                self.assertFalse(value['result']['original_placement_history_verified'])
                self.assertEqual(Path(self.journal).read_bytes(), original_monitor)
        self.assertEqual(self.peer.monitor_queries, before)
        self.assertEqual(child.read_bytes(), original)

    def test_original_batch_path_replacement_cannot_inherit_completion_goal(self):
        self.advance()
        code, value = self.start()
        self.assertEqual(code, 0, value)
        original_monitor, original = Path(self.journal).read_bytes(), self.original_bytes()
        moved = self.root / 'original-batch'
        self.batch_path.rename(moved)
        shutil.copytree(moved, self.batch_path)
        self.assertNotEqual(moved.stat().st_ino, self.batch_path.stat().st_ino)
        with monitor_environment(self.peer), \
                patch.object(native.socket, 'socket', side_effect=AssertionError('copied source opened native socket')):
            code, value = self.command(['sample', '--journal', self.journal])
        self.assertEqual(code, 2, value)
        self.assertFalse(value['result']['complete_original_plan_sampled_condition'])
        self.assertEqual(Path(self.journal).read_bytes(), original_monitor)
        self.assertEqual(self.original_bytes(), original)
        self.assertEqual(self.original_bytes(moved), original)

    def test_first_monitor_sample_cannot_adopt_new_native_software(self):
        self.advance()
        original, commits = self.original_bytes(), list(self.peer.commits)
        self.peer.df = 'different-df-but-both-profiles-agree'
        code, value = self.start()
        self.assertEqual(code, 2, value)
        self.assertTrue(value['result']['native_connection_attempted'])
        self.assertFalse(value['result']['complete_original_plan_sampled_condition'])
        with patch.dict(os.environ, {}, clear=True), \
                patch.object(native.socket, 'socket', side_effect=AssertionError('offline inspection contacted native')):
            code, inspected = self.command(['inspect', '--journal', self.journal])
        self.assertEqual(code, 0, inspected)
        self.assert_plan(inspected)
        self.assertEqual(inspected['result']['progress']['observations'], 0)
        self.assertTrue(inspected['result']['progress']['read_outcome_unknown'])
        self.assertEqual(self.original_bytes(), original)
        self.assert_only_monitor_reads(commits)

    def test_cancellation_without_original_batch_or_credentials_retains_source_uncertainty(self):
        self.advance()
        code, value = self.start()
        self.assertEqual(code, 0, value)
        moved = self.root / 'retained-original'
        original = self.original_bytes()
        self.batch_path.rename(moved)
        with patch.dict(os.environ, {}, clear=True), \
                patch.object(native.socket, 'socket', side_effect=AssertionError('offline cancellation contacted native')):
            code, refused = self.command(['inspect', '--journal', self.journal])
            self.assertEqual(code, 2, refused)
            code, value = self.command(['cancel', '--journal', self.journal])
            self.assertEqual(code, 0, value)
            self.assert_plan(value, source_verified=False)
            self.assertEqual(value['result']['progress']['phase'], 'cancelled')
            self.assertFalse(value['result']['complete_original_plan_sampled_condition'])
            self.assertTrue(value['result']['storage_acknowledged_this_call'])
            retained = Path(self.journal).read_bytes()
            code, repeated = self.command(['cancel', '--journal', self.journal], process=True)
            self.assertEqual(code, 0, repeated)
            self.assert_plan(repeated, source_verified=False)
            self.assertFalse(repeated['result']['storage_acknowledged_this_call'])
        self.assertEqual(Path(self.journal).read_bytes(), retained)
        self.assertEqual(self.original_bytes(moved), original)

    def test_terminal_inspection_and_sample_reverify_original_history_offline_without_writes(self):
        self.advance()
        code, value = self.start()
        self.assertEqual(code, 0, value)
        self.peer.sample_tick += 1
        code, value = self.sample()
        self.assertEqual(code, 0, value)
        self.assertTrue(value['result']['complete_original_plan_sampled_condition'])
        original, retained, commits = self.original_bytes(), Path(self.journal).read_bytes(), list(self.peer.commits)
        with patch.dict(os.environ, {}, clear=True), \
                patch.object(native.socket, 'socket', side_effect=AssertionError('terminal native access')), \
                patch('os.fsync', side_effect=AssertionError('terminal write')):
            for operation in ('inspect', 'sample'):
                code, value = self.command([operation, '--journal', self.journal])
                self.assertEqual(code, 0, value)
                self.assert_plan(value)
                self.assertTrue(value['result']['complete_original_plan_sampled_condition'])
                self.assertFalse(value['result']['native_connection_attempted'])
                self.assertFalse(value['result']['storage_acknowledged_this_call'])
            code, value = self.command(['cancel', '--journal', self.journal])
            self.assertEqual(code, 0, value)
            self.assert_plan(value, source_verified=False)
            self.assertEqual(value['result']['progress']['phase'], 'satisfied')
            self.assertFalse(value['result']['complete_original_plan_sampled_condition'])
        self.assertEqual(self.original_bytes(), original)
        self.assertEqual(Path(self.journal).read_bytes(), retained)
        self.assert_only_monitor_reads(commits)

    def test_original_child_replaced_during_final_query_prevents_sample_publication(self):
        self.advance()
        child = next((self.batch_path / 'effects').iterdir())
        original = child.read_bytes()
        final_key = max(self.peer.records.values(), key=lambda record: record.insertion.building).plan.key.encode()
        changed = False
        def hook(tag, fields, reply, peer):
            nonlocal changed
            if tag == 'after_receipt' and fields[10] == final_key:
                replacement = self.root / 'during-read-replacement'
                replacement.write_bytes(original)
                replacement.chmod(0o600)
                os.replace(replacement, child)
                changed = True
            return reply
        self.peer.monitor_hook = hook
        code, value = self.start()
        self.assertTrue(changed)
        self.assertEqual(code, 2, value)
        self.assertFalse(value['result']['complete_original_plan_sampled_condition'])
        self.assertFalse(value['result']['storage_acknowledged_this_call'])
        with cli.Journal(self.journal, cli.Budget(10000)) as owner:
            self.assertEqual(owner.state.progress.observations, 0)
            self.assertTrue(owner.state.progress.reading)
        self.assertEqual(child.read_bytes(), original)

    def test_reopening_cannot_replace_original_plan_policy_endpoint_or_import_receipts(self):
        self.advance()
        code, value = self.start()
        self.assertEqual(code, 0, value)
        retained = Path(self.journal).read_bytes()
        extras = [['--batch', str(self.batch_path)], ['--batch-id', self.batch_id],
                  ['--deadline-tick', str(self.deadline + 1)], ['--stable-samples', '3'],
                  ['--receipts-file', 'do-not-echo-this-selection']]
        with monitor_environment(self.peer), \
                patch.object(native.socket, 'socket', side_effect=AssertionError('substituted plan contacted native')):
            for extra in extras:
                code, value = self.command(['sample', '--journal', self.journal, *extra])
                self.assertEqual(code, 2, value)
                self.assertFalse(value['result']['native_connection_attempted'])
                self.assertNotIn(b'do-not-echo-this-selection', self.last_stdout)
                self.assertEqual(Path(self.journal).read_bytes(), retained)
            port = 5001 if self.peer.address[1] != 5001 else 5002
            with patch.dict(os.environ, {native.ENDPOINT: f'127.0.0.1:{port}'}):
                code, value = self.command(['sample', '--journal', self.journal])
            self.assertEqual(code, 2, value)
            self.assertEqual(Path(self.journal).read_bytes(), retained)

    def test_monitor_cannot_be_created_inside_original_closed_batch_inventory(self):
        self.advance()
        original = self.original_bytes()
        for target in (self.batch_path / 'monitor', self.batch_path / 'effects' / 'monitor'):
            with monitor_environment(self.peer), \
                    patch.object(native.socket, 'socket', side_effect=AssertionError('invalid monitor path contacted native')):
                code, value = self.command(self.start_args(journal=str(target)))
                self.assertEqual(code, 2, value)
                self.assertFalse(target.exists())
        self.assertEqual(self.original_bytes(), original)

    def test_32_actual_placements_dense_full_plan_and_complete_response_bound(self):
        self.peer.close()
        names = tuple(f'r{index:02}' + 'x' * 45 for index in range(32))
        steps = tuple(plan_model.Step(names[index], plan_model.KINDS[index % 3], 2147483600 + index,
                                      (10 + 3 * index, 10, index % 4)) for index in range(32))
        dense = plan_model.FurniturePlan(steps)
        # Fill the real normalized 16-KiB plan allowance with acyclic original
        # dependencies and maximum-width names, without guessing its JSON size.
        for index in range(31, 0, -1):
            for predecessor in range(index):
                candidate = list(dense.steps)
                candidate[index] = replace(candidate[index], after=(*candidate[index].after, names[predecessor]))
                try:
                    dense = plan_model.FurniturePlan(tuple(candidate))
                except ValueError:
                    break
        self.plan = dense
        self.assertGreater(len(plan_model.canonical(dense.json())), 16300)
        self.assertGreater(sum(len(step.after) for step in dense.steps), 200)
        self.peer = Peer(dense)
        self.peer.folder, self.peer.df, self.peer.dfhack = 'é' * 256, 'd' * 128, 'h' * 128
        self.peer.generation, self.peer.sequence = 2**64 - 2, 2**64 - 100
        self.peer.tick = wire.MAX_TICK - 1000
        self.peer.sample_tick = self.peer.tick + 1
        self.peer.next_building = self.peer.next_job = 2147483600
        self.deadline = self.peer.tick + 100
        self.batch_path = self.root / 'full-batch'
        self.batch_path.mkdir(mode=0o700)
        with placement_environment(self.peer):
            initial = batch.initialize(str(self.batch_path), dense, self.peer.folder, self.peer.site)
        self.batch_id = initial['batch_id']
        placed = self.advance()
        self.assertEqual(placed['placed'], 32)
        original, commits = self.original_bytes(), list(self.peer.commits)
        self.peer.monitoring = True
        with monitor_environment(self.peer):
            code, value = self.command([*self.start_args(), '--timeout-ms', '60000'], process=True)
        self.assertEqual(code, 0, value)
        self.assert_plan(value)
        self.assertEqual(len(value['result']['progress']['assessments']), 32)
        self.assertEqual(value['result']['progress']['condition_met_count'], 32)
        self.assertFalse(value['result']['complete_original_plan_sampled_condition'])
        actual_length = len(self.last_stdout)
        with cli.Journal(self.journal, cli.Budget(60000)) as owner:
            reserved = cli.reserve('start', owner.state, source_verified=True,
                                   native_contacted=True, storage_acknowledged=True)
            self.assertGreaterEqual(len(reserved), actual_length)
            self.assertLessEqual(len(reserved), 65536)
        self.assertEqual((self.peer.monitor_connections, self.peer.monitor_reads, self.peer.monitor_queries), (1, 1, 64))
        self.assertEqual(self.original_bytes(), original)
        self.assert_only_monitor_reads(commits)
        print('MAXIMUM_32_FURNITURE_COMPLETION_RESPONSE_BYTES', actual_length,
              'RESERVED_BYTES', len(reserved),
              'REQUESTED_PLAN_BYTES', len(plan_model.canonical(dense.json())))


if __name__ == '__main__':
    unittest.main(verbosity=2)
