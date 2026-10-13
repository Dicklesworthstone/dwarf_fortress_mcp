"""Actual query transport, journal and CLI with a joined independent TCP peer.

The peer supplies evidence fixtures, not a DFHack game or attribution proof.
"""
from contextlib import contextmanager
from dataclasses import replace
import io
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

import build_placement_client as app
import build_placement_rpc as rpc
import build_placement_store as store
from build_placement_wire import Plan, Record
from test_build_placement_reconciliation import fixture, reply

TOKEN = 'r' * 48
ROOT = Path(__file__).resolve().parents[1]


def number(n):
    raw = bytearray()
    while n > 127:
        raw.append((n & 127) | 128)
        n >>= 7
    raw.append(n)
    return bytes(raw)


def encode(fields):
    raw = bytearray()
    for key, value in sorted(fields.items()):
        raw += number(key * 8 + (2 if isinstance(value, bytes) else 0))
        raw += number(len(value)) + value if isinstance(value, bytes) else number(value)
    return bytes(raw)


def decode(raw):
    pos, fields = 0, {}
    def read():
        nonlocal pos
        out, shift = 0, 0
        while True:
            value = raw[pos]
            pos += 1
            out |= (value & 127) << shift
            if value < 128:
                return out
            shift += 7
    while pos < len(raw):
        tag = read()
        assert tag >> 3 not in fields
        if tag & 7 == 2:
            size = read()
            value = raw[pos:pos + size]
            pos += size
        else:
            assert tag & 7 == 0
            value = read()
        fields[tag >> 3] = value
    return fields


class Peer:
    def __init__(self, plan, manifest, records, fault=None):
        self.plan, self.manifest, self.records, self.fault = plan, manifest, records, fault
        self.calls, self.errors, self.connection = [], [], None
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(1)
        self.listener.settimeout(5)
        self.address = self.listener.getsockname()
        self.thread = threading.Thread(target=self.serve)
        self.thread.start()

    def serve(self):
        try:
            self.connection, _ = self.listener.accept()
            with self.connection as sock:
                sock.settimeout(5)
                def take(count):
                    raw = bytearray()
                    while len(raw) < count:
                        part = sock.recv(count - len(raw))
                        if not part:
                            raise EOFError
                        raw += part
                    return bytes(raw)
                def frame():
                    method, width = struct.unpack('<h2xi', take(8))
                    assert 0 <= width <= 2048
                    return method, decode(take(width))
                def send(fields):
                    raw = encode(fields)
                    sock.sendall(struct.pack('<h2xi', -1, len(raw)) + raw)
                assert take(12) == b'DFHack?\n' + struct.pack('<i', 1)
                sock.sendall(b'DFHack!\n' + struct.pack('<i', 1))
                names = ('Handshake', 'ReadPlacement', 'PreparePlacement', 'CommitPlacement',
                         'QueryPlacement', 'CancelPlacement')
                for identity, name in enumerate(names, 2):
                    method, fields = frame()
                    assert method == 0
                    assert fields == {1: name.encode(), 2: b'dfmcp.build.v1_19.Request',
                                      3: b'dfmcp.build.v1_19.Reply', 4: b'dfmcp_build_v1_19'}
                    send({1: identity})
                method, fields = frame()
                assert method == 2
                self.calls.append('Handshake')
                nonce = fields[2]
                assert len(nonce) == 32
                base = {1: TOKEN.encode(), 2: nonce, 3: 1, 4: 19}
                assert fields == base
                def response(record=None):
                    out = {1: 1, 2: 0, 3: nonce, 4: 1, 5: 19,
                           6: self.manifest.generation, 7: self.manifest.df_version.encode(),
                           8: self.manifest.dfhack_version.encode(), 12: 1, 13: 1}
                    if record is not None:
                        out[10] = record.raw
                        out[12] = int(record.phase == 'indeterminate')
                    return out
                send(response())
                for record in self.records:
                    method, fields = frame()
                    assert method == 6, 'recovery attempted a non-query native operation'
                    self.calls.append('QueryPlacement')
                    assert fields == {**base, 10: self.plan.key.encode(), 12: self.plan.digest}
                    if self.fault == 'lost_reply':
                        return
                    out = response(record)
                    if self.fault == 'nonce':
                        out[3] = b'x' * 32
                    elif self.fault == 'generation':
                        out[6] += 1
                    elif self.fault == 'checksum':
                        out[10] = out[10][:-1] + bytes([out[10][-1] ^ 1])
                    send(out)
                # Any unplanned subsequent method (especially mutation) is a test failure.
                assert sock.recv(1) == b'', 'unexpected native traffic after recovery'
        except EOFError:
            pass  # Client refused or closed after a deliberate protocol failure.
        except BaseException as error:
            self.errors.append(error)

    def close(self):
        self.thread.join(7)
        self.listener.close()
        if self.thread.is_alive():
            if self.connection is not None:
                self.connection.close()
            self.thread.join(2)
        assert not self.thread.is_alive(), 'peer did not quiesce'
        if self.errors:
            raise self.errors[0]


