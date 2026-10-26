"""Complete recovery CLI, real private clock files and actual query-only TCP.

Only original route-batch opening and aggregate budget construction are ports;
packet rendering, argument parsing, recovery, native codec/transport and history
custody execute unchanged. No real game or complete private-batch claim.
"""
from contextlib import contextmanager
import fcntl
import io
import json
import os
from pathlib import Path
import runpy
import socket
import sys
import types
import unittest
from unittest.mock import patch

import bounded_run_outcomes as storage
import recover_scoped_clocks as cli
from scoped_clock_recovery import Recovery
from test_route_furniture_clock_inventory import setup, save, bytes_in
from test_route_furniture_clock_inventory_workflow import Budget
from test_scoped_clock_reconciliation import QueryPeer, configured, expected, final_save


class Output:
    def __init__(self, short=False, failure=False, on_write=lambda: None):
        self.buffer = self
        self.raw, self.writes, self.flushes = b'', 0, 0
        self.short, self.failure, self.on_write = short, failure, on_write

    def write(self, raw):
        self.writes += 1
        self.on_write()
        if self.failure:
            raise OSError('output unavailable')
        self.raw += raw
        return len(raw) - 1 if self.short else len(raw)

    def flush(self):
        self.flushes += 1


@contextmanager
def owners(batch):
    opened, budgets = [], []
    batch.closed = False
    def make_budget(timeout):
        budgets.append(timeout)
        batch.budget = Budget(timeout)
        return batch.budget
    def open_batch(path, identity, budget):
        assert identity == batch.id and budget is batch.budget
        opened.append(path)
        batch.closed = False
        return batch
    def close():
        batch.closed = True
    with patch.object(cli, 'make_budget', side_effect=make_budget), \
         patch.object(cli, 'open_batch', side_effect=open_batch), \
         patch.object(batch, 'close', side_effect=close, create=True):
        yield opened, budgets


def arguments(directory, batch, root=None, maximum=8):
    return ['--directory', '/private/original-route-batch', '--batch-id', batch.id,
            '--clock-directory', str(directory),
            '--expected-inventory', root or expected(directory, batch),
            '--max-queries', str(maximum)]


def invoke(args, output=None):
    output = output or Output()
    with patch.object(cli.sys, 'stdout', output):
        code = cli.main(args)
    return code, json.loads(output.raw), output


