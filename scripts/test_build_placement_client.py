"""Exercise actual furniture RPC, journal and CLI with joined loopback doubles.

Byte vectors are independently constructed by the existing native engine test.
This does not execute DFHack, generated protobuf, Rust/MCP or a real fortress.
"""
from __future__ import annotations

from dataclasses import replace
import contextlib
import hashlib
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
import time
import unittest
from unittest.mock import patch

import build_placement_wire as w
import build_placement_rpc as rpc
import build_placement_store as store
import build_placement_client as cli
from test_build_placement_engine import golden_vectors

VECTORS = {key: bytes.fromhex(value) for key, value in golden_vectors().items()}
SECRET = b's' * 32


def plan(key='golden'):
    return w.Plan(key, w.Capture.decode(VECTORS['capture']))


def native_record(key='golden', phase='prepared'):
    # Rebind the independent engine fixture's length-framed key, plan and token.
    # Everything after token (including exact expected after/insertion) stays fixed.
    before = VECTORS['capture']
    framed = lambda raw: struct.pack('>H', len(raw)) + raw
    digest = bytes.fromhex(golden_vectors()['plan'])
    token = hashlib.sha256(b'dfmcp-build-token/1\0' + framed(key.encode()) + digest).digest()[:16]
    old = VECTORS[phase]
    offset = 8 + 2 + len('golden') + 2 + len(before) + 32 + 16
    body = b'DFMBR019' + framed(key.encode()) + framed(before) + digest + token + old[offset:-32]
    return body + hashlib.sha256(b'dfmcp-build-receipt/1\0' + body).digest()


def message(fields):
    def number(value):
        out = []
        while value > 127:
            out.append(value % 128 + 128)
            value //= 128
        return bytes(out + [value])
    out = b''
    for key, value in fields.items():
        if isinstance(value, bytes):
            out += number(key * 8 + 2) + number(len(value)) + value
        else:
            out += number(key * 8) + number(value)
    return out