@contextmanager
def peer(plan, manifest, records, fault=None):
    p = Peer(plan, manifest, records, fault)
    env = {key: value for key, value in os.environ.items() if not key.startswith('DFMCP_')}
    env.update(DFMCP_ALLOW_UNADMITTED_BUILD_V1_19='1', DFMCP_BUILD_TOKEN=TOKEN,
               DFMCP_BUILD_ENDPOINT=f'{p.address[0]}:{p.address[1]}')
    try:
        with patch.dict(os.environ, env, clear=True):
            yield p
    finally:
        p.close()


def initialize(path, plan, uncertain, manifest, address):
    with store.PlacementDirectory(path, rpc.Budget(60000), True) as owner:
        journal = owner.create(plan, manifest, address)
        journal.retain(reply(Record(plan, 'prepared', 'none'), manifest), prepared=True)
        journal.append('dispatch', {'plan_digest': plan.digest.hex()})
        journal.retain(reply(uncertain, manifest))
        return journal.raw


class RuntimeRecovery(unittest.TestCase):
    def test_real_query_resolves_original_without_prepare_read_or_commit(self):
        for kind in (1, 2, 3):
            plan, uncertain, placed, manifest = fixture(kind)
            with peer(plan, manifest, [placed]) as server, tempfile.TemporaryDirectory() as path:
                old = initialize(path, plan, uncertain, manifest, server.address)
                with store.PlacementDirectory(path, rpc.Budget(60000), True) as owner:
                    journal = owner.get(plan.key)
                    out = app.recover(journal, rpc.Authority.load())
                    self.assertEqual(out['effect_status'], 'placed')
                    self.assertFalse(out['pending'] or out['retry_permitted'])
                    self.assertTrue(out['storage_acknowledged_this_call'])
                    self.assertTrue(journal.raw.startswith(old))
                    self.assertEqual(journal.state.reconciled_from, uncertain)
                self.assertEqual(server.calls, ['Handshake', 'QueryPlacement'])
                with patch.object(app, 'Client', side_effect=AssertionError('offline expected')):
                    with store.PlacementDirectory(path, rpc.Budget(60000)) as owner:
                        out = app.recover(owner.get(plan.key), None)
                        self.assertEqual(out['effect_status'], 'placed')
                        self.assertFalse(out['native_contacted'])

    def test_same_connection_can_observe_resolution_but_never_gain_a_permit(self):
        plan, uncertain, placed, manifest = fixture()
        with peer(plan, manifest, [uncertain, placed, placed]) as server:
            with rpc.Client(rpc.Authority.load(), rpc.Budget(60000), plan.before.selection) as client:
                self.assertEqual(client.query(plan).record, uncertain)
                self.assertEqual(client.query(plan).record, placed)
                self.assertEqual(client.query(plan).record, placed)
                self.assertIsNone(client._permit)
                for invoke in (client.commit, client.prepare):
                    with self.assertRaises(ValueError):
                        invoke(plan)
            self.assertEqual(server.calls, ['Handshake'] + ['QueryPlacement'] * 3)

    def test_uncertain_or_absent_native_record_keeps_original_pending(self):
        plan, uncertain, _, manifest = fixture()
        for record in (uncertain, None):
            with peer(plan, manifest, [record]), tempfile.TemporaryDirectory() as path:
                address = rpc.Authority.load().address
                old = initialize(path, plan, uncertain, manifest, address)
                with store.PlacementDirectory(path, rpc.Budget(60000), True) as owner:
                    journal = owner.get(plan.key)
                    out = app.recover(journal, rpc.Authority.load())
                    self.assertTrue(out['pending'])
                    self.assertFalse(out['storage_acknowledged_this_call'])
                    self.assertEqual(journal.raw, old)
                    with self.assertRaises(ValueError):
                        owner.ready('another')

    def test_lost_reply_and_malformed_proof_do_not_change_durable_history(self):
        plan, uncertain, placed, manifest = fixture()
        for fault in ('lost_reply', 'nonce', 'generation', 'checksum'):
            with peer(plan, manifest, [placed], fault) as server, tempfile.TemporaryDirectory() as path:
                old = initialize(path, plan, uncertain, manifest, server.address)
                with store.PlacementDirectory(path, rpc.Budget(60000), True) as owner:
                    journal = owner.get(plan.key)
                    with self.assertRaises((ValueError, OSError)):
                        app.recover(journal, rpc.Authority.load())
                    self.assertEqual(journal.raw, old)
                    self.assertTrue(journal.state.pending)
                self.assertEqual(server.calls, ['Handshake', 'QueryPlacement'])

    def test_cancel_and_refusal_cannot_resolve_an_attempted_placement(self):
        plan, uncertain, _, manifest = fixture()
        for record in (Record(plan, 'cancelled', 'cancelled'), Record(plan, 'refused', 'stale'),
                       Record(plan, 'prepared', 'none')):
            with peer(plan, manifest, [record]) as server, tempfile.TemporaryDirectory() as path:
                old = initialize(path, plan, uncertain, manifest, server.address)
                with store.PlacementDirectory(path, rpc.Budget(60000), True) as owner:
                    journal = owner.get(plan.key)
                    with self.assertRaises(ValueError):
                        app.recover(journal, rpc.Authority.load())
                    self.assertEqual(journal.raw, old)

    def test_new_incarnation_cannot_supply_original_reconciliation(self):
        plan, uncertain, placed, manifest = fixture()
        with peer(plan, replace(manifest, generation=8), [placed]) as server, tempfile.TemporaryDirectory() as path:
            old = initialize(path, plan, uncertain, manifest, server.address)
            with store.PlacementDirectory(path, rpc.Budget(60000), True) as owner:
                journal = owner.get(plan.key)
                with self.assertRaises(ValueError):
                    app.recover(journal, rpc.Authority.load())
                self.assertEqual(journal.raw, old)

    def test_cancel_of_retained_uncertainty_and_resolved_inspection_are_offline(self):
        plan, uncertain, placed, manifest = fixture()
        with tempfile.TemporaryDirectory() as path:
            initialize(path, plan, uncertain, manifest, ('127.0.0.1', 5000))
            with store.PlacementDirectory(path, rpc.Budget(60000), True) as owner:
                journal = owner.get(plan.key)
                with patch.object(app, 'Client', side_effect=AssertionError('unexpected contact')):
                    self.assertTrue(app.recover(journal, None, cancel=True)['pending'])
                    self.assertTrue(app.recovery_requires_native(journal))
                    self.assertFalse(app.recovery_requires_native(journal, True))
                    journal.retain_reconciliation(reply(placed, manifest))
                    self.assertFalse(app.recover(journal, None)['native_contacted'])

    def test_missing_authority_wrong_endpoint_and_expired_budget_fail_before_contact(self):
        plan, uncertain, _, manifest = fixture()
        with tempfile.TemporaryDirectory() as path:
            initialize(path, plan, uncertain, manifest, ('127.0.0.1', 5000))
            with store.PlacementDirectory(path, rpc.Budget(60000), True) as owner:
                journal = owner.get(plan.key)
                with patch.object(app, 'Client', side_effect=AssertionError('unexpected contact')):
                    with self.assertRaises(ValueError):
                        app.recover(journal, None)
                    with self.assertRaises(ValueError):
                        app.recover(journal, rpc.Authority(('127.0.0.1', 5001), TOKEN.encode()))
                    owner.budget.deadline = 0
                    with self.assertRaises(ValueError):
                        app.recover(journal, rpc.Authority(('127.0.0.1', 5000), TOKEN.encode()))

    def test_revocation_during_sync_suppresses_success_but_preserves_recovery(self):
        plan, uncertain, placed, manifest = fixture()
        with peer(plan, manifest, [placed]) as server, tempfile.TemporaryDirectory() as path:
            old = initialize(path, plan, uncertain, manifest, server.address)
            with store.PlacementDirectory(path, rpc.Budget(60000), True) as owner:
                journal = owner.get(plan.key)
                sync = os.fsync
                def revoke(fd):
                    sync(fd)
                    os.environ.pop('DFMCP_BUILD_TOKEN', None)
                with patch.object(store.os, 'fsync', revoke):
                    with self.assertRaises(ValueError):
                        app.recover(journal, rpc.Authority.load())
                self.assertTrue(journal.raw.startswith(old))
                self.assertFalse(journal.state.pending)
            with store.PlacementDirectory(path, rpc.Budget(60000)) as owner:
                out = app.recover(owner.get(plan.key), None)
                self.assertEqual(out['effect_status'], 'placed')
                self.assertFalse(out['storage_acknowledged_this_call'])

    def test_real_cli_query_then_offline_reopen_uses_one_original_native_key(self):
        plan, uncertain, placed, manifest = fixture()
        with peer(plan, manifest, [placed]) as server, tempfile.TemporaryDirectory() as path:
            old = initialize(path, plan, uncertain, manifest, server.address)
            command = [sys.executable, str(ROOT/'scripts/build_placement_client.py'), 'query',
                       '--directory', path, '--key', plan.key]
            first = subprocess.run(command, capture_output=True, timeout=10)
            self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
            self.assertEqual(len(first.stdout.splitlines()), 1)
            out = json.loads(first.stdout)
            self.assertTrue(out['ok'])
            self.assertFalse(out['result']['pending'])
            self.assertEqual(out['result']['effect_status'], 'placed')
            self.assertNotIn(TOKEN.encode(), first.stdout + first.stderr)
            self.assertTrue(Path(path, store.filename(plan.key)).read_bytes().startswith(old))
            env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
            second = subprocess.run(command, capture_output=True, env=env, timeout=10)
            self.assertEqual(second.returncode, 0, second.stdout + second.stderr)
            self.assertFalse(json.loads(second.stdout)['result']['native_contacted'])
            self.assertEqual(server.calls, ['Handshake', 'QueryPlacement'])

    def test_real_cli_failure_preserves_uncertainty_and_sanitizes_output(self):
        plan, uncertain, placed, manifest = fixture()
        with peer(plan, manifest, [placed], 'checksum') as server, tempfile.TemporaryDirectory() as path:
            old = initialize(path, plan, uncertain, manifest, server.address)
            out = subprocess.run([sys.executable, str(ROOT/'scripts/build_placement_client.py'),
                'query', '--directory', path, '--key', plan.key], capture_output=True, timeout=10)
            self.assertEqual(out.returncode, 2)
            self.assertEqual(len(out.stdout.splitlines()), 1)
            value = json.loads(out.stdout)
            self.assertFalse(value['ok'])
            self.assertTrue(value['result']['pending'])
            self.assertNotIn(TOKEN.encode(), out.stdout + out.stderr)
            self.assertEqual(Path(path, store.filename(plan.key)).read_bytes(), old)

    def test_normal_start_still_records_dispatch_before_one_native_commit(self):
        plan, _, placed, manifest = fixture()
        calls = []
        class Authority:
            address = ('127.0.0.1', 5000)
            def guard(self, operation):
                self.last = operation
        with tempfile.TemporaryDirectory() as path:
            with store.PlacementDirectory(path, rpc.Budget(60000), True) as owner:
                class Client:
                    def __init__(self, authority, budget, selection):
                        assert selection == plan.before.selection and budget is owner.budget
                    def observe(self):
                        return rpc.Reply(manifest, False, 0, capture=plan.before)
                    def query(self, original):
                        assert original == plan
                        return rpc.Reply(manifest, False, 0)
                    def prepare(self, original):
                        assert owner.get(plan.key).state.frames == 1
                        calls.append('prepare')
                        return rpc.Reply(manifest, False, 1, record=Record(plan, 'prepared', 'none'), replayed=False)
                    def commit(self, original):
                        assert owner.get(plan.key).state.dispatched
                        calls.append('commit')
                        return reply(placed, manifest)
                    def __enter__(self):
                        return self
                    def __exit__(self, *_):
                        calls.append('close')
                with patch.object(app, 'Client', Client):
                    out, original = app.start(owner, Authority(), plan.before.selection, plan.key, plan.digest.hex())
                    self.assertEqual(original, plan)
                    self.assertFalse(out['pending'])
                    with self.assertRaises(ValueError):
                        app.start(owner, Authority(), plan.before.selection, plan.key, plan.digest.hex())
                self.assertEqual(calls, ['prepare', 'commit', 'close'])

    def test_late_directory_close_failure_does_not_emit_a_second_cli_object(self):
        plan, uncertain, placed, manifest = fixture()
        with tempfile.TemporaryDirectory() as path:
            initialize(path, plan, uncertain, manifest, ('127.0.0.1', 5000))
            with store.PlacementDirectory(path, rpc.Budget(60000), True) as owner:
                owner.get(plan.key).retain_reconciliation(reply(placed, manifest))
            close = store.PlacementDirectory.close
            def late_failure(owner):
                close(owner)
                raise OSError('late descriptor close failure')
            output = io.StringIO()
            with patch.object(sys, 'stdout', output), patch.object(store.PlacementDirectory, 'close', late_failure):
                status = app.main(['query', '--directory', path, '--key', plan.key])
            self.assertEqual(status, 2)
            self.assertEqual(len(output.getvalue().splitlines()), 1)
            self.assertTrue(json.loads(output.getvalue())['ok'])

    def test_final_publication_rechecks_authority_and_never_duplicates_short_stdout(self):
        class Output:
            def __init__(self):
                self.calls = 0
            def write(self, raw):
                self.calls += 1
                return len(raw) - 1
            def flush(self):
                raise AssertionError('short write must not be flushed')
        output = Output()
        with patch.object(app.sys, 'stdout', type('Stream', (), {'buffer': output})()):
            self.assertEqual(app.write_output(b'{}\n'), 2)
        self.assertEqual(output.calls, 1)
        budget = rpc.Budget(60000)
        authority = type('Denied', (), {'guard': lambda *_: (_ for _ in ()).throw(ValueError('revoked'))})()
        with patch.object(app, 'write_output', side_effect=AssertionError('must not publish')):
            with self.assertRaises(ValueError):
                app.publish('query', {'pending': True}, None, budget, authority)


if __name__ == '__main__':
    unittest.main()
