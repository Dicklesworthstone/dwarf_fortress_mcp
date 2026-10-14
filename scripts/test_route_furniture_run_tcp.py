"""Actual placement TCP, run CLI, coordinator and private files; map I/O is injected.

The joined test peer is not DFHack. Literal binding/framing checks are independent
of the production wire encoder. All produced Capture/Record values are synthetic.
"""
from __future__ import annotations

from contextlib import contextmanager
import io
import json
import os
from pathlib import Path
import socket
import struct
import tempfile
import threading
import types
import unittest
from unittest.mock import patch

from test_route_furniture_run_integration import Game
import build_placement_rpc as native
from build_placement_wire import Insertion, Plan, Record, Selection
import route_furniture_batch as steps
import route_furniture_run as run


METHODS = ('Handshake', 'ReadPlacement', 'PreparePlacement', 'CommitPlacement',
           'QueryPlacement', 'CancelPlacement')


def encode(values):
    def varint(n):
        out = bytearray()
        while n > 127: out.append((n & 127) | 128); n >>= 7
        return bytes(out + bytes([n]))
    out = bytearray()
    for field, value in sorted(values.items()):
        if isinstance(value, bytes): out += varint(field * 8 + 2) + varint(len(value)) + value
        else: out += varint(field * 8) + varint(value)
    return bytes(out)


def decode(raw):
    offset, out = 0, {}
    def varint():
        nonlocal offset
        n = 0
        for i in range(10):
            assert offset < len(raw)
            b = raw[offset]; offset += 1; n |= (b & 127) << (i * 7)
            if b < 128: return n
        raise AssertionError('varint')
    while offset < len(raw):
        field = varint(); key, wire = field >> 3, field & 7
        assert key not in out and wire in (0, 2)
        n = varint()
        if wire == 0: out[key] = n
        else:
            assert n <= len(raw) - offset
            out[key] = raw[offset:offset+n]; offset += n
    return out


def exact(sock, count):
    raw = bytearray()
    while len(raw) < count:
        part = sock.recv(count - len(raw))
        if not part:
            assert not raw, 'partial request'
            return None
        raw += part
    return bytes(raw)


class Peer:
    def __init__(self, game):
        self.game = game
        self.socket = socket.socket()
        self.socket.bind(('127.0.0.1', 0))
        self.address = self.socket.getsockname()
        self.socket.listen(4)
        self.socket.settimeout(0.1)
        self.stop = threading.Event()
        self.lock = threading.RLock()
        self.children, self.clients, self.errors = [], [], []
        self.writer_keys = set()

    def __enter__(self):
        self.thread = threading.Thread(target=self.accept)
        self.thread.start()
        return self

    def accept(self):
        while not self.stop.is_set():
            try: client, _ = self.socket.accept()
            except socket.timeout: continue
            except OSError: break
            with self.lock:
                if len(self.children) >= 128:
                    self.errors.append(AssertionError('connection bound')); client.close(); break
                self.clients.append(client)
                child = threading.Thread(target=self.serve, args=(client,))
                self.children.append(child)
            child.start()

    def serve(self, client):
        try:
            client.settimeout(5)
            assert exact(client, 12) == b'DFHack?\n\x01\x00\x00\x00'
            client.sendall(b'DFHack!\n\x01\x00\x00\x00')
            bound, permit, nonce = {}, None, None
            while not self.stop.is_set():
                head = exact(client, 8)
                if head is None: break
                method, length = struct.unpack('<h2xi', head)
                assert 0 <= length <= 2048
                body = exact(client, length)
                assert body is not None
                request = decode(body)
                if method == 0:
                    name = request[1].decode('ascii')
                    assert name == METHODS[len(bound)]
                    assert request == {1: name.encode(), 2: b'dfmcp.build.v1_19.Request',
                                       3: b'dfmcp.build.v1_19.Reply', 4: b'dfmcp_build_v1_19'}
                    number = len(bound) + 2; bound[number] = name
                    answer = {1: number}
                else:
                    assert len(bound) == 6 and method in bound
                    name = bound[method]
                    assert request[1] == b't' * 32 and request[3] == 1 and request[4] == 19
                    assert len(request[2]) == 32
                    nonce = request[2] if nonce is None else nonce
                    assert nonce == request[2]
                    with self.lock:
                        g = self.game
                        record, capture, replayed = None, None, None
                        key = request.get(10, b'').decode()
                        g.calls.append((name, key or None))
                        if name == 'Handshake': assert set(request) == {1, 2, 3, 4}
                        elif name == 'ReadPlacement':
                            assert set(request) == set(range(1, 10))
                            capture = g.capture(Selection(*(request[k] for k in range(5, 10))))
                        elif name == 'PreparePlacement':
                            assert set(request) == set(range(1, 13))
                            selected = Selection(*(request[k] for k in range(5, 10)))
                            plan = Plan(key, g.capture(selected))
                            assert request[11] == plan.before.witness and request[12] == plan.digest
                            assert key not in g.records
                            record = Record(plan, 'prepared', 'none')
                            g.records[key], permit, replayed = record, plan, False
                        elif name == 'CommitPlacement':
                            assert set(request) == {1, 2, 3, 4, 10, 12, 13}
                            assert permit is not None and permit.key == key
                            plan, permit = permit, None
                            assert request[12] == plan.digest and request[13] == plan.token
                            assert key not in self.writer_keys
                            g.on_commit(plan)
                            self.writer_keys.add(key)
                            b = plan.before
                            insert = Insertion(b.next_building, b.next_job, b.selection.item,
                                b.selection.kind, b.selection.target, b.item.material,
                                b.item.material_index, 0, 3, True, True, True, False)
                            record = Record(plan, 'placed', 'none', b.expected_after(), insert)
                            g.records[key] = record; g.placed.append(record)
                            if g.fault == 'commit_lost': return
                        elif name == 'QueryPlacement':
                            assert set(request) == {1, 2, 3, 4, 10, 12}
                            record = g.records.get(key)
                            if record is not None: assert request[12] == record.plan.digest
                            permit = None
                        else: raise AssertionError('run sent a cancellation')
                        answer = {1: 1, 2: 0, 3: nonce, 4: 1, 5: 19, 6: 11,
                                  7: b'df', 8: b'dfhack', 12: 0, 13: len(g.records)}
                        if capture is not None: answer[9] = capture.raw
                        if record is not None: answer[10] = record.raw
                        if replayed is not None: answer[11] = int(replayed)
                payload = encode(answer)
                client.sendall(struct.pack('<h2xi', -1, len(payload)) + payload)
        except BaseException as error:
            if not self.stop.is_set(): self.errors.append(error)
        finally:
            client.close()

    def __exit__(self, *_):
        self.stop.set()
        self.socket.close()
        self.thread.join(2)
        for client in self.clients:
            try: client.shutdown(socket.SHUT_RDWR)
            except OSError: pass
        for child in self.children: child.join(2)
        assert not self.thread.is_alive() and all(not t.is_alive() for t in self.children)
        if self.errors: raise self.errors[0]