class RecoveryCliTests(unittest.TestCase):
    def test_complete_cli_recovers_discovered_next_and_final_records_without_route_credentials(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer), owners(batch) as ports:
            one, _ = save(directory, batch, phase=None)
            two, _ = final_save(directory, batch, 1)
            for scope in (one, two):
                peer.records[scope.clock_key] = (scope.intent(), 'stopped')
            args = arguments(directory, batch)
            code, packet, out = invoke(args)
            self.assertEqual(code, 0)
            self.assertTrue(packet['ok'])
            result = packet['result']
            self.assertEqual(result['batch_id'], batch.id)
            self.assertEqual(result['review_digest'], batch.review.digest)
            self.assertFalse(result['advance_allowed'])
            self.assertIsNone(result['expected_plan'])
            self.assertIsNone(result['confirm_review'])
            self.assertEqual(result['clock_recovery']['counts']['unresolved'], 0)
            self.assertEqual(packet['agent_turn']['briefing']['attempted_original_queries'], 2)
            self.assertEqual(len(packet['agent_turn']['changes']), 2)
            self.assertEqual(set(peer.keys), {one.clock_key, two.clock_key})
            self.assertEqual(out.writes, 1)
            self.assertTrue(batch.closed)
            self.assertEqual(ports, (['/private/original-route-batch'], [10000]))
            for credential in ('r' * 32, 'DFMCP_MAP_TOKEN', 'DFMCP_BUILD_TOKEN'):
                self.assertNotIn(credential, out.raw.decode())

    def test_no_queryable_originals_are_offline_without_credentials_sync_or_socket(self):
        with setup() as (directory, batch), owners(batch):
            batch.budget = Budget()
            save(directory, batch, phase='stopped')
            save(directory, batch, sequence=1, phase='source_lost')
            before = bytes_in(directory)
            env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
            args = arguments(directory, batch)
            with patch.dict(os.environ, env, clear=True), \
                 patch.object(socket, 'socket', side_effect=AssertionError('offline')), \
                 patch.object(os, 'fsync', side_effect=AssertionError('offline')):
                code, packet, _ = invoke(args)
            self.assertEqual(code, 0)
            self.assertEqual(packet['result']['clock_recovery']['counts']['unresolved'], 1)
            self.assertFalse(packet['result']['clock_recovery']['directory_history_reconciled'])
            self.assertEqual(packet['result']['clock_recovery']['attempted_count'], 0)
            self.assertEqual(before, bytes_in(directory))

    def test_stale_selection_rejected_before_credentials_or_native_contact(self):
        with setup() as (directory, batch), owners(batch):
            batch.budget = Budget()
            save(directory, batch, phase=None)
            args = arguments(directory, batch)
            save(directory, batch, sequence=1, phase=None)
            before = bytes_in(directory)
            with patch.object(cli.QueryAuthority, 'load', side_effect=AssertionError('no credentials yet')):
                code, packet, out = invoke(args)
            self.assertEqual(code, 2)
            self.assertFalse(packet['ok'])
            self.assertIsNone(packet['result'])
            self.assertEqual(before, bytes_in(directory))
            self.assertEqual(out.writes, 1)

    def test_failed_second_query_returns_first_durable_result_with_exit_two(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer), owners(batch):
            for i in range(3):
                scope, _ = save(directory, batch, sequence=i, phase=None)
                peer.records[scope.clock_key] = (scope.intent(), 'stopped')
            peer.drop_at = 2
            code, packet, out = invoke(arguments(directory, batch))
            self.assertEqual(code, 2)
            self.assertFalse(packet['ok'])
            report = packet['result']['clock_recovery']
            self.assertEqual(report['terminal_records_retained'], 1)
            self.assertEqual(report['attempted_count'], 2)
            self.assertEqual(report['counts']['unresolved'], 2)
            self.assertEqual(packet['agent_turn']['briefing']['stop_reason'], 'query_failed')
            self.assertEqual(len(packet['agent_turn']['changes']), 1)
            self.assertEqual(len(peer.keys), 2)
            self.assertEqual(out.writes, 1)

    def test_lock_and_original_batch_held_through_single_output_attempt(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer), owners(batch):
            scope, _ = save(directory, batch, phase=None)
            peer.records[scope.clock_key] = (scope.intent(), 'stopped')
            def held():
                self.assertFalse(batch.closed)
                fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
                try:
                    with self.assertRaises(BlockingIOError):
                        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                finally:
                    os.close(fd)
            code, _, out = invoke(arguments(directory, batch), Output(on_write=held))
            self.assertEqual(code, 0)
            self.assertTrue(batch.closed)
            self.assertEqual(out.writes, 1)
            fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            finally:
                os.close(fd)

    def test_short_stdout_never_queries_again_or_emits_second_packet(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer), owners(batch):
            scope, path = save(directory, batch, phase=None)
            peer.records[scope.clock_key] = (scope.intent(), 'stopped')
            code, packet, out = invoke(arguments(directory, batch), Output(short=True))
            self.assertEqual(code, 2)
            self.assertTrue(packet['ok'])  # The returned packet was not fully acknowledged.
            self.assertEqual(out.writes, 1)
            self.assertEqual(peer.keys, [scope.clock_key])
            with storage.OutcomeStore(path) as store:
                self.assertIsNotNone(store.load())

    def test_failed_stdout_does_not_attempt_error_output_or_repeat_query(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer), owners(batch):
            scope, _ = save(directory, batch, phase=None)
            peer.records[scope.clock_key] = (scope.intent(), 'stopped')
            out = Output(failure=True)
            with patch.object(cli.sys, 'stdout', out):
                code = cli.main(arguments(directory, batch))
            self.assertEqual(code, 2)
            self.assertEqual(out.writes, 1)
            self.assertEqual(peer.keys, [scope.clock_key])
            self.assertTrue(batch.closed)

    def test_complete_original_packet_reservation_precedes_every_query(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer), owners(batch):
            scope, _ = save(directory, batch, phase=None)
            audit = batch.audit
            with patch.object(batch, 'audit', side_effect=lambda: dict(audit(), original_payload='x' * cli.MAX_OUTPUT)):
                code, packet, _ = invoke(arguments(directory, batch))
            self.assertEqual(code, 2)
            self.assertIsNone(packet['result'])
            self.assertEqual(peer.calls, [])
            self.assertFalse(any(p.name.endswith(storage.SUFFIX) for p in directory.iterdir()))

    def test_invalid_arguments_refuse_before_custody_budget_or_network(self):
        base = ['--directory', '/private/batch', '--batch-id', 'a' * 64,
                '--clock-directory', '/private/clocks', '--expected-inventory', 'b' * 64]
        bad = [base + ['--max-queries', n] for n in ('0', '-1', '17', 'true')]
        bad += [base + ['--timeout-ms', n] for n in ('0', '60001')]
        bad += [base + ['--ticks', '1'], base + ['--wait-ms', '1'], base[:-2],
                base[:-1] + ['bad'], ['--directory', 'relative'] + base[2:]]
        with patch.object(cli, 'make_budget', side_effect=AssertionError('before budget')):
            for args in bad:
                with self.subTest(args=args):
                    code, packet, out = invoke(args)
                    self.assertEqual(code, 2)
                    self.assertFalse(packet['ok'])
                    self.assertEqual(out.writes, 1)

    def test_missing_run_credentials_cannot_trigger_route_credential_fallback(self):
        with setup() as (directory, batch), owners(batch):
            batch.budget = Budget()
            save(directory, batch, phase=None)
            args = arguments(directory, batch)
            env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
            with patch.dict(os.environ, env, clear=True), \
                 patch.object(socket, 'socket', side_effect=AssertionError('no authority')):
                code, packet, _ = invoke(args)
            self.assertEqual(code, 2)
            self.assertIsNone(packet['result'])

    def test_cli_opens_actual_batch_interface_read_only(self):
        observed = []
        sentinel = object()
        def factory(*args, **kwargs):
            observed.append((args, kwargs))
            return sentinel
        module = types.ModuleType('route_furniture_batch_store')
        module.Batch = factory
        budget = object()
        with patch.dict(sys.modules, route_furniture_batch_store=module):
            self.assertIs(cli.open_batch('/original', 'a' * 64, budget), sentinel)
        self.assertEqual(observed, [(('/original', 'a' * 64, budget), {'writable': False})])

    def test_direct_script_entrypoint_executes_same_cli_and_recovery(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer):
            scope, _ = save(directory, batch, phase=None)
            peer.records[scope.clock_key] = (scope.intent(), 'stopped')
            args = arguments(directory, batch)
            opened = []
            route = types.ModuleType('route_furniture_batch')
            route.Budget = Budget
            store = types.ModuleType('route_furniture_batch_store')
            def factory(path, identity, budget, *, writable):
                self.assertFalse(writable)
                self.assertEqual(identity, batch.id)
                batch.budget = budget
                batch.close = lambda: opened.append('closed')
                opened.append(path)
                return batch
            store.Batch = factory
            out = Output()
            with patch.dict(sys.modules, route_furniture_batch=route, route_furniture_batch_store=store), \
                 patch.object(sys, 'argv', [cli.__file__, *args]), patch.object(sys, 'stdout', out):
                with self.assertRaises(SystemExit) as stopped:
                    runpy.run_path(cli.__file__, run_name='__main__')
            self.assertEqual(stopped.exception.code, 0)
            self.assertTrue(json.loads(out.raw)['ok'])
            self.assertEqual(peer.keys, [scope.clock_key])
            self.assertEqual(opened, ['/private/original-route-batch', 'closed'])
            self.assertEqual(out.writes, 1)


if __name__ == '__main__':
    unittest.main()
