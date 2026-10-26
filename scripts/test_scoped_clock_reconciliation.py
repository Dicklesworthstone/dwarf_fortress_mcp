"""Actual clock TCP, strict scope/native codecs and private files for recovery.

Only original route-batch custody/budget are explicit ports. Native responses are
synthetic; no real fortress, private route-batch or Rust qualification is claimed.
"""
from contextlib import contextmanager
from dataclasses import replace
import json
import os
from pathlib import Path
import socket
import struct
import unittest
from unittest.mock import patch

import bounded_run_client as clock
import bounded_run_outcomes as storage
from furniture_plan import canonical
from route_furniture_clock_context import (
    Client, FinalScope, FINAL_POLICY, FINAL_SCHEMA, ScopeStore,
)
from route_furniture_clock_inventory import Inventory
import scoped_clock_recovery as recovery
from test_route_furniture_clock_inventory import Batch, save, scope_for, bytes_in, setup
from test_route_furniture_clock_inventory_workflow import (
    Budget, Peer, encode_fields, decode_fields, read_exact, native_record,
)


class QueryPeer(Peer):
    """Independently framed peer refuses EVERY effect or observation call."""
    def __init__(self):
        super().__init__()
        self.keys = []
        self.before_reply = lambda key, intent: None
        self.drop_at = None
        self.foreign_at = None
        self.active = False

    def handle(self, connection):
        assert read_exact(connection, 12) == b'DFHack?\n' + struct.pack('<i', 1)
        connection.sendall(b'DFHack!\n' + struct.pack('<i', 1))
        bound = []
        while not self.done.is_set():
            method, length = struct.unpack('<h2xi', read_exact(connection, 8))
            assert 0 <= length <= 2048
            fields = decode_fields(read_exact(connection, length))
            if method == 0:
                name = fields[1].decode('ascii')
                assert name == self.methods[len(bound)]
                assert fields[2] == b'dfmcp.run.v1_13.Request'
                assert fields[3] == b'dfmcp.run.v1_13.Reply' and fields[4] == b'dfmcp_run_v1_13'
                bound.append(name)
                payload = encode_fields({1: self.methods.index(name) + 2})
            else:
                assert tuple(bound) == self.methods
                name = self.methods[method - 2]
                assert name in ('Handshake', 'QueryRun'), 'recovery attempted a non-query RPC'
                assert fields[1] == b'r' * 32 and len(fields[2]) == 32
                assert fields[3] == 1 and fields[4] == 13
                self.calls.append(name)
                reply = {1: 1, 2: 0, 3: fields[2], 4: 1, 5: 13,
                         6: 11, 7: b'df', 8: b'dfhack', 11: int(self.active), 12: len(self.records)}
                if name == 'QueryRun':
                    assert set(fields) == {1, 2, 3, 4, 5, 9}
                    key = fields[5].decode('ascii')
                    self.keys.append(key)
                    if self.drop_at == len(self.keys):
                        return
                    intent, phase = self.records[key]
                    assert fields[9].hex() == intent['plan_digest_hex']
                    self.before_reply(key, intent)
                    if phase is not None:
                        supplied = intent
                        if self.foreign_at == len(self.keys):
                            supplied = dict(intent, idempotency_key='foreign')
                        reply[10] = native_record(supplied, phase)
                payload = encode_fields(reply)
            connection.sendall(struct.pack('<h2xi', -1, len(payload)) + payload)


@contextmanager
def configured(batch, peer):
    # Rewrite the fixture's original immutable source before creating any scopes.
    batch.address = peer.address
    batch.source = replace(batch.source, endpoint=f'{peer.address[0]}:{peer.address[1]}')
    batch.budget = Budget()
    environment = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
    environment.update(DFMCP_ALLOW_UNADMITTED_RUN_V1_13='1',
                       DFMCP_RUN_ENDPOINT=f'{peer.address[0]}:{peer.address[1]}',
                       DFMCP_RUN_TOKEN='r' * 32)
    with patch.dict(os.environ, environment, clear=True):
        yield recovery.QueryAuthority.load()


def final_save(directory, batch, sequence):
    value = scope_for(batch, sequence).value()
    value.pop('next_step')
    value.update(schema=FINAL_SCHEMA, policy=FINAL_POLICY, phase='final_construction')
    scope = FinalScope.create(value)
    path = directory / ('progress-' + scope.digest + '.run')
    audit = batch.audit
    with patch.object(batch, 'audit', side_effect=lambda: dict(audit(), status='all_placed',
            next_step=None, pending_step=None, stopped=False, placed=len(batch.plan.steps))):
        with storage.new_run(path) as parent:
            with ScopeStore(path, parent, batch, scope.digest, create=scope):
                pass
            with clock.capsule(path, scope.intent()):
                pass
    return scope, path


