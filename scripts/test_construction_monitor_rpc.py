"""Real joined TCP peers exercise the actual query-only construction transport.

Peers implement explicit test protocols, not DFHack/protobuf-runtime substitutes
for compatibility admission. No game or game mutation is executed here.
"""
from __future__ import annotations

from dataclasses import replace
import hashlib
import os
import socket
import struct
import threading
import time
import unittest
from unittest.mock import patch

import construction_monitor_rpc as rpc
from build_placement_wire import Rejected
from test_construction_receipt import RECEIPT, goal, operations, item

BUILD_SECRET, OPS_SECRET = b'b' * 32, b'o' * 32


def wire_message(fields):
    def number(value):
        result = []
        while value >= 128:
            result.append((value & 127) | 128)
            value >>= 7
        return bytes(result + [value])
    raw = bytearray()
    for key, value in fields.items():
        if isinstance(value, bytes):
            raw += number(key * 8 + 2) + number(len(value)) + value
        else:
            raw += number(key * 8) + number(value)
    return bytes(raw)


def read_message(raw):
    index, result = 0, {}
    def number():
        nonlocal index
        value, shift = 0, 0
        while True:
            byte = raw[index]
            index += 1
            value |= (byte & 127) << shift
            if not byte & 128:
                return value
            shift += 7
            assert shift < 70
    while index < len(raw):
        tag = number()
        key, kind = tag >> 3, tag & 7
        assert key not in result
        if kind == 0:
            result[key] = number()
        else:
            assert kind == 2
            length = number()
            result[key] = raw[index:index + length]
            index += length
    return result