class NativeDouble:
    def __init__(self, connections=1, lose_on=None, hook=None, retained=None, alias=False,
                 notifications=0, delay=0, outcome='placed'):
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(4)
        self.listener.settimeout(2)
        self.address = self.listener.getsockname()
        self.connections, self.lose_on, self.hook = connections, lose_on, hook
        self.records = {} if retained is None else dict(retained)
        self.alias, self.notifications, self.delay, self.outcome = alias, notifications, delay, outcome
        self.calls, self.requests, self.error, self.effects = [], [], None, 0
        self.thread = threading.Thread(target=self.serve, daemon=False)

    @staticmethod
    def read(sock, count):
        out = b''
        while len(out) < count:
            part = sock.recv(count - len(out))
            if not part:
                raise EOFError()
            out += part
        return out

    def serve(self):
        try:
            for _ in range(self.connections):
                with self.listener.accept()[0] as sock:
                    sock.settimeout(2)
                    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                    try:
                        assert self.read(sock, 12) == b'DFHack?\n\x01\x00\x00\x00'
                        if self.delay:
                            time.sleep(self.delay)
                        sock.sendall(b'DFHack!\n\x01\x00\x00\x00')
                        bindings = {}
                        while True:
                            method, size = struct.unpack('<h2xi', self.read(sock, 8))
                            assert 0 <= size <= 2048
                            fields = rpc.decode(self.read(sock, size))
                            if method == 0:
                                name = fields[1].decode()
                                assert name in rpc.METHODS
                                assert fields[2] == b'dfmcp.build.v1_19.Request'
                                assert fields[3] == b'dfmcp.build.v1_19.Reply'
                                assert fields[4] == b'dfmcp_build_v1_19'
                                identity = 2 if self.alias else len(bindings) + 2
                                bindings[identity] = name
                                response = {1: identity}
                                for _ in range(self.notifications):
                                    sock.sendall(struct.pack('<h2xi', -3, 0))
                            else:
                                name = bindings[method]
                                self.calls.append(name)
                                self.requests.append(fields)
                                assert fields[1] == SECRET and fields[3] == 1 and fields[4] == 19
                                shapes = {'Handshake': set(), 'ReadPlacement': set(range(5, 10)),
                                    'PreparePlacement': set(range(5, 13)), 'CommitPlacement': {10, 12, 13},
                                    'QueryPlacement': {10, 12}, 'CancelPlacement': {10, 12, 13}}
                                assert set(fields) == {1, 2, 3, 4} | shapes[name]
                                response = {1: 1, 2: 0, 3: fields[2], 4: 1, 5: 19, 6: 41,
                                            7: b'fake-df', 8: b'fake-dfhack'}
                                key = fields.get(10, b'golden').decode()
                                if name in ('ReadPlacement', 'PreparePlacement'):
                                    assert tuple(fields[n] for n in range(5, 10)) == (1, 42, 15, 15, 2)
                                if name == 'ReadPlacement':
                                    response[9] = VECTORS['capture']
                                elif name == 'PreparePlacement':
                                    assert fields[11] == plan(key).before.witness and fields[12] == plan(key).digest
                                    response[11] = int(key in self.records)
                                    self.records.setdefault(key, native_record(key))
                                    response[10] = self.records[key]
                                elif name in ('CommitPlacement', 'CancelPlacement', 'QueryPlacement'):
                                    assert fields[12] == plan(key).digest
                                    if name != 'QueryPlacement':
                                        assert fields[13] == plan(key).token
                                    if name == 'CommitPlacement':
                                        assert key in self.records
                                        if w.Record.decode(self.records[key]).phase == 'prepared':
                                            self.effects += 1
                                            self.records[key] = native_record(key, self.outcome)
                                    elif name == 'CancelPlacement' and key in self.records:
                                        if w.Record.decode(self.records[key]).phase == 'prepared':
                                            self.records[key] = native_record(key, 'cancelled')
                                    if key in self.records:
                                        response[10] = self.records[key]
                                response[12] = int(any(w.Record.decode(raw).phase == 'indeterminate' for raw in self.records.values()))
                                response[13] = len(self.records)
                                if self.hook:
                                    response = self.hook(name, fields, response)
                                if name == self.lose_on:
                                    sock.sendall(struct.pack('<h2xi', -1, 512) + b'\x08')
                                    break
                            framed = message(response)
                            framed = struct.pack('<h2xi', -1, len(framed)) + framed
                            for start in range(0, len(framed), 7):
                                sock.sendall(framed[start:start + 7])
                    except (EOFError, BrokenPipeError, ConnectionResetError):
                        pass
        except BaseException as error:
            self.error = error
        finally:
            self.listener.close()

    def environment(self, placement='1'):
        values = {rpc.OPT_IN: '1', rpc.TOKEN: SECRET.decode(), rpc.ENDPOINT: f'{self.address[0]}:{self.address[1]}'}
        if placement is not None:
            values[rpc.PLACE] = placement
        return patch.dict(os.environ, values, clear=True)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_args):
        self.thread.join(5)
        if self.thread.is_alive():
            self.listener.close()
            self.thread.join(3)
            raise AssertionError('native double did not quiesce')
        if self.error:
            raise AssertionError('native double failed') from self.error


