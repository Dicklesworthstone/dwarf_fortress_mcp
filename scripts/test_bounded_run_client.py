#!/usr/bin/env python3
"""Client codec/custody tests and joined loopback TCP protocol-double scenarios.

No game, C++ protobuf ABI, Rust, or MCP qualification is established. All temporary
capsules are retained. TCP helpers are supervised and joined, never detached.
"""
from __future__ import annotations

from contextlib import redirect_stdout
import hashlib
import io
import json
import os
from pathlib import Path
import socket
import struct
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import bounded_run_client as c

OBS = b'DFMRO013' + struct.pack('>QQQBBB', 41, 0, 100, 1, 1, 1)
TOKEN = b's' * 32


def reference_message(fields: dict) -> bytes:
    """Independent small protobuf encoder for server-side reference messages."""
    def number(n):
        parts = []
        while n > 127:
            parts.append(n % 128 + 128); n //= 128
        return bytes(parts + [n])
    result = b''
    for key, value in fields.items():
        if isinstance(value, bytes):
            result += number(key * 8 + 2) + number(len(value)) + value
        else:
            result += number(key * 8) + number(value)
    return result


def reference_record(key='test', phase=0, reason=0, attempted=0, paused=0, known=0, ticks=10, wall=1000):
    name = key.encode(); name = struct.pack('>H', len(name)) + name
    spec = struct.pack('>II', ticks, wall)
    plan = hashlib.sha256(b'dfmcp-bounded-run-plan/1\0' + spec + OBS).digest()
    token = hashlib.sha256(b'dfmcp-bounded-run-token/1\0' + name + plan).digest()[:16]
    data = (b'DFMRE013' + name + spec + OBS + plan + token + bytes([phase, reason, attempted, paused, known])
            + struct.pack('>Q', 110 if known and phase == 3 else 100 if known else 0))
    return data + hashlib.sha256(b'dfmcp-bounded-run-receipt/1\0' + data).digest()


def intent(address=('127.0.0.1', 5000)):
    plan = hashlib.sha256(b'dfmcp-bounded-run-plan/1\0' + struct.pack('>II', 10, 1000) + OBS).digest()
    return {'format': 'dfmcp.bounded-run-intent/1', 'endpoint': f'{address[0]}:{address[1]}', 'idempotency_key': 'test',
            'game_ticks': 10, 'wall_ms': 1000, 'observation_hex': OBS.hex(), 'plan_digest_hex': plan.hex(),
            'prepare_token_hex': c.token_for('test', plan).hex(), 'df_version': 'fake-df',
            'dfhack_version': 'fake-dfhack', 'effect_status': 'indeterminate_until_native_query'}