class NativePeer:
    def __init__(self, captures=None, *, hook=None, lose=None, notifications=0, alias=False,
                 receipt=RECEIPT, delay=0):
        self.captures = [operations()] if captures is None else list(captures)
        self.hook, self.lose, self.notifications, self.alias = hook, lose, notifications, alias
        self.receipt, self.delay = receipt, delay
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(4)
        self.listener.settimeout(2)
        self.address = self.listener.getsockname()
        self.error, self.calls, self.bindings = None, [], []
        self.reads, self.releases, self.queries, self.connections = 0, 0, 0, 0
        self.thread = threading.Thread(target=self.serve, daemon=False)

    @staticmethod
    def exact(sock, count):
        raw = bytearray()
        while len(raw) < count:
            part = sock.recv(count - len(raw))
            if not part:
                raise EOFError()
            raw += part
        return bytes(raw)

    def serve(self):
        try:
            for capture in self.captures:
                with self.listener.accept()[0] as sock:
                    self.connections += 1
                    sock.settimeout(2)
                    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                    methods, queried, captured = {}, 0, False
                    try:
                        assert self.exact(sock, 12) == b'DFHack?\n\x01\0\0\0'
                        if self.delay:
                            time.sleep(self.delay)
                        sock.sendall(b'DFHack!\n\x01\0\0\0')
                        while True:
                            method, size = struct.unpack('<h2xi', self.exact(sock, 8))
                            assert 0 <= size <= 2048
                            fields = read_message(self.exact(sock, size))
                            if method == 0:
                                assert set(fields) == {1, 2, 3, 4}
                                plugin = fields[4].decode()
                                family = 'build' if plugin == 'dfmcp_build_v1_19' else 'operations'
                                assert plugin == rpc.PROFILES[family][0]
                                name = fields[1].decode()
                                assert (family, name) in rpc.BINDINGS, 'effectful or unknown method bound'
                                assert fields[2] == (rpc.PROFILES[family][1] + '.Request').encode()
                                assert fields[3] == (rpc.PROFILES[family][1] + '.Reply').encode()
                                identity = 2 if self.alias else len(methods) + 2
                                methods[identity] = family, name
                                self.bindings.append((family, name))
                                reply, tag = {1: identity}, 'bind'
                            else:
                                family, name = methods[method]
                                self.calls.append((family, name))
                                assert fields[1] == (BUILD_SECRET if family == 'build' else OPS_SECRET)
                                assert len(fields[2]) == 32 and fields[3] == 1 and fields[4] == rpc.PROFILES[family][2]
                                reply = {1: 1, 2: 0, 3: fields[2], 4: 1, 5: fields[4],
                                         6: 41 if family == 'build' else 987,
                                         7: b'test-df', 8: b'test-dfhack'}
                                tag = family + '_handshake'
                                if family == 'build':
                                    reply.update({12: 0, 13: 1 if self.receipt is not None else 0})
                                    if name == 'Handshake':
                                        assert set(fields) == {1, 2, 3, 4}
                                    else:
                                        assert name == 'QueryPlacement' and set(fields) == {1, 2, 3, 4, 10, 12}
                                        assert fields[10] == b'golden' and fields[12] == goal().record.plan.digest
                                        queried += 1
                                        self.queries += 1
                                        tag = 'before_receipt' if queried == 1 else 'after_receipt'
                                        if self.receipt is not None:
                                            reply[10] = self.receipt
                                else:
                                    assert tuple(fields[n] for n in (5, 6, 7, 8, 11)) == (4096, 4096, 65536, rpc.MAX_CAPTURE, rpc.PAGE)
                                    if name == 'Handshake':
                                        assert set(fields) == set(range(1, 9)) | {11}
                                    else:
                                        assert set(fields) == set(range(1, 13))
                                        assert name == 'ReadObservation'
                                        token, offset, release = fields[9], fields[10], fields[12]
                                        if release:
                                            assert release == 1 and token == b't' * 16 and offset == 0 and captured
                                            self.releases += 1
                                            reply[10] = token
                                            tag = 'release'
                                        else:
                                            tag = 'page'
                                            self.reads += 1
                                            if not token:
                                                assert offset == 0 and not captured
                                                captured = True
                                            else:
                                                assert token == b't' * 16 and captured and offset > 0
                                            part = capture[offset:offset + rpc.PAGE]
                                            reply.update({9: part, 10: b't' * 16, 11: offset,
                                                          12: len(capture), 13: hashlib.sha256(capture).digest(),
                                                          14: int(offset + len(part) == len(capture))})
                            if self.hook:
                                reply = self.hook(tag, fields, reply, self)
                            if tag == self.lose:
                                sock.sendall(struct.pack('<h2xi', -1, 1000) + b'\x08')
                                break
                            for _ in range(self.notifications):
                                sock.sendall(struct.pack('<h2xi', -3, 0))
                            body = wire_message(reply) if isinstance(reply, dict) else reply
                            framed = struct.pack('<h2xi', -1, len(body)) + body
                            for offset in range(0, len(framed), 503):
                                sock.sendall(framed[offset:offset + 503])
                    except (EOFError, BrokenPipeError, ConnectionResetError):
                        pass
        except BaseException as error:
            self.error = error
        finally:
            self.listener.close()

    def environment(self):
        return patch.dict(os.environ, {rpc.OPT_IN: '1', rpc.ENDPOINT: f'{self.address[0]}:{self.address[1]}',
                                     rpc.BUILD_TOKEN: BUILD_SECRET.decode(), rpc.OPERATIONS_TOKEN: OPS_SECRET.decode()}, clear=True)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_args):
        self.thread.join(5)
        if self.thread.is_alive():
            self.listener.close()
            self.thread.join(3)
            raise AssertionError('native peer did not quiesce')
        if self.error is not None:
            raise AssertionError('native peer failed') from self.error


def fetch():
    return rpc.acquire(rpc.Authority.load(), goal(), rpc.Budget(5000))