class RpcTests(unittest.TestCase):
    def client(self, server, **kwargs):
        return rpc.Client(rpc.Authority.load(), rpc.Budget(5000, **kwargs), plan().before.selection)

    def test_fragmented_exact_lifecycle_and_duplicate_fence(self):
        with NativeDouble() as server, server.environment(), self.client(server) as client:
            self.assertEqual(client.observe().capture, plan().before)
            self.assertFalse(client.prepare(plan()).replayed)
            self.assertEqual(client.commit(plan()).record.phase, 'placed')
            self.assertEqual(client.query(plan()).record.raw, native_record(phase='placed'))
            with self.assertRaises(w.Rejected):
                client.commit(plan())
            self.assertEqual(server.effects, 1)
            self.assertEqual(server.calls, ['Handshake', 'ReadPlacement', 'PreparePlacement', 'CommitPlacement', 'QueryPlacement'])

    def test_query_and_replayed_preparation_cannot_dispatch(self):
        with NativeDouble(retained={'golden': native_record()}) as server, server.environment(), self.client(server) as client:
            self.assertTrue(client.prepare(plan()).replayed)
            with self.assertRaises(w.Rejected):
                client.commit(plan())
            self.assertEqual(server.effects, 0)
        with NativeDouble(retained={'golden': native_record()}) as server, server.environment(), self.client(server) as client:
            client.query(plan())
            with self.assertRaises(w.Rejected):
                client.commit(plan())
            with self.assertRaises(w.Rejected):
                client.prepare(plan())

    def test_query_consumes_fresh_permit(self):
        with NativeDouble() as server, server.environment(), self.client(server) as client:
            client.prepare(plan())
            client.query(plan())
            with self.assertRaises(w.Rejected):
                client.commit(plan())
            self.assertEqual(server.effects, 0)

    def test_lost_commit_reconnect_only_queries(self):
        with NativeDouble(connections=2, lose_on='CommitPlacement') as server, server.environment():
            with self.client(server) as client:
                client.prepare(plan())
                with self.assertRaises(w.Rejected):
                    client.commit(plan())
                with self.assertRaises(w.Rejected):
                    client.commit(plan())
            with self.client(server) as client:
                self.assertEqual(client.query(plan()).record.phase, 'placed')
                with self.assertRaises(w.Rejected):
                    client.commit(plan())
            self.assertEqual(server.calls.count('CommitPlacement'), 1)

    def test_revocation_preserves_cancellation_and_refuses_placement(self):
        with NativeDouble() as server, server.environment(), self.client(server) as client:
            client.prepare(plan())
            os.environ[rpc.PLACE] = '0'
            self.assertEqual(client.cancel(plan()).record.phase, 'cancelled')
            self.assertEqual(server.effects, 0)
        with NativeDouble() as server, server.environment(), self.client(server) as client:
            client.prepare(plan())
            os.environ[rpc.PLACE] = '0'
            with self.assertRaises(w.Rejected):
                client.commit(plan())
            self.assertTrue(client._closed)
            self.assertEqual(server.effects, 0)

    def test_absence_source_drift_and_missing_known_record(self):
        with NativeDouble() as server, server.environment(placement=None), self.client(server) as client:
            self.assertIsNone(client.query(plan()).record)
            with self.assertRaises(w.Rejected):
                client.commit(plan())
        def erase(name, fields, reply):
            if name == 'QueryPlacement':
                reply.pop(10, None)
            return reply
        with NativeDouble(hook=erase) as server, server.environment(), self.client(server) as client:
            client.prepare(plan())
            with self.assertRaises(w.Rejected):
                client.query(plan())
            self.assertTrue(client._closed)
        with NativeDouble(hook=lambda n, f, r: {**r, 6: 42} if n == 'ReadPlacement' else r) as server, server.environment(), self.client(server) as client:
            with self.assertRaises(w.Rejected):
                client.observe()
            self.assertTrue(client._closed)

    def test_wrong_fields_types_bounds_aliases_and_indeterminate_fence(self):
        for changes in ({3: b'wrong'}, {5: 16}, {6: b'41'}, {12: 2}, {13: 257}, {9: b'extra'}, {11: 0}):
            with self.subTest(changes=changes), NativeDouble(hook=lambda n, f, r: {**r, **changes}) as server, server.environment():
                with self.assertRaises(w.Rejected):
                    self.client(server)
        with NativeDouble(alias=True) as server, server.environment(), self.assertRaises(w.Rejected):
            self.client(server)
        with NativeDouble(outcome='indeterminate') as server, server.environment(), self.client(server) as client:
            client.prepare(plan())
            reply = client.commit(plan())
            self.assertTrue(reply.unresolved)
            self.assertFalse(reply.record.resolved)
        with NativeDouble(hook=lambda n, f, r: {**r, 12: 1} if n == 'PreparePlacement' else r) as server, server.environment(), self.client(server) as client:
            with self.assertRaises(w.Rejected):
                client.prepare(plan())
            self.assertTrue(client._closed)
            self.assertEqual(server.effects, 0)

    def test_protobuf_canonical_and_whole_connection_budgets(self):
        for value in (0, 127, 128, 2**32 - 1, 2**64 - 1):
            self.assertEqual(rpc.decode(message({1: value, 2: b'abc'})), {1: value, 2: b'abc'})
            self.assertEqual(rpc.encode({1: value, 2: b'abc'}), message({1: value, 2: b'abc'}))
        for raw in (b'\x08\x80\0', b'\x08\x80', b'\x08' + b'\xff' * 10, b'\x00', b'\x0b',
                    b'\x08\1\x08\1', b'\x12\4abc', b'\x70\1', b'x' * 8193):
            with self.assertRaises(w.Rejected):
                rpc.decode(raw)
        with NativeDouble(notifications=9) as server, server.environment(), self.assertRaises(w.Rejected):
            self.client(server)
        with NativeDouble() as server, server.environment(), self.client(server, max_calls=7) as client:
            with self.assertRaises(w.Rejected):
                client.observe()
            self.assertEqual(server.calls, ['Handshake'])
        with NativeDouble() as server, server.environment(), self.client(server) as client:
            client.budget.bytes_left = 1
            with self.assertRaises(w.Rejected):
                client.observe()
            self.assertTrue(client._closed)
        with NativeDouble(delay=0.04) as server, server.environment(), self.assertRaises((w.Rejected, TimeoutError)):
            rpc.Client(rpc.Authority.load(), rpc.Budget(5), plan().before.selection)

    def test_authority_endpoint_and_secret_redaction(self):
        for value in ('example.com:5000', '192.168.1.1:5000', '127.0.0.1:0', '127.0.0.1:05000', '127.0.0.1:65536', '[::1]:5000'):
            with self.assertRaises(ValueError):
                rpc.endpoint(value)
        with patch.dict(os.environ, {rpc.OPT_IN: '1', rpc.TOKEN: SECRET.decode(), rpc.PLACE: '1'}, clear=True):
            value = rpc.Authority.load(True)
            self.assertNotIn(SECRET.decode(), repr(value))
            os.environ['DFMCP_ADMITTED_BRIDGE_PROTOCOL'] = '1.0'
            with self.assertRaises(w.Rejected):
                value.guard('QueryPlacement')