class NativeDouble:
    def __init__(self, connections=1, lose_commit=False, nonce_error=False, alias=False, notifications=0, delay=0, absent_query=False, query_running=0):
        self.listener = socket.socket(); self.listener.bind(('127.0.0.1', 0)); self.listener.listen(2); self.listener.settimeout(2)
        self.address = self.listener.getsockname(); self.connections = connections
        self.lose_commit, self.nonce_error, self.alias = lose_commit, nonce_error, alias
        self.notifications, self.delay = notifications, delay
        self.absent_query, self.query_running = absent_query, query_running
        self.effects = 0; self.calls = []; self.error = None; self.record = None
        self.thread = threading.Thread(target=self.serve, daemon=False)

    @staticmethod
    def read(sock, size):
        result = b''
        while len(result) < size:
            part = sock.recv(size - len(result))
            if not part:
                raise EOFError()
            result += part
        return result

    @staticmethod
    def reply(sock, fields):
        data = reference_message(fields)
        framed = struct.pack('<h2xi', -1, len(data)) + data
        # Fragment every response to exercise exact-read loops and one deadline.
        for offset in range(0, len(framed), 7):
            sock.sendall(framed[offset:offset + 7])

    def serve(self):
        try:
            for _ in range(self.connections):
                with self.listener.accept()[0] as sock:
                    sock.settimeout(2)
                    try:
                        assert self.read(sock, 12) == b'DFHack?\n\x01\x00\x00\x00'
                        if self.delay:
                            time.sleep(self.delay)
                        sock.sendall(b'DFHack!\n\x01\x00\x00\x00')
                        methods = {}
                        while True:
                            method, width = struct.unpack('<h2xi', self.read(sock, 8))
                            assert 0 <= width <= 2048
                            request = c.decode(self.read(sock, width))
                            if method == 0:
                                assert request[2] == b'dfmcp.run.v1_13.Request' and request[3] == b'dfmcp.run.v1_13.Reply'
                                assert request[4] == b'dfmcp_run_v1_13' and request[1].decode() in c.METHODS
                                name = request[1].decode(); bound = 2 if self.alias else len(methods) + 2
                                methods[bound] = name
                                for _ in range(self.notifications):
                                    sock.sendall(struct.pack('<h2xi', -3, 0))
                                self.reply(sock, {1: bound}); continue
                            name = methods[method]; self.calls.append(name)
                            assert request[1] == TOKEN and request[3] == 1 and request[4] == 13
                            common = {1: 1, 2: 0, 3: b'x' * 32 if self.nonce_error else request[2], 4: 1, 5: 13,
                                      6: 41, 7: b'fake-df', 8: b'fake-dfhack', 11: 0, 12: int(self.record is not None)}
                            if name == 'ObserveRun':
                                common[9] = OBS
                            elif name == 'PrepareRun':
                                assert set(request) == set(range(1, 10))
                                assert request[5] == b'test' and request[6] == 10 and request[7] == 1000 and request[8] == OBS
                                assert request[9] == hashlib.sha256(b'dfmcp-bounded-run-plan/1\0' + struct.pack('>II', 10, 1000) + OBS).digest()
                                self.record = reference_record(); common[10] = self.record; common[12] = 1
                            elif name in ('CommitRun', 'CancelRun', 'QueryRun'):
                                assert set(request) == ({1, 2, 3, 4, 5, 9} if name == 'QueryRun' else {1, 2, 3, 4, 5, 9, 10})
                                if name == 'CommitRun':
                                    self.effects += 1
                                    self.record = reference_record(phase=1, attempted=1, known=1)
                                    common[11] = 1
                                    if self.lose_commit:
                                        # A write happened, but the reply header is all the client receives.
                                        sock.sendall(struct.pack('<h2xi', -1, 200)); break
                                else:
                                    self.record = reference_record(phase=3, reason=3 if name == 'CancelRun' else 1,
                                                                   attempted=1, paused=1, known=1)
                                common[10] = self.record; common[12] = 1
                            if name == 'QueryRun' and self.absent_query:
                                common.pop(10, None); common[11] = 0; common[12] = 0
                            elif name == 'QueryRun' and self.query_running:
                                self.query_running -= 1
                                common[10] = reference_record(phase=1, attempted=1, known=1); common[11] = 1
                            self.reply(sock, common)
                    except (EOFError, BrokenPipeError, ConnectionResetError):
                        pass
        except BaseException as error:
            self.error = error
        finally:
            self.listener.close()

    def __enter__(self):
        self.thread.start(); return self

    def __exit__(self, *_args):
        self.thread.join(4)
        if self.thread.is_alive():
            self.listener.close(); self.thread.join(3)
            raise AssertionError('TCP double did not quiesce')
        if self.error:
            raise AssertionError('TCP double failed') from self.error