class QueryTransportTests(unittest.TestCase):
    def test_same_connection_bracket_and_only_four_read_bindings(self):
        with NativePeer() as peer, peer.environment():
            result = fetch()
            self.assertEqual(result.capture, operations())
            self.assertEqual(result.before_record, result.after_record)
            self.assertEqual(peer.connections, 1)
            self.assertEqual(peer.bindings, list(rpc.BINDINGS))
            self.assertEqual(peer.calls, [('build', 'Handshake'), ('operations', 'Handshake'),
                ('build', 'QueryPlacement'), ('operations', 'ReadObservation'),
                ('operations', 'ReadObservation'), ('build', 'QueryPlacement')])
            self.assertEqual((peer.reads, peer.releases, peer.queries), (1, 1, 2))

    def test_immutable_multi_page_capture_and_verified_release(self):
        records = tuple(item(n, holder=70 if n == 42 else None) for n in range(1, 2001))
        raw = operations(items=records, horizons=(91, 71, 2001))
        self.assertGreater(len(raw), rpc.PAGE)
        with NativePeer([raw]) as peer, peer.environment():
            result = fetch()
            self.assertEqual(result.capture, raw)
            self.assertGreater(peer.reads, 1)
            self.assertEqual(peer.releases, 1)
            self.assertEqual(len(result.validate(goal(), lambda: None).items), 2000)

    def test_missing_original_record_never_reads_operations(self):
        with NativePeer(receipt=None) as peer, peer.environment():
            with self.assertRaises(Rejected):
                fetch()
            self.assertEqual(peer.reads, 0)
        with NativePeer(receipt=RECEIPT[:-1] + b'\0') as peer, peer.environment():
            with self.assertRaises(Rejected):
                fetch()
            self.assertEqual(peer.reads, 0)

    def test_source_changes_at_either_bracket_end_never_publish(self):
        for at in ('build_handshake', 'before_receipt', 'after_receipt'):
            def hook(tag, fields, reply, peer):
                if tag == at:
                    reply[6] = 42
                return reply
            with self.subTest(at=at), NativePeer(hook=hook) as peer, peer.environment():
                with self.assertRaises(Rejected):
                    fetch()
                self.assertLessEqual(peer.reads, 1)

    def test_lost_page_release_or_trailing_query_never_returns_sample(self):
        for at in ('before_receipt', 'page', 'release', 'after_receipt'):
            with self.subTest(at=at), NativePeer(lose=at) as peer, peer.environment():
                with self.assertRaises(Rejected):
                    fetch()
                self.assertEqual(peer.connections, 1)
                self.assertLessEqual(peer.queries, 2)
                self.assertLessEqual(peer.reads, 1)

    def test_page_bounds_digest_completeness_and_profile_refusals(self):
        changes = ({11: 1}, {12: rpc.MAX_CAPTURE + 1}, {14: 0}, {13: b'x' * 32},
                   {10: b't' * 15}, {9: b''}, {9: 1}, {6: 988}, {5: 3}, {7: b'other-df'})
        for changed in changes:
            def hook(tag, fields, reply, peer):
                return {**reply, **changed} if tag == 'page' else reply
            with self.subTest(changed=changed), NativePeer(hook=hook) as peer, peer.environment():
                with self.assertRaises(Rejected):
                    fetch()
                self.assertEqual(peer.releases, 0)

    def test_mixed_page_identity_and_release_token_refused(self):
        raw = operations(items=tuple(item(n, holder=None) for n in range(1, 2001)), horizons=(91, 71, 2001))
        def hook(tag, fields, reply, peer):
            return {**reply, 10: b'v' * 16} if tag == 'page' and fields[10] else reply
        with NativePeer([raw], hook=hook) as peer, peer.environment():
            with self.assertRaises(Rejected):
                fetch()
            self.assertEqual(peer.releases, 0)
        with NativePeer(hook=lambda t, f, r, p: {**r, 10: b'v' * 16} if t == 'release' else r) as peer, peer.environment():
            with self.assertRaises(Rejected):
                fetch()
            self.assertEqual(peer.queries, 1)

    def test_complete_semantic_decoder_runs_after_transport(self):
        raw = operations(items=(item(holder=69),))
        with NativePeer([raw]) as peer, peer.environment():
            with self.assertRaises(Rejected):
                fetch()
            self.assertEqual(peer.releases, 1)
            self.assertEqual(peer.queries, 2)

    def test_alias_unknown_duplicate_and_noncanonical_native_fields(self):
        with NativePeer(alias=True) as peer, peer.environment(), self.assertRaises(Rejected):
            fetch()
        for extra in (b'\x08\x01', b'\x78\x01', b'\x08\x80\x00'):
            def hook(tag, fields, reply, peer):
                return wire_message(reply) + extra if tag == 'build_handshake' else reply
            with NativePeer(hook=hook) as peer, peer.environment(), self.assertRaises(Rejected):
                fetch()
        for raw in (b'\x08\x80', b'\x08' + b'\xff' * 10, b'\x0b', b'\x12\x04abc', b'\x00'):
            with self.assertRaises(Rejected):
                rpc.decode(raw)
        for n in (0, 127, 128, 2**64 - 1):
            encoded = rpc.encode({1: n, 2: b'abc'})
            self.assertEqual(read_message(encoded), {1: n, 2: b'abc'})
            self.assertEqual(rpc.decode(wire_message({1: n, 2: b'abc'})), {1: n, 2: b'abc'})

    def test_whole_connection_deadline_calls_bytes_and_notifications(self):
        with NativePeer() as peer, peer.environment():
            budget = rpc.Budget(5000)
            budget.calls = 4
            with self.assertRaises(Rejected):
                rpc.acquire(rpc.Authority.load(), goal(), budget)
            self.assertEqual(peer.reads, 0)
        with NativePeer(notifications=9) as peer, peer.environment(), self.assertRaises(Rejected):
            fetch()
        with NativePeer(delay=0.05) as peer, peer.environment(), self.assertRaises((Rejected, TimeoutError)):
            rpc.acquire(rpc.Authority.load(), goal(), rpc.Budget(5))
        with NativePeer() as peer, peer.environment():
            with rpc.Client(rpc.Authority.load(), goal(), rpc.Budget(5000)) as client:
                client.budget.network_bytes = 1
                with self.assertRaises(Rejected):
                    client.capture_once()
                self.assertTrue(client.closed)

    def test_configuration_revocation_and_source_software_disagreement(self):
        def hook(tag, fields, reply, peer):
            if tag == 'before_receipt':
                os.environ[rpc.OPT_IN] = '0'
            return reply
        with NativePeer(hook=hook) as peer, peer.environment(), self.assertRaises(Rejected):
            fetch()
        with NativePeer(hook=lambda t, f, r, p: {**r, 8: b'other'} if t == 'operations_handshake' else r) as peer, peer.environment():
            with self.assertRaises(Rejected):
                fetch()
            self.assertEqual(peer.reads, 0)

    def test_one_acquisition_per_connection_no_replay_permit(self):
        with NativePeer() as peer, peer.environment():
            with rpc.Client(rpc.Authority.load(), goal(), rpc.Budget(5000)) as client:
                client.capture_once()
                with self.assertRaises(Rejected):
                    client.capture_once()
            self.assertEqual(peer.reads, 1)

    def test_mutation_admission_authority_and_nonloopback_rejected(self):
        for value in ('localhost:5000', '192.168.1.1:5000', '127.0.0.1:05000', '127.0.0.1:0', '[::1]:5000'):
            with self.assertRaises(Rejected):
                rpc.endpoint(value)
        environment = {rpc.OPT_IN: '1', rpc.BUILD_TOKEN: BUILD_SECRET.decode(), rpc.OPERATIONS_TOKEN: OPS_SECRET.decode()}
        with patch.dict(os.environ, environment, clear=True):
            authority = rpc.Authority.load()
            self.assertNotIn(BUILD_SECRET.decode(), repr(authority))
            self.assertNotIn(OPS_SECRET.decode(), repr(authority))
            for key in ('DFMCP_BUILD_ALLOW_PLACE', 'DFMCP_ADMITTED_BRIDGE_PROTOCOL', 'DFMCP_ADMISSION_TICKET'):
                with patch.dict(os.environ, {key: '1'}), self.assertRaises(Rejected):
                    rpc.Authority.load()


if __name__ == '__main__':
    unittest.main(verbosity=2)
