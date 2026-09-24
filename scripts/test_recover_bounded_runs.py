"""Recovery inventory/batch tests: actual Python, filesystem and one loopback RPC path."""
from contextlib import redirect_stdout
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import bounded_run_client as c
import bounded_run_outcomes as o
import recover_bounded_runs as r
from test_bounded_run_client import NativeDouble, TOKEN, intent, reference_record


def native(key, phase='stopped'):
    options = {'stopped': (3, 1, 1, 1, 1), 'source_lost': (5, 7, 1, 0, 0), 'running': (1, 0, 1, 0, 1)}[phase]
    raw = reference_record(key=key, phase=options[0], reason=options[1], attempted=options[2],
                           paused=options[3], known=options[4])
    return {'manifest': {'generation': 41, 'df_version': 'fake-df', 'dfhack_version': 'fake-dfhack'},
            'record': c.decode_record(raw), 'owner_active': phase == 'running', 'retained_records': 1}


class FakeQueries:
    def __init__(self, fail_at=None, phase='stopped', after=None):
        self.fail_at, self.phase, self.after = fail_at, phase, after
        self.calls = []; self.opens = []; self.closed = 0

    def factory(self, address, token, deadline):
        self.opens.append((address, token, deadline))
        return self

    def __enter__(self): return self
    def __exit__(self, *_args): self.closed += 1

    def call(self, operation, fields):
        if operation != 'QueryRun':
            raise AssertionError('recovery dispatched a non-query operation')
        self.calls.append(fields[5].decode())
        if len(self.calls) == self.fail_at:
            raise OSError('lost query response')
        if self.after:
            self.after()
        result = native(self.calls[-1], self.phase)
        assert result['record']['plan_digest_hex'] == fields[9].hex()
        return result


