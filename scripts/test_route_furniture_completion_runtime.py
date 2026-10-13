"""Actual query-only TCP clients and monitor CLI with injected original Batch API.

Independent framing/operations encoder; strict receipt fixture codec. No live
DFHack, real SDK or complete private route-parent pipeline is exercised here.
"""
from contextlib import contextmanager
from dataclasses import replace
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
import unittest
from unittest.mock import patch

import construction_monitor_rpc as wire
from construction_plan_rpc import Budget
import route_furniture_completion_store as store
import track_route_furniture_completion as cli
from test_route_furniture_completion import fixture, operations, Parent, BATCH_ID

SECRET_BUILD, SECRET_OPS = b'b' * 32, b'o' * 32
BINDINGS = [('dfmcp_build_v1_19', 'dfmcp.build.v1_19', 'Handshake'),
            ('dfmcp_build_v1_19', 'dfmcp.build.v1_19', 'QueryPlacement'),
            ('dfmcp_operations_v1_4', 'dfmcp.operations.v1_4', 'Handshake'),
            ('dfmcp_operations_v1_4', 'dfmcp.operations.v1_4', 'ReadObservation')]


def message(values):
    def varint(n):
        out = bytearray()
        while n > 127:
            out.append((n % 128) + 128)
            n //= 128
        out.append(n)
        return bytes(out)
    result = bytearray()
    for key, value in sorted(values.items()):
        if isinstance(value, bytes):
            result += varint(8 * key + 2) + varint(len(value)) + value
        else:
            result += varint(8 * key) + varint(value)
    return bytes(result)


def parse(raw):
    at, result = 0, {}
    def number():
        nonlocal at
        value, shift = 0, 0
        while True:
            byte = raw[at]
            at += 1
            value += (byte & 127) << shift
            if byte < 128:
                return value
            shift += 7
            assert shift <= 63
    while at < len(raw):
        tag = number()
        key, kind = divmod(tag, 8)
        assert key not in result
        if kind == 0:
            value = number()
        else:
            assert kind == 2
            length = number()
            value = raw[at:at + length]
            at += length
        result[key] = value
    assert at == len(raw)
    return result


class Peer:
    def __init__(self, goal, *, connections=1, fault=None, extra_items=0, fixed_tick=None):
        self.goal, self.connections, self.fault = goal, connections, fault
        self.extra_items, self.fixed_tick = extra_items, fixed_tick
        self.calls, self.requests, self.errors = [], [], []
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(4)
        self.listener.settimeout(3)
        self.address = self.listener.getsockname()
        self.thread = threading.Thread(target=self.run, daemon=False)
        self.by_key = {child.record.plan.key.encode('ascii'): child.record for child in goal.condition.goals}

    @staticmethod
    def read(sock, length):
        data = bytearray()
        while len(data) < length:
            value = sock.recv(length - len(data))
            if not value:
                raise EOFError
            data += value
        return bytes(data)

    def run(self):
        try:
            for connection in range(self.connections):
                with self.listener.accept()[0] as sock:
                    sock.settimeout(3)
                    assert self.read(sock, 12) == b'DFHack?\n' + struct.pack('<i', 1)
                    sock.sendall(b'DFHack!\n' + struct.pack('<i', 1))
                    bound, nonce, queries = [], None, 0
                    raw = operations(self.goal, self.fixed_tick or 100 + connection, extra_items=self.extra_items)
                    token = b'z' * 16
                    try:
                        while True:
                            method, size = struct.unpack('<h2xi', self.read(sock, 8))
                            assert 0 <= size <= 2048
                            request = parse(self.read(sock, size))
                            self.requests.append(request)
                            if method == 0:
                                plugin, package, name = BINDINGS[len(bound)]
                                assert request == {1: name.encode(), 2: (package + '.Request').encode(),
                                                   3: (package + '.Reply').encode(), 4: plugin.encode()}
                                bound.append(name)
                                reply = {1: 20 + len(bound)}
                                label = 'bind:' + name
                            else:
                                assert len(bound) == 4 and 21 <= method <= 24
                                index = method - 21
                                minor = 19 if index < 2 else 4
                                credential = SECRET_BUILD if minor == 19 else SECRET_OPS
                                assert request[1] == credential and request[3] == 1 and request[4] == minor
                                assert len(request[2]) == 32 and nonce in (None, request[2])
                                nonce = request[2]
                                generation = 7 if minor == 19 else 13
                                if self.fault == 'incarnation' and minor == 4:
                                    generation = 14
                                reply = {1: 1, 2: 0, 3: nonce, 4: 1, 5: minor, 6: generation,
                                         7: b'test-df', 8: b'test-dfhack'}
                                if minor == 19:
                                    reply.update({12: 0, 13: len(self.by_key)})
                                else:
                                    assert all(request[n] == value for n, value in
                                               ((5,4096),(6,4096),(7,65536),(8,16777216),(11,65536)))
                                label = BINDINGS[index][2]
                                if index == 1:
                                    assert set(request) == {1,2,3,4,10,12}
                                    record = self.by_key[request[10]]
                                    assert request[12] == record.plan.digest
                                    queries += 1
                                    receipt = record.raw
                                    if self.fault == 'receipt' and queries > len(self.by_key):
                                        receipt = receipt[:-1] + bytes([receipt[-1] ^ 1])
                                    reply[10] = receipt
                                elif index == 3:
                                    assert set(request) == set(range(1,13))
                                    if request[12] == 1:
                                        assert request[9] == token and request[10] == 0
                                        label = 'ReleaseObservation'
                                        if self.fault == 'lost_release':
                                            return
                                        reply[10] = b'x' * 16 if self.fault == 'release' else token
                                    else:
                                        offset = request[10]
                                        assert request[9] == (b'' if offset == 0 else token)
                                        part = raw[offset:offset + 65536]
                                        assert part and offset % 65536 == 0
                                        checksum = hashlib.sha256(raw).digest()
                                        if self.fault == 'digest':
                                            checksum = b'x' * 32
                                        reply.update({9: part, 10: token, 11: offset, 12: len(raw),
                                                      13: checksum, 14: int(offset + len(part) == len(raw))})
                                if self.fault == 'nonce' and index == 3:
                                    reply[3] = b'x' * 32
                            self.calls.append(label)
                            encoded = message(reply)
                            sock.sendall(struct.pack('<h2xi', -1, len(encoded)) + encoded)
                    except (EOFError, ConnectionResetError, BrokenPipeError):
                        pass
        except BaseException as error:
            self.errors.append(error)
        finally:
            self.listener.close()

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, kind, *_):
        self.thread.join(5)
        if self.thread.is_alive():
            self.listener.close()
            self.thread.join(4)
        assert not self.thread.is_alive(), 'test peer leaked a worker'
        if kind is None and self.errors:
            raise self.errors[0]


