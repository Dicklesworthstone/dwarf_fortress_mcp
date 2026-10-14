"""Clock/batch workflow failures, bounded I/O and CLI on real native clock/files.

Only route acquisition, batch custody and the enclosing route packet are ports.
No source rewriting, native-client replacement, or synthetic storage result.
"""
from __future__ import annotations

from dataclasses import replace
import hashlib
import io
import json
import os
from pathlib import Path
import socket
import sys
import unittest
from unittest.mock import patch

import bounded_run_outcomes as storage
import route_furniture_clock_context as context
from furniture_plan import canonical
from test_route_furniture_clock_context import Budget, fixture, native_observation, reviewed


class ProgressContextTests(unittest.TestCase):
    def test_all_unready_and_ready_nonpending_frontiers_skip_clock_acquisition(self):
        for status in ('ready', 'construction_blocked', 'construction_unestablished',
                       'route_blocked', 'route_unestablished'):
            with self.subTest(status=status), fixture() as (m, route, peer, batch, auth, folder):
                batch.route_status = status
                result = m.review_progress(batch, auth, 20, 50)
                self.assertIsNone(result['progress_review']['confirm_progress'])
                self.assertIsNone(result['progress_review']['scope'])
                self.assertEqual(peer.operations, [])
                self.assertEqual(list(folder.iterdir()), [])

    def test_clock_review_cannot_hide_a_changed_bracketing_route_observation(self):
        for kind in ('ready', 'evidence'):
            with self.subTest(kind=kind), fixture() as (m, route, peer, batch, auth, folder):
                def change(state):
                    if batch.reads == 2:
                        if kind == 'ready':
                            state['route_frontier']['status'] = 'ready'
                            state.update(advance_allowed=True, expected_plan='a' * 64, confirm_review='f' * 64)
                        else: state['route_frontier']['evidence_digest'] = 'f' * 64
                batch.on_review = change
                result = m.review_progress(batch, auth, 20, 50)
                self.assertIsNone(result['progress_review']['confirm_progress'])
                self.assertEqual(result['progress_review']['reason'], 'frontier_changed_during_clock_review')
                self.assertEqual(peer.operations, ['Handshake', 'ObserveRun'])

    def test_new_ready_frontier_is_not_whole_batch_completion_or_placement(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            token = reviewed(m, batch, auth)
            peer.on_call = lambda p, op, f, n: setattr(batch, 'route_status', 'ready') if op == 'CommitRun' else None
            result = m.progress(batch, auth, 20, 50, token, str(folder))
            value = result['construction_progress']
            self.assertTrue(value['next_step_ready'])
            self.assertFalse(value['construction_completion_proven'])
            self.assertFalse(value['placement_dispatched'])
            self.assertEqual(result['placed'], 1)
            self.assertEqual(result['next_step'], 's01')
            self.assertEqual(value['post_progress_review']['expected_plan'], 'e' * 64)
            self.assertFalse(result['advance_allowed'])
            self.assertNotIn('PreparePlacement', peer.operations)
            self.assertFalse(m.packet('progress', result)['agent_turn']['briefing']['construction_completion_proven'])

    def test_failed_post_clock_read_keeps_durable_outcome_without_cached_readiness(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            token = reviewed(m, batch, auth)
            def fail(state):
                if 'CommitRun' in peer.operations:
                    raise OSError('source token must not escape')
            batch.on_review = fail
            result = m.progress(batch, auth, 20, 50, token, str(folder))
            value = result['construction_progress']
            self.assertTrue(value['clock_result']['terminal_record_retained'])
            self.assertEqual(value['post_progress_review_status'], 'readiness_unestablished')
            self.assertIsNone(value['post_progress_review'])
            self.assertFalse(value['next_step_ready'])
            self.assertNotIn(b'source token', canonical(result))
            self.assertEqual(peer.operations.count('CommitRun'), 1)
            self.assertEqual(batch.reads, 5)  # Two review, two re-review, one failed post-stop read.

    def test_missing_original_record_never_proves_nonapplication_or_retries(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            token = reviewed(m, batch, auth)
            peer.drop_commit = True
            with self.assertRaises(ValueError): m.progress(batch, auth, 20, 50, token, str(folder))
            peer.records.clear()
            batch.budget = Budget()
            result = m.query_progress(batch, auth, token, str(folder))
            value = result['construction_progress']
            self.assertEqual(value['clock_result']['native_record_status'], 'absent')
            self.assertFalse(value['clock_terminal_pause_verified'])
            self.assertFalse(value['clock_result']['terminal_record_retained'])
            self.assertFalse(value['clock_result']['retry_permitted'])
            self.assertEqual(peer.operations.count('CommitRun'), 1)
            self.assertEqual(len(list(folder.iterdir())), 2)

    def test_running_result_is_not_terminal_and_followup_uses_only_query(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            token = reviewed(m, batch, auth)
            peer.commit_phase = 'running'
            result = m.progress(batch, auth, 20, 50, token, str(folder))
            self.assertEqual(result['construction_progress']['clock_result']['record']['phase'], 'running')
            self.assertIsNone(result['construction_progress']['post_progress_review'])
            self.assertFalse(result['construction_progress']['clock_terminal_pause_verified'])
            key, saved = next(iter(peer.records.items()))
            peer.records[key] = (*saved[:3], 'stopped')
            batch.budget = Budget()
            before = len(peer.operations)
            query = m.query_progress(batch, auth, token, str(folder), refresh_frontier=True)
            self.assertEqual(peer.operations[before:], ['Handshake', 'QueryRun'])
            self.assertEqual(query['construction_progress']['post_progress_review']['status'], 'construction_pending')
            self.assertFalse(query['construction_progress']['clock_commit_invoked_this_call'])

    def test_actual_request_allowance_includes_queries_not_only_constant_reservation(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            token = reviewed(m, batch, auth)
            peer.commit_phase = 'running'
            m.progress(batch, auth, 20, 50, token, str(folder))
            batch.budget = Budget()
            batch.budget.calls = 8  # Six bindings + handshake + one query; no second query.
            before = len(peer.operations)
            with self.assertRaises(ValueError):
                m.query_progress(batch, auth, token, str(folder), wait_ms=1000)
            self.assertEqual(peer.operations[before:], ['Handshake', 'QueryRun'])
            self.assertEqual(batch.budget.calls, 0)
            self.assertEqual(len(list(folder.iterdir())), 2)
            self.assertEqual(peer.operations.count('CommitRun'), 1)

    def test_actual_network_and_deadline_changes_refuse_without_effect(self):
        for kind in ('bytes', 'deadline', 'revocation'):
            with self.subTest(kind=kind), fixture() as (m, route, peer, batch, auth, folder):
                def change(peer, operation, fields, connection):
                    if operation == 'ObserveRun':
                        if kind == 'bytes': batch.budget.network_bytes = 0
                        elif kind == 'deadline': batch.budget.deadline += 1
                        else: os.environ['DFMCP_RUN_TOKEN'] = 'x' * 32
                peer.on_call = change
                with self.assertRaises(ValueError):
                    m.review_progress(batch, auth, 20, 50)
                self.assertNotIn('PrepareRun', peer.operations)
                self.assertEqual(list(folder.iterdir()), [])

    def test_scope_and_intent_are_synced_before_prepare_and_lock_never_released(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            token = reviewed(m, batch, auth)
            path = m._record_path(str(folder), token)
            synced = set()
            violations = []
            real_sync = os.fsync
            def sync(fd):
                target = os.readlink(f'/proc/self/fd/{fd}')
                real_sync(fd)
                if target in (str(path), str(path) + context.SCOPE_SUFFIX):
                    synced.add((target, Path(target).read_bytes()))
            def observe(peer, op, fields, connection):
                if op in ('PrepareRun', 'CommitRun'):
                    for target in (path, Path(str(path) + context.SCOPE_SUFFIX)):
                        if (str(target), target.read_bytes()) not in synced:
                            violations.append('unsynchronized ' + target.name)
                    import fcntl
                    fd = os.open(folder, os.O_RDONLY | os.O_DIRECTORY)
                    try:
                        try:
                            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                            violations.append('directory lock was released')
                        except BlockingIOError:
                            pass
                    finally: os.close(fd)
            peer.on_call = observe
            with patch.object(os, 'fsync', side_effect=sync):
                m.progress(batch, auth, 20, 50, token, str(folder))
            self.assertEqual(violations, [])
            self.assertEqual(peer.operations.count('CommitRun'), 1)

    def test_sync_failure_leaves_evidence_and_never_repeats_clock_effect(self):
        for target in ('scope', 'intent', 'outcome'):
            with self.subTest(target=target), fixture() as (m, route, peer, batch, auth, folder):
                token = reviewed(m, batch, auth)
                path = m._record_path(str(folder), token)
                selected = str(path) + ({'scope': context.SCOPE_SUFFIX, 'outcome': storage.SUFFIX}.get(target, ''))
                real_sync = os.fsync
                def fail(fd):
                    if os.readlink(f'/proc/self/fd/{fd}') == selected:
                        raise OSError('synchronization failed')
                    return real_sync(fd)
                with patch.object(os, 'fsync', side_effect=fail), self.assertRaises(OSError):
                    m.progress(batch, auth, 20, 50, token, str(folder))
                self.assertTrue(Path(selected).exists())
                self.assertEqual(peer.operations.count('CommitRun'), int(target == 'outcome'))
                batch.budget = Budget()
                before = peer.operations.count('CommitRun')
                with self.assertRaises((OSError, ValueError)): m.progress(batch, auth, 20, 50, token, str(folder))
                self.assertEqual(peer.operations.count('CommitRun'), before)

    def test_changed_association_or_replaced_path_is_refused_unchanged_before_query(self):
        for kind in ('truncated', 'modified', 'symlink', 'permissions', 'hardlink'):
            with self.subTest(kind=kind), fixture() as (m, route, peer, batch, auth, folder):
                token = reviewed(m, batch, auth)
                peer.drop_commit = True
                with self.assertRaises(ValueError): m.progress(batch, auth, 20, 50, token, str(folder))
                path = Path(str(m._record_path(str(folder), token)) + context.SCOPE_SUFFIX)
                if kind == 'truncated': path.write_bytes(path.read_bytes()[:-1])
                elif kind == 'modified': path.write_bytes(path.read_bytes().replace(b'20', b'21', 1))
                elif kind == 'symlink':
                    path.rename(folder / 'moved')
                    path.symlink_to(folder / 'moved')
                elif kind == 'permissions': path.chmod(0o644)
                else: os.link(path, folder / 'linked')
                before = len(peer.operations)
                batch.budget = Budget()
                with self.assertRaises((OSError, ValueError)):
                    m.query_progress(batch, auth, token, str(folder))
                self.assertEqual(len(peer.operations), before)
                self.assertTrue(path.exists())

    def test_scope_codec_bounds_and_all_authorizing_fields_are_bound(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            review = m.review_progress(batch, auth, 20, 50)
            scope = context.Scope.create(review['progress_review']['scope'])
            value = scope.value()
            for key in ('batch_id', 'review_digest', 'plan_digest', 'inventory_digest', 'frontier_evidence'):
                with self.subTest(key=key):
                    other = context.Scope.create({**value, key: '1' * 64})
                    self.assertNotEqual(scope.digest, other.digest)
                    self.assertNotEqual(scope.intent()['idempotency_key'], other.intent()['idempotency_key'])
            for raw in (scope.raw[:-1], scope.raw + b' ', b'x' * 8192, b'[' * 10 + b']' * 10):
                with self.assertRaises(ValueError): context.Scope(raw).value()
            for change in ({'game_ticks': True}, {'wall_ms': 0}, {'next_step': '../foreign'},
                           {'clock_observation_hex': native_observation(tick=11).hex()},
                           {'schema': 'dfmcp.route-furniture-progress/1'}, {'unpause': True}):
                with self.assertRaises(ValueError): context.Scope.create({**value, **change})

    def test_invalid_cli_arguments_refuse_before_any_file_or_native_read(self):
        cases = [('query', ['--ticks', '1']), ('query', ['--wall-ms', '1']),
                 ('review', ['--ticks', '0', '--wall-ms', '50']),
                 ('review', ['--ticks', '20', '--wall-ms', '50', '--wait-ms', '0']),
                 ('review', ['--ticks', '20', '--wall-ms', '50', '--refresh-frontier']),
                 ('progress', ['--ticks', '20', '--wall-ms', '60001']),
                 ('query', ['--wait-ms', '-1'])]
        with fixture() as (m, route, peer, batch, auth, folder):
            for operation, options in cases:
                args = [operation, '--directory', '/unused', '--batch-id', batch.id, *options]
                if operation != 'review': args += ['--confirm-progress', 'd' * 64, '--clock-directory', str(folder)]
                output = io.BytesIO()
                with self.subTest(args=args), patch.object(m, 'Batch', side_effect=AssertionError('custody opened')), \
                     patch.object(sys, 'stdout', types_stdout(output)):
                    self.assertEqual(m.main(args), 2)
                    self.assertFalse(json.loads(output.getvalue())['ok'])
                    self.assertEqual(output.getvalue().count(b'\n'), 1)
            self.assertEqual(peer.operations, [])

    def test_cli_review_progress_then_offline_query_share_real_clock_and_files(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            prefix = ['--directory', '/explicit-batch-port', '--batch-id', batch.id]
            def open_batch(path, expected, budget, *, writable):
                self.assertFalse(writable)
                self.assertEqual(expected, batch.id)
                batch.closed, batch.budget = False, budget
                return batch
            def call(args):
                output = io.BytesIO()
                with patch.object(m, 'Batch', side_effect=open_batch), patch.object(sys, 'stdout', types_stdout(output)):
                    status = m.main(args + prefix)
                self.assertEqual(status, 0, output.getvalue())
                self.assertTrue(batch.closed)
                return json.loads(output.getvalue())
            review = call(['review', '--ticks', '20', '--wall-ms', '50'])
            token = review['result']['progress_review']['confirm_progress']
            value = call(['progress', '--ticks', '20', '--wall-ms', '50',
                          '--confirm-progress', token, '--clock-directory', str(folder)])
            self.assertTrue(value['result']['construction_progress']['clock_commit_invoked_this_call'])
            before = len(peer.operations)
            with patch.dict(os.environ, {}, clear=True):
                recovered = call(['query', '--confirm-progress', token, '--clock-directory', str(folder)])
            self.assertEqual(len(peer.operations), before)
            self.assertFalse(recovered['result']['construction_progress']['native_query_contacted'])

    def test_cli_partial_stdout_never_emits_second_packet_or_retries(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            class Output:
                calls = 0
                def write(self, data):
                    self.calls += 1
                    return 1
                def flush(self): pass
            output = Output()
            with patch.object(m, 'Batch', return_value=batch), patch.object(sys, 'stdout', types_stdout(output)):
                result = m.main(['review', '--directory', '/explicit', '--batch-id', batch.id,
                                 '--ticks', '20', '--wall-ms', '50'])
            self.assertEqual(result, 2)
            self.assertEqual(output.calls, 1)
            self.assertTrue(batch.closed)
            self.assertEqual(peer.operations, ['Handshake', 'ObserveRun'])

    def test_post_stop_route_cannot_regress_behind_the_clock_receipt(self):
        for kind in ('tick', 'horizon', 'generation'):
            with self.subTest(kind=kind), fixture() as (m, route, peer, batch, auth, folder):
                token = reviewed(m, batch, auth)
                def change(state):
                    if 'CommitRun' in peer.operations:
                        state.update(advance_allowed=True, expected_plan='e' * 64, confirm_review='f' * 64)
                        report = state['route_frontier']
                        report['status'] = 'ready'
                        if kind == 'tick': report['source']['tick'] = 29  # Original10 < 29 < clock-stop30.
                        elif kind == 'horizon': report['source']['horizons'][0] -= 1
                        else: report['source']['map_generation'] += 1
                batch.on_review = change
                value = m.progress(batch, auth, 20, 50, token, str(folder))['construction_progress']
                self.assertFalse(value['next_step_ready'])
                self.assertTrue(value['clock_terminal_pause_verified'])
                self.assertIsNone(value['post_progress_review'])
                self.assertEqual(value['post_progress_review_status'], 'readiness_unestablished')
                self.assertEqual(peer.operations.count('CommitRun'), 1)

    def test_terminal_recovery_after_clock_incarnation_change_keeps_history_only(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            token = reviewed(m, batch, auth)
            peer.drop_commit = True
            with self.assertRaises(ValueError): m.progress(batch, auth, 20, 50, token, str(folder))
            peer.generation += 1
            batch.budget = Budget()
            before = batch.reads
            value = m.query_progress(batch, auth, token, str(folder), refresh_frontier=True)['construction_progress']
            self.assertTrue(value['clock_terminal_pause_verified'])
            self.assertEqual(value['post_progress_review_status'], 'clock_stop_source_unestablished')
            self.assertEqual(batch.reads, before)
            self.assertFalse(value['next_step_ready'])
            self.assertFalse(value['clock_commit_invoked_this_call'])

    def test_output_refusal_reserves_before_any_new_clock_file_or_effect(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            token = reviewed(m, batch, auth)
            batch.on_review = lambda state: state.update(padding='x' * m.MAX_OUTPUT)
            with self.assertRaises(ValueError): m.progress(batch, auth, 20, 50, token, str(folder))
            self.assertNotIn('PrepareRun', peer.operations)
            self.assertEqual(list(folder.iterdir()), [])

    def test_guard_vocabulary_and_environment_never_grant_furniture_effects(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            for operation in ('Handshake', 'ReadPlacement', 'QueryPlacement'):
                auth.guard(operation)
            for operation in ('PreparePlacement', 'CommitPlacement', 'CancelPlacement', 'ObservePlacement'):
                with self.assertRaises(ValueError): auth.guard(operation)
            for field, value in ((route.PLACE, '1'), ('DFMCP_ADMITTED_BRIDGE_PROTOCOL', '1.0'),
                                 ('DFMCP_RUN_ENDPOINT', '127.0.0.1:05000')):
                with patch.dict(os.environ, {field: value}), self.assertRaises(ValueError):
                    m.ProgressAuthority.load()
            self.assertEqual(peer.operations, [])


def types_stdout(buffer):
    import types
    return types.SimpleNamespace(buffer=buffer)


if __name__ == '__main__':
    unittest.main()