class InventoryTests(unittest.TestCase):
    def setUp(self):
        self.directory = Path(tempfile.mkdtemp(prefix='dfmcp-run-inventory-'))
        self.directory.chmod(0o700)
        self.env = {'DFMCP_ALLOW_UNADMITTED_RUN_V1_13': '1', 'DFMCP_RUN_TOKEN': TOKEN.decode()}
        self.addCleanup(patch.stopall)
        patch.dict(os.environ, self.env, clear=True).start()

    def add(self, name, phase=None, key=None, address=('127.0.0.1', 5000)):
        key = key or name
        value = intent(address)
        value['idempotency_key'] = key
        value['prepare_token_hex'] = c.token_for(key, bytes.fromhex(value['plan_digest_hex'])).hex()
        path = self.directory / name
        with c.capsule(path, value):
            pass
        if phase:
            o.retain(path, native(key, phase))
        return path

    def page(self, **kwargs):
        with r.Inventory(self.directory) as inventory:
            return inventory.page(**kwargs)

    def test_pagination_retains_complete_pending_inventory_counts(self):
        for index in range(20):
            self.add(f'run-{index:02}', 'stopped' if index < 18 else None)
        first = self.page(limit=1)
        self.assertEqual(first['counts']['unresolved'], 2)
        self.assertEqual(first['counts']['pending_names_preview'], ['run-18', 'run-19'])
        self.assertEqual(first['rows'][0]['name'], 'run-00')
        second = self.page(limit=1, continuation=first['continuation'])
        self.assertEqual(second['rows'][0]['name'], 'run-01')
        pending = self.page(selection='pending')
        self.assertEqual([row['name'] for row in pending['rows']], ['run-18', 'run-19'])
        self.assertFalse(pending['native_contacted'])
        self.assertFalse(pending['retry_permitted'])

    def test_continuations_fence_root_selection_limit_and_corruption(self):
        self.add('a'); self.add('b')
        first = self.page(limit=1)
        for kwargs in ({'limit': 2}, {'limit': 1, 'selection': 'pending'}):
            with self.assertRaises(ValueError):
                self.page(continuation=first['continuation'], **kwargs)
        token = first['continuation']
        with self.assertRaises(ValueError):
            self.page(limit=1, continuation=token[:-1] + ('0' if token[-1] != '0' else '1'))
        o.retain(self.directory / 'a', native('a'))
        with self.assertRaises(ValueError): self.page(limit=1, continuation=token)

    def test_source_loss_is_pending_operator_work_not_a_verified_stop(self):
        self.add('lost', 'source_lost'); self.add('finished', 'stopped'); self.add('pending')
        packet = self.page(selection='pending')
        self.assertEqual(packet['counts']['operator_attention'], 1)
        self.assertEqual(packet['counts']['unresolved'], 2)
        self.assertEqual(packet['counts']['query_required'], 1)
        self.assertEqual(packet['matched'], 2)
        fake = FakeQueries()
        done = r.reconcile(self.directory, packet['inventory_digest'], factory=fake.factory)
        self.assertEqual(fake.calls, ['pending'])
        self.assertEqual(done['counts']['operator_attention'], 1)
        self.assertEqual(done['counts']['unresolved'], 1)

    def test_stale_inventory_and_conflicting_duplicate_keys_prevent_native_work(self):
        self.add('first', key='same')
        old = self.page()['inventory_digest']
        self.add('second')
        fake = FakeQueries()
        with self.assertRaises(ValueError): r.reconcile(self.directory, old, factory=fake.factory)
        self.assertEqual(fake.opens, [])
        self.add('duplicate', key='same')
        with self.assertRaises(ValueError): self.page()

    def test_bounded_pass_uses_one_connection_and_one_query_per_selected_key(self):
        for index in range(5): self.add(f'run-{index}')
        before = self.page()
        fake = FakeQueries()
        done = r.reconcile(self.directory, before['inventory_digest'], maximum=2, factory=fake.factory)
        self.assertTrue(done['ok'])
        self.assertEqual(fake.calls, ['run-0', 'run-1'])
        self.assertEqual(len(fake.opens), 1); self.assertEqual(fake.closed, 1)
        self.assertEqual(done['not_selected_count'], 3)
        self.assertEqual(done['counts']['unresolved'], 3)
        self.assertNotEqual(done['inventory_after'], before['inventory_digest'])
        self.assertTrue(all(row['storage_acknowledged_this_call'] for row in done['processed']))

    def test_partial_failure_preserves_earlier_receipts_and_defers_remaining_work(self):
        for name in ('a', 'b', 'c'): self.add(name)
        fake = FakeQueries(fail_at=2)
        done = r.reconcile(self.directory, self.page()['inventory_digest'], factory=fake.factory)
        self.assertFalse(done['ok'])
        self.assertEqual(fake.calls, ['a', 'b'])
        self.assertEqual(done['deferred'], ['b', 'c'])
        self.assertEqual(done['failed_name'], 'b')
        self.assertEqual(done['counts']['unresolved'], 2)
        self.assertTrue(o.inspect(self.directory / 'a')['terminal_record_retained'])
        self.assertFalse(Path(str(self.directory / 'b') + o.SUFFIX).exists())

    def test_one_shrinking_deadline_stops_later_queries_without_renewal(self):
        for name in ('a', 'b', 'c'): self.add(name)
        now = [100.0]
        def advance(): now[0] += 2
        fake = FakeQueries(after=advance)
        done = r.reconcile(self.directory, self.page()['inventory_digest'], timeout_ms=1000,
                           factory=fake.factory, clock=lambda: now[0])
        self.assertFalse(done['ok']); self.assertEqual(fake.calls, ['a'])
        self.assertEqual(done['deferred'], ['b', 'c'])
        self.assertLess(fake.opens[0][2], 101)
        self.assertTrue(o.inspect(self.directory / 'a')['terminal_record_retained'])

    def test_authority_change_after_connect_prevents_query_dispatch(self):
        self.add('a')
        fake = FakeQueries()
        def factory(*args):
            os.environ['DFMCP_ADMITTED_BRIDGE_PROTOCOL'] = '1.0'
            return fake.factory(*args)
        done = r.reconcile(self.directory, self.page()['inventory_digest'], factory=factory)
        self.assertFalse(done['ok']); self.assertEqual(fake.calls, [])
        self.assertEqual(fake.closed, 1)

    def test_oversized_output_is_refused_before_native_work(self):
        self.add('a')
        root = self.page()['inventory_digest']
        fake = FakeQueries()
        with patch.object(r, 'MAX_OUTPUT', 100), self.assertRaises(ValueError):
            r.reconcile(self.directory, root, factory=fake.factory)
        self.assertEqual(fake.opens, [])
        self.assertFalse(Path(str(self.directory / 'a') + o.SUFFIX).exists())

    def test_native_transport_rejects_all_effect_and_observation_methods(self):
        client = object.__new__(r.QueryOnlyClient)
        for operation in ('ObserveRun', 'PrepareRun', 'CommitRun', 'CancelRun', 'arbitrary'):
            with self.subTest(operation=operation), self.assertRaises(ValueError):
                client.call(operation)

    def test_orphan_corruption_special_files_and_membership_drift_are_refused(self):
        self.add('a')
        with self.assertRaises(ValueError):
            with r.Inventory(self.directory):
                self.add('extra')  # The low-level legacy writer does not hold the directory lock.
        orphan = self.directory / ('orphan' + o.SUFFIX)
        orphan.write_bytes(b'{}\n'); orphan.chmod(0o600)
        with self.assertRaises(ValueError): self.page()
        other = Path(tempfile.mkdtemp(prefix='dfmcp-run-fifo-')); other.chmod(0o700)
        os.mkfifo(other / 'intent', 0o600)
        with self.assertRaises(ValueError), r.Inventory(other): pass

    def test_maximum_names_and_native_keys_fit_output_and_survive_sidecars(self):
        for index in range(64):
            self.add(f'{index:03}-' + 'n' * 188, 'stopped', key=f'{index:03}' + 'k' * 125)
        page = self.page(limit=64)
        self.assertEqual(len(page['rows']), 64)
        self.assertLessEqual(len(r.bounded(page)), r.MAX_OUTPUT)
        fake = FakeQueries()
        done = r.reconcile(self.directory, page['inventory_digest'], factory=fake.factory)
        self.assertTrue(done['ok']); self.assertEqual(fake.opens, [])

    def test_entry_bound_refuses_without_scanning_unbounded_content(self):
        for index in range(257):
            path = self.directory / f'{index:03}'
            path.write_bytes(b''); path.chmod(0o600)
        with self.assertRaises(ValueError): self.page()

    def test_real_loopback_query_persists_receipt_then_offline_inventory(self):
        with NativeDouble() as server:
            self.add('test.json', key='test', address=server.address)
            os.environ['DFMCP_RUN_ENDPOINT'] = f'{server.address[0]}:{server.address[1]}'
            root = self.page()['inventory_digest']
            output = io.StringIO()
            with redirect_stdout(output):
                status = r.main(['reconcile', '--directory', str(self.directory), '--expected-inventory', root])
            self.assertEqual(status, 0, output.getvalue())
            self.assertEqual(server.calls, ['Handshake', 'QueryRun'])
            self.assertEqual(server.effects, 0)
        with patch.dict(os.environ, {}, clear=True), patch.object(r, 'QueryOnlyClient', side_effect=AssertionError('offline connect')):
            output = io.StringIO()
            with redirect_stdout(output): self.assertEqual(r.main(['inventory', '--directory', str(self.directory)]), 0)
            packet = json.loads(output.getvalue())
        self.assertEqual(packet['counts']['query_required'], 0)
        self.assertEqual(packet['counts']['terminal_resolved'], 1)
        self.assertNotIn(TOKEN.decode(), output.getvalue())


if __name__ == '__main__':
    unittest.main(verbosity=2)