@contextmanager

def environment(address):
    values = {key: value for key, value in os.environ.items() if not key.startswith('DFMCP_')}
    values.update({wire.OPT_IN: '1', wire.ENDPOINT: f'{address[0]}:{address[1]}',
                   wire.BUILD_TOKEN: SECRET_BUILD.decode(), wire.OPERATIONS_TOKEN: SECRET_OPS.decode()})
    with patch.dict(os.environ, values, clear=True):
        yield


class CompletionRuntimeTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / 'monitor'
        self.goal = fixture()
        self.parent = Parent(self.goal)
        self.parent.close = lambda: None

    def test_actual_read_brackets_every_original_and_releases_all_pages(self):
        with Peer(self.goal, extra_items=3000) as peer, environment(peer.address):
            result = cli.acquire(cli.native.Authority.load(), self.goal, Budget(60000))
            self.assertGreater(len(result.capture), 131072)
            self.assertEqual(result.before_records, self.goal.receipts)
        calls = peer.calls[6:]
        self.assertEqual(calls[:2], ['QueryPlacement'] * 2)
        self.assertEqual(calls[-3:], ['ReleaseObservation', 'QueryPlacement', 'QueryPlacement'])
        self.assertGreaterEqual(calls.count('ReadObservation'), 3)
        self.assertFalse(any('Commit' in value or 'Prepare' in value for value in peer.calls))

    def test_source_receipt_page_and_release_failures_leave_no_sample(self):
        for fault in ('incarnation', 'receipt', 'lost_release', 'release', 'digest', 'nonce'):
            with self.subTest(fault=fault), Peer(self.goal, fault=fault) as peer, environment(peer.address):
                with self.assertRaises((ValueError, OSError)):
                    cli.acquire(cli.native.Authority.load(), self.goal, Budget(60000))
            if fault == 'incarnation':
                self.assertNotIn('QueryPlacement', peer.calls)
            self.assertFalse(any('Commit' in value for value in peer.calls))

    def test_real_monitor_and_tcp_complete_across_reopen_without_placement_authority(self):
        with Peer(self.goal, connections=2) as peer, environment(peer.address):
            self.parent.address = peer.address
            with store.Journal(str(self.path), Budget(60000), writable=True,
                               create=(self.goal, peer.address)) as owner:
                cli.sample_once(owner, self.parent, cli.native.Authority.load(), lambda s: cli.reserve('sample', s))
                self.assertEqual(owner.state.progress.phase, 'candidate')
            with store.Journal(str(self.path), Budget(60000), writable=True) as owner:
                cli.sample_once(owner, self.parent, cli.native.Authority.load(), lambda s: cli.reserve('sample', s))
                self.assertEqual(owner.state.progress.phase, 'satisfied')
        with store.Journal(str(self.path), Budget(60000)) as owner:
            cli.guard(owner, self.parent)
            self.assertEqual(owner.state.progress.observations, 2)
            self.assertFalse(cli.packet('inspect', owner.state)['result']['placement_effects_discharged'])

    def test_cli_start_and_wait_use_original_goal_and_one_current_budget(self):
        with Peer(self.goal, connections=2) as peer, environment(peer.address):
            self.parent.address = peer.address
            def open_parent(path, expected, budget):
                self.assertEqual((path, expected), (self.goal.origin, BATCH_ID))
                self.parent.budget = budget
                return self.parent
            raw = []
            base = ['--journal', str(self.path), '--batch-directory', self.goal.origin, '--batch-id', BATCH_ID]
            with patch.object(cli, 'open_batch', open_parent), \
                 patch.object(cli, 'write_output', lambda value: raw.append(value) or True):
                self.assertEqual(cli.main(['start', *base, '--deadline-tick', '1000']), 0)
                first = json.loads(raw[-1])
                expected = first['result']['goal_digest']
                self.assertEqual(first['result']['progress']['phase'], 'candidate')
                self.assertEqual(cli.main(['wait', *base, '--expected-goal', expected,
                                           '--wait-samples', '2', '--poll-ms', '10']), 0)
                final = json.loads(raw[-1])
                self.assertEqual(final['result']['progress']['phase'], 'satisfied')
                self.assertEqual(final['result']['wait']['samples_published'], 1)
                self.assertEqual(final['result']['original_plan'], self.goal.plan.json())
                self.assertEqual(final['result']['target_count'], 2)
            self.assertEqual(len(raw), 2)

    def test_final_render_revocation_and_custody_loss_do_not_disclose_cached_success(self):
        for mode in ('authority', 'parent'):
            self.path = Path(self.directory.name) / mode
            self.parent.valid = True
            with Peer(self.goal) as peer, environment(peer.address):
                self.parent.address = peer.address
                saved_output = cli.output
                emitted = []
                def output(value):
                    result = saved_output(value)
                    if value['ok'] and value['result']['journal'].get('bytes', 0) not in (0, store.MAX_FILE):
                        if mode == 'authority':
                            os.environ[wire.OPERATIONS_TOKEN] = 'x' * 32
                        else:
                            self.parent.valid = False
                    return result
                args = ['start', '--journal', str(self.path), '--batch-directory', self.goal.origin,
                        '--batch-id', BATCH_ID, '--deadline-tick', '1000']
                with patch.object(cli, 'open_batch', return_value=self.parent), patch.object(cli, 'output', output), \
                     patch.object(cli, 'write_output', lambda value: emitted.append(value) or True):
                    self.assertEqual(cli.main(args), 2)
                self.assertEqual(len(emitted), 1)
                failure = json.loads(emitted[0])
                self.assertFalse(failure['ok'])
                self.assertIsNone(failure['result'])
            with store.Journal(str(self.path), Budget(60000)) as owner:
                self.assertEqual(owner.state.progress.observations, 1)

    def test_precontact_budget_and_endpoint_refusals(self):
        budget = Budget(60000)
        budget.calls = 1
        with environment(('127.0.0.1',5000)), patch.object(cli.native, 'Client') as client:
            with self.assertRaises(ValueError):
                cli.acquire(cli.native.Authority.load(), self.goal, budget)
            client.assert_not_called()
        self.parent.address = ('127.0.0.1',6000)
        with store.Journal(str(self.path), Budget(60000), writable=True,
                           create=(self.goal,('127.0.0.1',5000))) as owner:
            with self.assertRaises(ValueError):
                cli.guard(owner, self.parent)

    def test_real_cli_process_rejects_invalid_overrides_without_source_contact(self):
        result = subprocess.run([sys.executable, str(Path(cli.__file__)), 'inspect',
            '--journal', str(self.path), '--batch-directory', self.goal.origin, '--batch-id', BATCH_ID,
            '--expected-goal', self.goal.digest, '--deadline-tick', '2000'],
            capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 2)
        packet = json.loads(result.stdout)
        self.assertFalse(packet['ok'])
        self.assertIsNone(packet['result'])
        self.assertNotIn(str(self.path).encode(), result.stdout)
        self.assertEqual(result.stderr, b'')
        self.assertFalse(self.path.exists())


if __name__ == '__main__':
    unittest.main()