@contextmanager
def environment(count=2):
    game = Game(count)
    with Peer(game) as peer, tempfile.TemporaryDirectory(prefix='dfmcp-run-tcp-') as path:
        game.address = peer.address
        os.chmod(path, 0o700)
        env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
        env.update({steps.OPT_IN: '1', steps.PLACE: '1', native.TOKEN: 't' * 32,
                    steps.OPS_TOKEN: 'o' * 32, steps.MAP_TOKEN: 'm' * 32,
                    steps.ENDPOINT: f'{peer.address[0]}:{peer.address[1]}'})
        with patch.dict(os.environ, env, clear=True), patch.object(steps, 'acquire_frontier', game.frontier):
            saved = game.review()
            state = steps.initialize(path, saved, steps.Authority.load(), steps.Budget(60000))
            yield game, peer, path, state['batch_id']


def cli(operation, path, identity, *extra):
    output = io.BytesIO()
    with patch.object(steps.sys, 'stdout', types.SimpleNamespace(buffer=output)):
        status = steps.main([operation, '--directory', path, '--batch-id', identity,
                             '--max-steps', '2', '--timeout-ms', '60000', *extra])
    raw = output.getvalue()
    assert len(raw.splitlines()) == 1 and len(raw) <= 256 * 1024
    return status, json.loads(raw)


class TcpRunTests(unittest.TestCase):
    def test_real_tcp_permits_and_journals_execute_exact_two_step_run(self):
        with environment() as (game, peer, path, identity):
            code, proposed = cli('review-run', path, identity)
            self.assertEqual(code, 0, proposed)
            self.assertEqual(len(peer.writer_keys), 0)
            token = proposed['result']['run_review']['confirm_run']
            code, finished = cli('run', path, identity, '--confirm-run', token)
            self.assertEqual(code, 0, finished)
            self.assertEqual(finished['result']['status'], 'all_placed')
            self.assertEqual(len(peer.writer_keys), 2)
            self.assertEqual(len(list((Path(path) / 'effects').iterdir())), 2)
            before = len(game.calls)
            code, refused = cli('run', path, identity, '--confirm-run', token)
            self.assertEqual(code, 2)
            self.assertEqual(len(game.calls), before)
            self.assertFalse(refused['ok'])

    def test_lost_tcp_commit_stops_and_does_not_query_or_reconnect_to_retry(self):
        with environment() as (game, peer, path, identity):
            code, proposed = cli('review-run', path, identity)
            self.assertEqual(code, 0)
            token = proposed['result']['run_review']['confirm_run']
            game.fault = 'commit_lost'
            code, value = cli('run', path, identity, '--confirm-run', token)
            self.assertEqual(code, 2, value)
            self.assertEqual(len(peer.writer_keys), 1)
            self.assertEqual(value['result']['pending_step'], 's00')
            self.assertTrue(value['result']['steps'][0]['dispatch_recorded'])
            self.assertEqual(game.calls[-1][0], 'CommitPlacement')
            before = len(game.calls)
            cli('run', path, identity, '--confirm-run', token)
            self.assertEqual(len(game.calls), before)
            self.assertEqual(len(peer.writer_keys), 1)

    def test_invalid_confirmed_native_frontier_is_read_only_over_tcp(self):
        with environment() as (game, peer, path, identity):
            code, value = cli('run', path, identity, '--confirm-run', '0' * 64)
            self.assertEqual(code, 2)
            self.assertEqual(len(peer.writer_keys), 0)
            self.assertFalse(any(name in ('PreparePlacement', 'CommitPlacement') for name, _ in game.calls))
            self.assertEqual(list((Path(path) / 'effects').iterdir()), [])


if __name__ == '__main__':
    unittest.main()