class WorkflowTests(unittest.TestCase):
    def directory(self, root):
        directory = Path(root) / 'placements'
        directory.mkdir(mode=0o700)
        return str(directory)

    def test_full_review_dispatch_durable_receipt_and_offline_cli(self):
        with tempfile.TemporaryDirectory() as temp, NativeDouble() as server, server.environment():
            directory = self.directory(temp)
            with store.PlacementDirectory(directory, rpc.Budget(5000), True) as owner:
                out, value = cli.start(owner, rpc.Authority.load(True), plan().before.selection, 'golden', plan().digest.hex())
                self.assertEqual(out['effect_status'], 'placed')
                self.assertTrue(out['storage_acknowledged_this_call'])
                self.assertFalse(out['pending'])
                self.assertEqual(owner.get('golden').state.frames, 4)
                self.assertLess(len(cli.bounded_output(cli.packet('start', out, value))), cli.MAX_OUTPUT)
            process = subprocess.run([sys.executable, str(Path(cli.__file__)), 'inspect', '--directory', directory,
                                      '--key', 'golden'], capture_output=True, timeout=10, env={'PATH': os.defpath}, check=False)
            self.assertEqual(process.returncode, 0, process.stderr)
            result = json.loads(process.stdout)
            self.assertEqual(result['result']['effect_status'], 'placed')
            self.assertFalse(result['result']['native_contacted'])
            self.assertFalse(result['result']['construction_completion_proven'])
            self.assertEqual(server.effects, 1)

    def test_lost_reply_reopen_query_and_no_new_dispatch(self):
        with tempfile.TemporaryDirectory() as temp, NativeDouble(connections=2, lose_on='CommitPlacement') as server, server.environment():
            directory = self.directory(temp)
            with store.PlacementDirectory(directory, rpc.Budget(5000), True) as owner:
                with self.assertRaises(w.Rejected):
                    cli.start(owner, rpc.Authority.load(True), plan().before.selection, 'golden', plan().digest.hex())
                self.assertTrue(owner.get('golden').state.dispatched)
            with store.PlacementDirectory(directory, rpc.Budget(5000), True) as owner:
                with self.assertRaises(w.Rejected):
                    owner.ready('different')
                out = cli.recover(owner.get('golden'), rpc.Authority.load())
                self.assertEqual(out['effect_status'], 'placed')
                owner.ready('different')
            self.assertEqual(server.calls.count('CommitPlacement'), 1)
            self.assertEqual(server.calls.count('PreparePlacement'), 1)

    def test_uncertain_receipt_remains_pending_and_offline_immutable(self):
        with tempfile.TemporaryDirectory() as temp, NativeDouble(outcome='indeterminate') as server, server.environment():
            directory = self.directory(temp)
            with store.PlacementDirectory(directory, rpc.Budget(5000), True) as owner:
                out, _ = cli.start(owner, rpc.Authority.load(True), plan().before.selection, 'golden', plan().digest.hex())
                self.assertTrue(out['pending'])
            with store.PlacementDirectory(directory, rpc.Budget(5000), True) as owner:
                out = cli.recover(owner.get('golden'), None)
                self.assertFalse(out['native_contacted'])
                self.assertEqual(out['effect_status'], 'indeterminate')
                with self.assertRaises(w.Rejected):
                    owner.ready('new')
            self.assertEqual(server.effects, 1)

    def test_wrong_confirmation_creates_no_intent_or_effect(self):
        with tempfile.TemporaryDirectory() as temp, NativeDouble() as server, server.environment():
            directory = self.directory(temp)
            with store.PlacementDirectory(directory, rpc.Budget(5000), True) as owner:
                with self.assertRaises(w.Rejected):
                    cli.start(owner, rpc.Authority.load(True), plan().before.selection, 'golden', '00' * 32)
                self.assertEqual(owner.journals, {})
            self.assertEqual(server.calls, ['Handshake', 'ReadPlacement'])

    def test_preparation_loss_recovers_with_retirement_after_revocation(self):
        with tempfile.TemporaryDirectory() as temp, NativeDouble(connections=2, lose_on='PreparePlacement') as server, server.environment():
            directory = self.directory(temp)
            with store.PlacementDirectory(directory, rpc.Budget(5000), True) as owner:
                with self.assertRaises(w.Rejected):
                    cli.start(owner, rpc.Authority.load(True), plan().before.selection, 'golden', plan().digest.hex())
                self.assertFalse(owner.get('golden').state.dispatched)
            os.environ[rpc.PLACE] = '0'
            with store.PlacementDirectory(directory, rpc.Budget(5000), True) as owner:
                out = cli.recover(owner.get('golden'), rpc.Authority.load(), True)
                self.assertEqual(out['effect_status'], 'cancelled')
                self.assertFalse(out['pending'])
            self.assertEqual(server.effects, 0)

    def test_dispatch_sync_failure_never_sends_commit(self):
        with tempfile.TemporaryDirectory() as temp, NativeDouble() as server, server.environment():
            directory = self.directory(temp)
            original = store.Journal.append
            def append(journal, kind, payload):
                if kind == 'dispatch':
                    raise OSError('injected dispatch sync failure')
                return original(journal, kind, payload)
            with store.PlacementDirectory(directory, rpc.Budget(5000), True) as owner, patch.object(store.Journal, 'append', append):
                with self.assertRaises(OSError):
                    cli.start(owner, rpc.Authority.load(True), plan().before.selection, 'golden', plan().digest.hex())
                self.assertTrue(owner.get('golden').state.pending)
            self.assertNotIn('CommitPlacement', server.calls)

    def test_terminal_sync_loss_recovers_without_another_commit(self):
        with tempfile.TemporaryDirectory() as temp, NativeDouble(connections=2) as server, server.environment():
            directory = self.directory(temp)
            original = store.Journal.retain
            def retain(journal, reply, prepared=False):
                if not prepared:
                    raise OSError('injected receipt sync loss')
                return original(journal, reply, prepared)
            with store.PlacementDirectory(directory, rpc.Budget(5000), True) as owner, patch.object(store.Journal, 'retain', retain):
                with self.assertRaises(OSError):
                    cli.start(owner, rpc.Authority.load(True), plan().before.selection, 'golden', plan().digest.hex())
            with store.PlacementDirectory(directory, rpc.Budget(5000), True) as owner:
                self.assertEqual(cli.recover(owner.get('golden'), rpc.Authority.load())['effect_status'], 'placed')
            self.assertEqual(server.effects, 1)

    def test_cli_refuses_recovery_retargeting_without_native_or_files(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output), patch.object(rpc.socket, 'socket', side_effect=AssertionError('no socket')):
            code = cli.main(['query', '--directory', '/absent', '--key', 'golden', '--kind', 'bed'])
        self.assertEqual(code, 2)
        value = json.loads(output.getvalue())
        self.assertFalse(value['ok'])
        self.assertEqual(value['agent_turn']['active_work'][0]['key'], 'golden')


if __name__ == '__main__':
    unittest.main(verbosity=2)