class ClientTests(unittest.TestCase):
    def setUp(self):
        self.directory = Path(tempfile.mkdtemp(prefix='dfmcp-run-client-'))
        self.directory.chmod(0o700); self.path = self.directory / 'intent.json'

    def create(self):
        with c.capsule(self.path, intent()):
            pass

    def test_varints_and_reference_protobuf(self):
        for value in (0, 1, 127, 128, 255, 16383, 16384, 2**32 - 1, 2**64 - 1):
            with self.subTest(value=value):
                self.assertEqual(c.encode({1: value, 2: b'abc'}), reference_message({1: value, 2: b'abc'}))
                self.assertEqual(c.decode(c.encode({1: value, 2: b'abc'})), {1: value, 2: b'abc'})
        self.assertEqual(c.encode({1: 1, 2: b'abc'}).hex(), '08011203616263')
        for raw in (b'\x08\x80\0', b'\x08\x80', b'\x08' + b'\xff'*10,
                    b'\x0b', b'\x00', b'\x68\x01', b'\x08\1\x08\1', b'\x12\4abc'):
            with self.subTest(raw=raw), self.assertRaises(c.Rejected):
                c.decode(raw)

    def test_record_vectors_and_every_byte_corruption(self):
        raw = reference_record(phase=3, reason=1, attempted=1, paused=1, known=1)
        decoded = c.decode_record(raw)
        self.assertEqual(decoded['phase'], 'stopped'); self.assertEqual(decoded['observed_ticks_advanced'], 10)
        self.assertTrue(decoded['current_pause_unproved'])
        for index in range(len(raw)):
            with self.subTest(byte=index), self.assertRaises(ValueError):
                c.decode_record(raw[:index] + bytes([raw[index] ^ 128]) + raw[index + 1:])

    def test_actual_cpp_prepared_record_and_key_length_bounds(self):
        # Captured from the actual bounded_run_wire.h encoder in the native
        # bridge suite; the SDK double does not implement this serialization.
        raw = bytes.fromhex(
            '44464d52453031330006676f6c64656e0000000a000003e844464d524f303133'
            '0000000000000029000000000000000300000000000c4e64010101'
            '5cf7b762d8ebf97dc893ab87d361b07f6463fa249643bac5adfbd72d0d4f78d8'
            'd7865c7c1b9d4b1fdb1fadaf0988b7fb00000000000000000000000000'
            '7e796c7500fc9ee0a38ea17a018b113afab28fc5514cbf5c1a7c69c2f7016776')
        record = c.decode_record(raw)
        self.assertEqual((record['key'], record['phase'], record['before']['tick']), ('golden', 'prepared', 806500))
        for size in (1, 6, 128):
            raw = reference_record(key='x' * size)
            self.assertEqual(len(raw), 146 + size)
            self.assertEqual(c.decode_record(raw)['key'], 'x' * size)
            with self.assertRaises(c.Rejected): c.decode_record(raw + b'\0')
            with self.assertRaises(c.Rejected): c.decode_record(raw[:-1])

    def test_special_file_rejected_without_blocking(self):
        os.mkfifo(self.path, 0o600)
        started = time.monotonic()
        with self.assertRaises(c.Rejected), c.capsule(self.path): pass
        self.assertLess(time.monotonic() - started, 1)

    def test_absent_record_remains_unknown_and_never_recommits(self):
        with NativeDouble(absent_query=True) as server, c.Client(server.address, TOKEN, 5000) as client:
            result = c.recover(client, intent(server.address), wait_ms=1000)
            self.assertEqual(result['effect_status'], 'unknown_absent_record_is_not_nonapplication')
            self.assertNotIn('record', result)
            self.assertEqual(server.calls, ['Handshake', 'QueryRun'])
            self.assertEqual(server.effects, 0)

    def test_foreground_wait_only_repeats_queries(self):
        with NativeDouble(query_running=2) as server, c.Client(server.address, TOKEN, 5000) as client:
            result = c.recover(client, intent(server.address), wait_ms=1500)
            self.assertEqual(result['record']['phase'], 'stopped')
            self.assertEqual(server.calls, ['Handshake', 'QueryRun', 'QueryRun', 'QueryRun'])
            self.assertEqual(server.effects, 0)
        with NativeDouble(query_running=100) as server, c.Client(server.address, TOKEN, 5000) as client:
            result = c.recover(client, intent(server.address), wait_ms=1)
            self.assertEqual(result['record']['phase'], 'running')
            self.assertTrue(result['wait_expired']); self.assertEqual(server.effects, 0)

    def test_all_phase_reason_flag_combinations(self):
        valid = {(0, 0, 0, 0, 0), (1, 0, 1, 0, 1), (5, 7, 1, 0, 0)}
        valid |= {(4, reason, 0, 0, 0) for reason in (3, 7, 9)}
        valid |= {(2, reason, 1, 0, known) for reason in (1, 2, 3, 5, 6, 8) for known in (0, 1)}
        valid |= {(3, reason, 1, 1, known) for reason in (1, 2, 3, 4, 5, 6, 8) for known in (0, 1)}
        for phase in range(6):
            for reason in range(10):
                for bits in range(8):
                    state = (phase, reason, bits & 1, (bits >> 1) & 1, (bits >> 2) & 1)
                    raw = reference_record(phase=phase, reason=reason, attempted=state[2], paused=state[3], known=state[4])
                    with self.subTest(state=state):
                        if state in valid:
                            c.decode_record(raw)
                        else:
                            with self.assertRaises(c.Rejected): c.decode_record(raw)

    def test_endpoint_and_closed_bounds(self):
        self.assertEqual(c.endpoint('127.0.0.1:5000'), ('127.0.0.1', 5000))
        for address in ('example.com:5000', '192.168.1.1:5000', '127.0.0.1:0', '127.0.0.1:65536', '[::1]:5000'):
            with self.assertRaises(ValueError): c.endpoint(address)
        for ticks, wall in ((0, 1), (1201, 1), (1, 0), (1, 60001), (True, 1)):
            with self.assertRaises(c.Rejected): c.plan_for(ticks, wall, OBS)

    def test_capsule_roundtrip_no_overwrite_and_offline_inspect(self):
        self.create(); before = self.path.read_bytes()
        with c.capsule(self.path) as value:
            self.assertEqual(value, intent())
        with self.assertRaises(FileExistsError), c.capsule(self.path, intent()):
            pass
        self.assertEqual(self.path.read_bytes(), before)
        result = io.StringIO()
        with patch.dict(os.environ, {}, clear=True), patch.object(c, 'Client', side_effect=AssertionError('offline contacted native')), redirect_stdout(result):
            self.assertEqual(c.main(['inspect', '--record', str(self.path)]), 0)
        self.assertEqual(json.loads(result.getvalue())['result']['effect_status'], 'unknown')

    def test_capsule_corruption_partial_and_noncanonical(self):
        self.create(); original = self.path.read_bytes()
        for index, bad in enumerate((original[:-1], original[:-10], original + b' ', b'{}\n', original.replace(b'1000', b'1001'))):
            path = self.directory / f'bad-{index}.json'; path.write_bytes(bad); path.chmod(0o600)
            with self.assertRaises(ValueError), c.capsule(path): pass

    def test_private_modes_links_and_parent_traversal(self):
        self.create(); self.path.chmod(0o400)
        with self.assertRaises(c.Rejected), c.capsule(self.path): pass
        self.path.chmod(0o600)
        link = self.directory / 'symlink'; link.symlink_to(self.path)
        with self.assertRaises(OSError), c.capsule(link): pass
        hard = self.directory / 'hard'; os.link(self.path, hard)
        with self.assertRaises(c.Rejected), c.capsule(hard): pass
        self.directory.chmod(0o755)
        with self.assertRaises(c.Rejected), c.capsule(self.path): pass
        self.directory.chmod(0o700)
        with self.assertRaises(c.Rejected), c.capsule(Path('relative.json')): pass
        with self.assertRaises(c.Rejected), c.capsule(self.directory / '..' / 'intent.json'): pass

    def test_real_tcp_start_query_and_exactly_one_unpause(self):
        with NativeDouble() as server, c.Client(server.address, TOKEN, 5000) as client:
            result = c.start(client, server.address, self.path, 'test', 10, 1000)
            self.assertEqual(result['record']['phase'], 'running')
            with c.capsule(self.path) as value:
                recovered = c.recover(client, value, wait_ms=1000)
            self.assertTrue(recovered['record']['pause_verified']); self.assertEqual(server.effects, 1)
            self.assertEqual(server.calls, ['Handshake', 'ObserveRun', 'PrepareRun', 'CommitRun', 'QueryRun'])

    def test_lost_commit_reconnect_query_never_replays(self):
        with NativeDouble(connections=2, lose_commit=True) as server:
            with c.Client(server.address, TOKEN, 5000) as client, self.assertRaises(c.Rejected):
                c.start(client, server.address, self.path, 'test', 10, 1000)
            with c.capsule(self.path) as value, c.Client(server.address, TOKEN, 5000) as client:
                result = c.recover(client, value)
                self.assertEqual(result['record']['phase'], 'stopped')
            self.assertEqual(server.effects, 1)
            self.assertEqual(server.calls.count('CommitRun'), 1)
            self.assertEqual(server.calls.count('QueryRun'), 1)

    def test_fsync_failure_precedes_every_mutating_request(self):
        actual_sync = os.fsync
        for fail_at in (1, 2):  # File publication and parent-directory durability.
            calls = 0
            def sync(fd):
                nonlocal calls
                calls += 1
                if calls == fail_at:
                    raise OSError('injected sync failure')
                actual_sync(fd)
            with self.subTest(fail_at=fail_at), NativeDouble() as server, c.Client(server.address, TOKEN, 5000) as client:
                with patch.object(c.os, 'fsync', side_effect=sync), self.assertRaises(OSError):
                    c.start(client, server.address, self.directory / f'sync-{fail_at}.json', 'test', 10, 1000)
                self.assertEqual(calls, fail_at)
                self.assertEqual(server.effects, 0); self.assertNotIn('PrepareRun', server.calls); self.assertNotIn('CommitRun', server.calls)

    def test_existing_record_refuses_start_before_prepare(self):
        self.create()
        with NativeDouble() as server, c.Client(server.address, TOKEN, 5000) as client:
            with self.assertRaises(FileExistsError): c.start(client, server.address, self.path, 'test', 10, 1000)
            self.assertNotIn('PrepareRun', server.calls); self.assertEqual(server.effects, 0)

    def test_cancel_does_not_commit(self):
        with NativeDouble() as server, c.Client(server.address, TOKEN, 5000) as client:
            c.start(client, server.address, self.path, 'test', 10, 1000)
            with c.capsule(self.path) as value: result = c.recover(client, value, cancel=True)
            self.assertEqual(result['record']['reason'], 'cancelled'); self.assertEqual(server.effects, 1)
            self.assertEqual(server.calls.count('CancelRun'), 1)

    def test_mismatched_nonce_and_aliased_methods_poison_connection(self):
        for options in ({'nonce_error': True}, {'alias': True}):
            with self.subTest(options=options), NativeDouble(**options) as server, self.assertRaises(c.Rejected):
                c.Client(server.address, TOKEN, 5000)

    def test_notification_flood_is_bounded(self):
        with NativeDouble(notifications=9) as server, self.assertRaises(c.Rejected):
            c.Client(server.address, TOKEN, 5000)

    def test_one_deadline_includes_native_handshake(self):
        start = time.monotonic()
        with NativeDouble(delay=0.05) as server, self.assertRaises((c.Rejected, TimeoutError)):
            c.Client(server.address, TOKEN, 10)
        self.assertLess(time.monotonic() - start, 1)

    def test_invalid_cli_and_opt_in_never_connect(self):
        for args in (['start', '--record', str(self.path)], ['query', '--record', str(self.path), '--ticks', '1'], ['observe']):
            output = io.StringIO()
            with patch.dict(os.environ, {}, clear=True), patch.object(c, 'Client', side_effect=AssertionError('must not connect')), redirect_stdout(output):
                self.assertEqual(c.main(args), 2)
                self.assertEqual(json.loads(output.getvalue())['effect_status'], 'unknown')


if __name__ == '__main__':
    unittest.main(verbosity=2)