def expected(directory, batch):
    with Inventory(directory, batch) as history:
        return history.digest


def originals(directory):
    return {name: raw for name, raw in bytes_in(directory).items() if not name.endswith(storage.SUFFIX)}


class RecoveryTests(unittest.TestCase):
    def test_mixed_original_scope_families_recover_over_actual_tcp_without_effects(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer) as authority:
            one, _ = save(directory, batch, sequence=0, phase=None)
            two, _ = final_save(directory, batch, 1)
            for scope in (one, two):
                peer.records[scope.clock_key] = (scope.intent(), 'stopped')
            before = originals(directory)
            old = expected(directory, batch)
            with recovery.Recovery(directory, batch) as owner:
                result = owner.reconcile(old, authority)
                self.assertEqual(result['terminal_records_retained'], 2)
                self.assertEqual(result['counts']['unresolved'], 0)
                self.assertNotEqual(old, result['inventory_digest'])
                self.assertEqual(result['inventory_digest'], owner.digest)
            self.assertEqual(set(peer.keys), {one.clock_key, two.clock_key})
            self.assertEqual(peer.calls.count('QueryRun'), 2)
            self.assertEqual(before, originals(directory))
            self.assertEqual(result['inventory_digest'], expected(directory, batch))
            self.assertFalse(result['clock_start_authorized'])
            self.assertFalse(result['construction_completion_proven'])
            self.assertTrue(result['current_pause_unproved'])

    def test_stale_inventory_refuses_before_any_socket_or_new_outcome(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer) as authority:
            save(directory, batch, phase=None)
            root = expected(directory, batch)
            save(directory, batch, sequence=1, phase=None)
            before = bytes_in(directory)
            with recovery.Recovery(directory, batch) as owner, \
                 patch.object(socket, 'socket', side_effect=AssertionError('no native contact')):
                with self.assertRaises(ValueError):
                    owner.reconcile(root, authority)
            self.assertEqual(before, bytes_in(directory))

    def test_exact_limits_through_sixteen_queries_and_no_quota_reset(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer) as authority:
            pairs = [save(directory, batch, sequence=i, phase=None) for i in range(17)]
            for scope, _ in pairs:
                peer.records[scope.clock_key] = (scope.intent(), 'refused')
            budget, deadline = batch.budget, batch.budget.deadline
            with recovery.Recovery(directory, batch) as owner:
                result = owner.reconcile(owner.digest, authority, maximum=16)
                self.assertEqual(result['stop_reason'], 'query_limit')
                self.assertEqual(result['attempted_count'], 16)
                self.assertEqual(result['counts']['query_supported'], 1)
                self.assertEqual(budget.calls, 560 - 16 * recovery.QUERY_CALLS)
                self.assertIs(budget, batch.budget)
                self.assertEqual(deadline, budget.deadline)
            self.assertEqual(len(peer.keys), 16)
            self.assertEqual(len(set(peer.keys)), 16)

    def test_absent_prepared_running_and_source_lost_remain_unresolved(self):
        for phase in (None, 'prepared', 'running', 'source_lost'):
            with self.subTest(phase=phase), setup() as (directory, batch), QueryPeer() as peer, \
                 configured(batch, peer) as authority:
                scope, path = save(directory, batch, phase=None)
                peer.records[scope.clock_key] = (scope.intent(), phase)
                peer.active = phase == 'running'
                with recovery.Recovery(directory, batch) as owner:
                    result = owner.reconcile(owner.digest, authority)
                    self.assertFalse(result['faulted'])
                    self.assertEqual(result['counts']['unresolved'], 1)
                    self.assertFalse(result['directory_history_reconciled'])
                    self.assertEqual(result['terminal_records_retained'], int(phase == 'source_lost'))
                with Inventory(directory, batch) as owner:
                    with self.assertRaises(ValueError):
                        owner.admit_new(scope_for(batch, 99))
                self.assertEqual(len(peer.keys), 1)

    def test_foreign_unbound_scope_only_and_terminal_are_not_queried(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer) as authority:
            save(directory, batch, sequence=0, phase='stopped')
            save(directory, batch, sequence=1, phase='source_lost')
            save(directory, batch, sequence=2, with_intent=False)
            save(directory, batch, sequence=3, with_scope=False, phase=None)
            other = Batch(identity='e' * 64)
            save(directory, other, sequence=4, phase=None)
            before = bytes_in(directory)
            with recovery.Recovery(directory, batch) as owner, \
                 patch.object(socket, 'socket', side_effect=AssertionError('offline terminal pass')):
                result = owner.reconcile(owner.digest, None)
            self.assertEqual(result['attempted_count'], 0)
            self.assertEqual(result['counts']['unresolved'], 4)
            self.assertEqual(result['counts']['other_batch'], 1)
            self.assertFalse(result['directory_history_reconciled'])
            self.assertEqual(before, bytes_in(directory))

    def test_failed_second_query_retains_first_and_does_not_retry_or_query_third(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer) as authority:
            pairs = [save(directory, batch, sequence=i, phase=None) for i in range(3)]
            for scope, _ in pairs:
                peer.records[scope.clock_key] = (scope.intent(), 'stopped')
            peer.drop_at = 2
            with recovery.Recovery(directory, batch) as owner:
                result = owner.reconcile(owner.digest, authority)
            self.assertTrue(result['faulted'])
            self.assertEqual(result['stop_reason'], 'query_failed')
            self.assertEqual(result['terminal_records_retained'], 1)
            self.assertEqual(result['attempted_count'], 2)
            self.assertEqual(result['counts']['unresolved'], 2)
            self.assertEqual(len(peer.keys), 2)
            self.assertEqual(len(set(peer.keys)), 2)
            with Inventory(directory, batch) as owner:
                self.assertEqual(owner.counts()['query_supported'], 2)

    def test_wrong_native_record_is_not_retained_or_retried(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer) as authority:
            scope, path = save(directory, batch, phase=None)
            peer.records[scope.clock_key] = (scope.intent(), 'stopped')
            peer.foreign_at = 1
            with recovery.Recovery(directory, batch) as owner:
                result = owner.reconcile(owner.digest, authority)
            self.assertTrue(result['faulted'])
            self.assertEqual(result['terminal_records_retained'], 0)
            self.assertFalse(Path(str(path) + storage.SUFFIX).exists())
            self.assertEqual(peer.keys, [scope.clock_key])

    def test_complete_result_reserved_before_contact(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer) as authority:
            save(directory, batch, phase=None)
            before = bytes_in(directory)
            def render(_):
                raise ValueError('output cannot fit')
            with recovery.Recovery(directory, batch) as owner, \
                 patch.object(socket, 'socket', side_effect=AssertionError('no contact')):
                with self.assertRaises(ValueError):
                    owner.reconcile(owner.digest, authority, render=render)
            self.assertEqual(before, bytes_in(directory))

    def test_post_retention_output_failure_does_not_repeat_original_query(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer) as authority:
            scope, path = save(directory, batch, phase=None)
            peer.records[scope.clock_key] = (scope.intent(), 'stopped')
            def render(report):
                if 'reserved_outcomes' not in report:
                    raise ValueError('output revoked')
            with self.assertRaises(ValueError), recovery.Recovery(directory, batch) as owner:
                owner.reconcile(owner.digest, authority, render=render)
            self.assertEqual(peer.keys, [scope.clock_key])
            with storage.OutcomeStore(path) as store:
                self.assertEqual(store.load()['record']['phase'], 'stopped')

    def test_outcome_sync_failure_fences_owner_and_preserves_ambiguous_bytes(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer) as authority:
            scope, path = save(directory, batch, phase=None)
            peer.records[scope.clock_key] = (scope.intent(), 'stopped')
            owner = recovery.Recovery(directory, batch).__enter__()
            try:
                with patch.object(os, 'fsync', side_effect=OSError('sync failed')):
                    with self.assertRaises(OSError):
                        owner.reconcile(owner.digest, authority)
                self.assertTrue(owner.fenced)
                with self.assertRaises(ValueError):
                    owner.check()
            finally:
                owner.close()
            self.assertEqual(peer.keys, [scope.clock_key])
            self.assertTrue(Path(str(path) + storage.SUFFIX).exists())
            self.assertTrue(path.exists())

    def test_byte_identical_scope_replacement_during_query_refuses_publication(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer) as authority:
            scope, path = save(directory, batch, phase=None)
            peer.records[scope.clock_key] = (scope.intent(), 'stopped')
            def replace_scope(*_):
                original = Path(str(path) + '.scope.json')
                replacement = directory / 'replacement'
                replacement.write_bytes(original.read_bytes())
                replacement.chmod(0o600)
                replacement.replace(original)
            peer.before_reply = replace_scope
            with self.assertRaises(ValueError), recovery.Recovery(directory, batch) as owner:
                owner.reconcile(owner.digest, authority)
            self.assertFalse(Path(str(path) + storage.SUFFIX).exists())
            self.assertEqual(peer.keys, [scope.clock_key])

    def test_unrelated_membership_change_during_query_is_not_adopted(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer) as authority:
            scope, path = save(directory, batch, phase=None)
            peer.records[scope.clock_key] = (scope.intent(), 'stopped')
            peer.before_reply = lambda *_: (directory / 'foreign').write_text('changed')
            with self.assertRaises(ValueError), recovery.Recovery(directory, batch) as owner:
                owner.reconcile(owner.digest, authority)
            self.assertFalse(Path(str(path) + storage.SUFFIX).exists())

    def test_revoked_authority_after_native_reply_never_retains_a_result(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer) as authority:
            scope, path = save(directory, batch, phase=None)
            peer.records[scope.clock_key] = (scope.intent(), 'stopped')
            peer.before_reply = lambda *_: os.environ.__setitem__('DFMCP_RUN_TOKEN', 'z' * 32)
            with self.assertRaises(ValueError), recovery.Recovery(directory, batch) as owner:
                owner.reconcile(owner.digest, authority)
            self.assertFalse(Path(str(path) + storage.SUFFIX).exists())

    def test_budget_boundary_is_not_replenished_or_an_unknown_resolution(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer) as authority:
            save(directory, batch, phase=None)
            batch.budget.calls = recovery.QUERY_CALLS - 1
            with recovery.Recovery(directory, batch) as owner:
                result = owner.reconcile(owner.digest, authority)
            self.assertEqual(result['stop_reason'], 'aggregate_budget_boundary')
            self.assertEqual(result['attempted_count'], 0)
            self.assertEqual(result['counts']['unresolved'], 1)
            self.assertFalse(peer.calls)
            self.assertEqual(batch.budget.calls, recovery.QUERY_CALLS - 1)

    def test_new_effect_is_never_admitted_by_recovery_owner(self):
        with setup() as (directory, batch), recovery.Recovery(directory, batch) as owner:
            with self.assertRaises(ValueError):
                owner.admit_new(scope_for(batch))

    def test_owner_cannot_reconcile_twice_or_renew_limits(self):
        with setup() as (directory, batch), recovery.Recovery(directory, batch) as owner:
            root = owner.digest
            owner.reconcile(root, None)
            with self.assertRaises(ValueError):
                owner.reconcile(root, None)

    def test_bad_limits_and_selected_endpoint_refuse_before_native_calls(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer) as authority:
            save(directory, batch, phase=None)
            for maximum in (True, 0, 17, -1, '1'):
                with recovery.Recovery(directory, batch) as owner:
                    with self.assertRaises(ValueError):
                        owner.reconcile(owner.digest, authority, maximum=maximum)
            with recovery.Recovery(directory, batch) as owner:
                with self.assertRaises(ValueError):
                    owner.reconcile(owner.digest, replace(authority, address=('127.0.0.1', 1)))
            self.assertEqual(peer.calls, [])

    def test_query_transport_refuses_every_non_query_method(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer) as authority:
            scope = scope_for(batch)
            for method in ('ObserveRun', 'PrepareRun', 'CommitRun', 'CancelRun'):
                with Client(authority, batch.budget, batch.check, intent=scope.intent(), query=True) as client:
                    with self.assertRaises(ValueError):
                        client.call(method)
            self.assertEqual(peer.calls, ['Handshake'] * 4)

    def test_run_only_configuration_needs_no_route_or_mutation_credentials(self):
        with setup() as (directory, batch), QueryPeer() as peer, configured(batch, peer) as authority:
            self.assertNotIn('DFMCP_MAP_TOKEN', os.environ)
            self.assertNotIn('DFMCP_BUILD_TOKEN', os.environ)
            self.assertNotIn('DFMCP_OPERATIONS_PAGED_TOKEN', os.environ)
            self.assertNotIn('r' * 32, repr(authority))
            for name in ('DFMCP_ROUTE_FURNITURE_ALLOW_PLACE', 'DFMCP_ADMITTED_BRIDGE_PROTOCOL',
                         'DFMCP_MAP_TOKEN'):
                with patch.dict(os.environ, {name: '1'}):
                    with self.assertRaises(ValueError):
                        recovery.QueryAuthority.load()


if __name__ == '__main__':
    unittest.main()
