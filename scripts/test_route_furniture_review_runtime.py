"""Actual query-only sockets and CLI; independently encoded native test peer."""
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
import time
import unittest
from unittest.mock import patch

from furniture_allocation import Candidate, Request, Slot
from furniture_plan import canonical
import route_furniture_allocation as routes
import route_furniture_inventory as inventory
import route_furniture_review as model
import route_furniture_review_runtime as runtime
import review_route_furniture as cli
from test_route_furniture_review import terrain, source


def text(value):
    value = value.encode()
    return struct.pack('>H', len(value)) + value


def item(identity=42, *, native=1, material=419, subtype=-1, position=(0, 1, 5), flags=64):
    return (struct.pack('>Ii', identity, native) + text('BED')
            + struct.pack('>iiiIiiiIBB', subtype, material, -1, 1, *position, flags, 0, 0))


def roster(*, tick=100, rows=None, padding=0, attached=False, site=2, paused=True, folder='fort'):
    jobs = (b'DFMJ1200' + struct.pack('>IIBiI', tick // 403200, tick % 403200, paused, site, 100)
            + text(folder) + struct.pack('>I', int(attached)))
    if attached:
        jobs += (struct.pack('>Ii', 5, 1) + text('HaulItem') + text('')
                 + struct.pack('>BBiiiBBiII', 0, 0, 0, 0, 5, 0, 0, -1, 1, 1))
    rows = [item()] if rows is None else rows
    rows = rows + [item(identity=100 + n, position=(0, 0, 5)) for n in range(padding)]
    rows.sort(key=lambda raw: struct.unpack('>I', raw[:4])[0])
    raw = (b'DFMO1400' + struct.pack('>I', len(jobs)) + jobs
           + struct.pack('>IIII', 200, 65537, 0, len(rows)) + b''.join(rows)
           + struct.pack('>I', int(attached)))
    if attached:
        raw += struct.pack('>iiii', 5, 42, 0, -1)
    return raw


def varint(number):
    data = bytearray()
    while number > 127:
        data.append((number & 127) | 128)
        number >>= 7
    return bytes(data) + bytes([number])


def encode(fields):
    raw = bytearray()
    for key, value in sorted(fields.items()):
        if isinstance(value, bytes):
            raw += varint(key * 8 + 2) + varint(len(value)) + value
        else:
            raw += varint(key * 8) + varint(value)
    return bytes(raw)


def decode(raw):
    offset, fields = 0, {}
    def number():
        nonlocal offset
        value, shift = 0, 0
        while True:
            byte = raw[offset]
            offset += 1
            value |= (byte & 127) << shift
            if byte < 128:
                return value
            shift += 7
    while offset < len(raw):
        tag = number()
        key, kind = tag >> 3, tag & 7
        assert key not in fields
        if kind == 0:
            fields[key] = number()
        else:
            assert kind == 2
            size = number()
            fields[key] = raw[offset:offset + size]
            offset += size
    return fields


class Peer:
    """Joined test-owned peer. Each connection has one prescribed read bracket."""
    def __init__(self, captures):
        self.captures = captures
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(2)
        self.listener.settimeout(3)
        self.address = self.listener.getsockname()
        self.errors, self.calls, self.releases = [], [], 0
        self.thread = threading.Thread(target=self.run)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.thread.join(5)
        self.listener.close()
        assert not self.thread.is_alive(), 'peer did not quiesce'
        if self.errors:
            raise self.errors[0]

    @staticmethod
    def read(connection, size):
        out = b''
        while len(out) < size:
            part = connection.recv(size - len(out))
            if not part:
                raise EOFError()
            out += part
        return out

    def run(self):
        try:
            for map_raw, operations, fault in self.captures:
                with self.listener.accept()[0] as conn:
                    conn.settimeout(3)
                    try:
                        self.serve(conn, map_raw, operations, fault)
                    except (EOFError, BrokenPipeError, ConnectionResetError):
                        # Expected for source rejection, budget loss and fault cases.
                        pass
        except BaseException as error:
            self.errors.append(error)

    def serve(self, conn, map_raw, operations, fault):
        assert self.read(conn, 12) == b'DFHack?\n' + struct.pack('<i', 1)
        conn.sendall(b'DFHack!\n' + struct.pack('<i', 1))
        bindings = ((b'dfmcp_operations_v1_4', b'dfmcp.operations.v1_4', b'Handshake'),
                    (b'dfmcp_operations_v1_4', b'dfmcp.operations.v1_4', b'ReadObservation'),
                    (b'dfmcp_map_v1_5', b'dfmcp.map.v1_5', b'Handshake'),
                    (b'dfmcp_map_v1_5', b'dfmcp.map.v1_5', b'ReadObservation'))
        methods, nonce, map_reads, offset = {}, None, 0, 0
        retained = b'r' * 16
        while True:
            method, size = struct.unpack('<h2xi', self.read(conn, 8))
            assert 0 <= size <= 2048
            request = decode(self.read(conn, size))
            if method == 0:
                plugin, package, name = bindings[len(methods)]
                assert request == {1: name, 2: package + b'.Request', 3: package + b'.Reply', 4: plugin}
                number = len(methods) + 2
                methods[number] = (plugin, name)
                reply = {1: number}
            else:
                assert method in methods and len(methods) == 4
                plugin, name = methods[method]
                is_map = plugin == b'dfmcp_map_v1_5'
                assert request[1] == (b'm' * 32 if is_map else b'o' * 32)
                assert len(request[2]) == 32 and (nonce is None or request[2] == nonce)
                nonce = request[2]
                assert (request[3], request[4]) == (1, 5 if is_map else 4)
                generation = 8 if fault == 'map_generation' and is_map else 20 if fault == 'inventory_generation' and not is_map else 7 if is_map else 19
                reply = {1: 1, 2: 0, 3: nonce, 4: 1, 5: 5 if is_map else 4,
                         6: generation, 7: b'df', 8: b'dfhack'}
                self.calls.append((plugin, name))
                if is_map:
                    assert {key: request[key] for key in range(5, 11)} == dict(zip(range(5, 11), (0, 0, 5, 5, 3, 1)))
                    assert request[11] == 1024 and set(request) == set(range(1, 12))
                    if name == b'ReadObservation':
                        map_reads += 1
                        assert map_reads <= 2
                        reply[9] = map_raw if fault != 'changed_map' or map_reads == 1 else terrain(tick=101)
                        if fault == 'slow_map':
                            time.sleep(0.15)
                    if fault == 'bad_nonce':
                        reply[3] = b'x' * 32
                else:
                    assert [request[n] for n in (5, 6, 7, 8, 11)] == [4096, 4096, 65536, 16 * 1024 * 1024, 65536]
                    if name == b'ReadObservation':
                        assert map_reads == 1 and set(request) == set(range(1, 13))
                        if request[12]:
                            assert request[12] == 1 and request[9] == retained and request[10] == 0
                            assert offset == len(operations)
                            self.releases += 1
                            if fault == 'lost_release':
                                return
                            reply[10] = b'x' * 16 if fault == 'wrong_release' else retained
                        else:
                            assert request[9] == (b'' if offset == 0 else retained) and request[10] == offset
                            chunk = operations[offset:offset + 65536]
                            reply.update({9: chunk, 10: retained, 11: offset, 12: len(operations),
                                          13: b'x' * 32 if fault == 'bad_digest' else hashlib.sha256(operations).digest(),
                                          14: int(offset + len(chunk) == len(operations))})
                            offset += len(chunk)
                    else:
                        assert name == b'Handshake' and set(request) == set(range(1, 9)) | {11}
            raw = encode(reply)
            conn.sendall(struct.pack('<h2xi', -1, len(raw)) + raw)


@contextmanager
def environment(address):
    env = {key: value for key, value in os.environ.items() if not key.startswith('DFMCP_')}
    env.update(DFMCP_ALLOW_UNADMITTED_ROUTE_FURNITURE='1',
               DFMCP_ROUTE_FURNITURE_ENDPOINT=f'{address[0]}:{address[1]}',
               DFMCP_MAP_TOKEN='m' * 32, DFMCP_OPERATIONS_PAGED_TOKEN='o' * 32,
               PYTHONDONTWRITEBYTECODE='1')
    with patch.dict(os.environ, env, clear=True):
        yield env


def original(address):
    request = Request('fort', 2, (Slot('bed', 'bed', (3, 1, 5), max_distance=8),))
    items = (Candidate(42, 'bed', (0, 1, 5), (419, -1), -1),)
    raw = terrain()
    allocation = routes.allocate_routes(request, items, raw, max_route_steps=4)
    review = model.create(request, allocation, source(raw, endpoint=f'{address[0]}:{address[1]}',
                          inventory_sha256=hashlib.sha256(roster()).hexdigest()), {42: 1})
    return review, request


class RuntimeTests(unittest.TestCase):
    def test_real_create_then_recheck_preserves_original_despite_better_item(self):
        captures = [(terrain(), roster(), None), (terrain(tick=101),
                     roster(tick=101, rows=[item(1, position=(3, 1, 5)), item(position=(1, 1, 5))]), None)]
        with Peer(captures) as peer, environment(peer.address):
            _, request = original(peer.address)
            created = json.loads(runtime.create(request, inventory.Region((0, 0, 5), (5, 3, 1)),
                inventory.Authority.load(), inventory.wire.Budget(10000), max_route_steps=4))
            review = model.Review.decode(canonical(created['review']), created['review_digest'])
            checked = json.loads(runtime.check(review, inventory.Authority.load(), inventory.wire.Budget(10000)))
            self.assertEqual(checked['result']['status'], 'constraints_met')
            self.assertEqual(checked['result']['plan'], created['result']['plan'])
            self.assertEqual(checked['result']['rows'][0]['item'], 42)
            self.assertEqual(checked['result']['rows'][0]['route_steps'], 2)
            self.assertEqual(checked['review_digest'], created['review_digest'])
            self.assertTrue(checked['native_capture_established'])
            self.assertEqual(peer.releases, 2)
            self.assertFalse(checked['agent_turn']['coverage']['placement_gate_enforced'])

    def test_complete_paged_inventory_final_attachment_invalidates_original(self):
        raw = roster(padding=1600, attached=True)
        self.assertGreater(len(raw), 65536)
        with Peer([(terrain(), raw, None)]) as peer, environment(peer.address):
            review, _ = original(peer.address)
            result = json.loads(runtime.check(review, inventory.Authority.load(), inventory.wire.Budget(10000)))
            self.assertEqual(result['result']['status'], 'unestablished')
            self.assertIsNone(result['result']['plan'])
            self.assertEqual(result['result']['rows'][0]['reason'], 'selected_item_missing_or_ineligible')
            self.assertEqual(result['source']['counts']['related_item'], 1)
            self.assertEqual(peer.releases, 1)

    def test_flags_and_changed_material_do_not_accept_other_stock(self):
        for changed, status in ((item(flags=66), 'unestablished'),
                                (item(material=420), 'constraints_not_met'),
                                (item(subtype=3), 'constraints_not_met'),
                                (item(native=2), 'constraints_not_met')):
            with self.subTest(status=status), Peer([(terrain(), roster(rows=[changed]), None)]) as peer, environment(peer.address):
                review, _ = original(peer.address)
                result = json.loads(runtime.check(review, inventory.Authority.load(), inventory.wire.Budget(10000)))
                self.assertEqual(result['result']['status'], status)
                self.assertIsNone(result['result']['plan'])

    def test_source_and_transport_faults_refuse_without_second_acquisition(self):
        for fault in ('map_generation', 'inventory_generation', 'changed_map', 'bad_nonce',
                      'lost_release', 'wrong_release', 'bad_digest'):
            with self.subTest(fault=fault), Peer([(terrain(), roster(), fault)]) as peer, environment(peer.address):
                review, _ = original(peer.address)
                with self.assertRaises((OSError, ValueError)):
                    runtime.check(review, inventory.Authority.load(), inventory.wire.Budget(10000))
                self.assertLessEqual(peer.releases, 1)

    def test_foreign_world_tick_pause_and_clock_regression_refuse(self):
        for map_raw, ops in ((terrain(), roster(site=3)), (terrain(), roster(tick=101)),
                             (terrain(paused=False), roster(paused=False)),
                             (terrain(tick=99), roster(tick=99))):
            with self.subTest(), Peer([(map_raw, ops, None)]) as peer, environment(peer.address):
                review, _ = original(peer.address)
                with self.assertRaises(ValueError):
                    runtime.check(review, inventory.Authority.load(), inventory.wire.Budget(10000))

    def test_create_shortage_returns_no_review_or_partial_plan(self):
        with Peer([(terrain(), roster(rows=[]), None)]) as peer, environment(peer.address):
            _, request = original(peer.address)
            out = json.loads(runtime.create(request, inventory.Region((0, 0, 5), (5, 3, 1)),
                inventory.Authority.load(), inventory.wire.Budget(10000)))
            self.assertEqual(out['result']['status'], 'shortage')
            self.assertIsNone(out['review'])
            self.assertIsNone(out['review_digest'])
            self.assertIsNone(out['result']['plan'])

    def test_source_is_closed_before_projection_and_review_work(self):
        real_decode = runtime.inventory.decode_operations
        real_client = inventory.Client
        clients = []
        class RecordingClient(real_client):
            def __init__(self, *args):
                super().__init__(*args)
                clients.append(self)
        def checked_decode(*args):
            self.assertTrue(clients[0].closed)
            return real_decode(*args)
        with Peer([(terrain(), roster(), None)]) as peer, environment(peer.address):
            review, _ = original(peer.address)
            with patch.object(inventory, 'Client', RecordingClient), patch.object(inventory, 'decode_operations', checked_decode):
                runtime.check(review, inventory.Authority.load(), inventory.wire.Budget(10000))

    def test_revocation_during_review_or_final_render_returns_no_result(self):
        for phase in ('review', 'render'):
            with self.subTest(phase=phase), Peer([(terrain(), roster(), None)]) as peer, environment(peer.address):
                review, _ = original(peer.address)
                owner = model if phase == 'review' else inventory
                name = 'revalidate' if phase == 'review' else 'bounded_output'
                original_function = getattr(owner, name)
                def revoke(*args, **kwargs):
                    result = original_function(*args, **kwargs)
                    os.environ['DFMCP_MAP_TOKEN'] = 'x' * 32
                    return result
                with patch.object(owner, name, revoke), self.assertRaises(ValueError):
                    runtime.check(review, inventory.Authority.load(), inventory.wire.Budget(10000))

    def test_precontact_budget_and_original_endpoint_refusals(self):
        with environment(('127.0.0.1', 5000)):
            review, _ = original(('127.0.0.1', 5000))
            authority = inventory.Authority.load()
            for field, value in (('calls', 9), ('network_bytes', 1), ('work_steps', 1), ('deadline', 0)):
                budget = inventory.wire.Budget(10000)
                setattr(budget, field, value)
                with patch.object(inventory.socket, 'socket') as opened, self.assertRaises(ValueError):
                    runtime.check(review, authority, budget)
                opened.assert_not_called()
            other, _ = original(('127.0.0.1', 5001))
            with patch.object(inventory, 'Client') as opened, self.assertRaises(ValueError):
                runtime.check(other, authority, inventory.wire.Budget(10000))
            opened.assert_not_called()

    def test_real_foreground_timeout_does_not_retry(self):
        with Peer([(terrain(), roster(), 'slow_map')]) as peer, environment(peer.address):
            review, _ = original(peer.address)
            with self.assertRaises((ValueError, OSError)):
                runtime.check(review, inventory.Authority.load(), inventory.wire.Budget(30))
            self.assertEqual(peer.releases, 0)

    def test_real_cli_create_and_check_keep_files_unchanged_and_emit_one_json(self):
        script = str(Path(cli.__file__).resolve())
        with Peer([(terrain(), roster(), None), (terrain(tick=101), roster(tick=101), None)]) as peer, environment(peer.address) as env, tempfile.TemporaryDirectory() as directory:
            _, request = original(peer.address)
            path = Path(directory) / 'request.json'
            path.write_bytes(canonical(request.json()))
            first = subprocess.run([sys.executable, script, 'create', '--request-file', str(path),
                '--origin', '0', '0', '5', '--size', '5', '3', '1', '--max-route-steps', '4'],
                env=env, capture_output=True, timeout=5)
            self.assertEqual(first.returncode, 0, first.stderr.decode())
            self.assertEqual(first.stderr, b'')
            packet = json.loads(first.stdout)
            review_path = Path(directory) / 'original.json'
            review_bytes = canonical(packet['review'])
            review_path.write_bytes(review_bytes)
            second = subprocess.run([sys.executable, script, 'check', '--review-file', str(review_path),
                '--expected-review', packet['review_digest']], env=env, capture_output=True, timeout=5)
            self.assertEqual(second.returncode, 0, second.stderr.decode())
            self.assertEqual(second.stderr, b'')
            result = json.loads(second.stdout)
            self.assertEqual(result['result']['plan'], packet['result']['plan'])
            self.assertEqual(review_path.read_bytes(), review_bytes)
            self.assertEqual(path.read_bytes(), canonical(request.json()))
            self.assertEqual(sorted(p.name for p in Path(directory).iterdir()), ['original.json', 'request.json'])

    def test_cli_bad_identity_and_constraint_overrides_refuse_before_contact(self):
        with tempfile.TemporaryDirectory() as directory, environment(('127.0.0.1', 5000)):
            review, _ = original(('127.0.0.1', 5000))
            path = Path(directory) / 'review.json'
            path.write_bytes(review.raw)
            cases = (['--expected-review', '0' * 64],
                     ['--expected-review', review.digest, '--max-route-steps', '100'],
                     ['--expected-review', review.digest, '--origin', '0', '0', '5'])
            for extra in cases:
                stdout = io.TextIOWrapper(io.BytesIO(), encoding='utf-8')
                with patch.object(inventory, 'Client') as opened, patch.object(cli.sys, 'stdout', stdout):
                    status = cli.main(['check', '--review-file', str(path), *extra])
                    stdout.flush()
                    packet = json.loads(stdout.buffer.getvalue())
                opened.assert_not_called()
                self.assertEqual(status, 2)
                self.assertFalse(packet['ok'])
                self.assertIsNone(packet['result'])
                self.assertIsNone(packet['source'])
                self.assertNotIn(directory, json.dumps(packet))

    def test_input_reader_rejects_symlinks_fifo_oversize_and_changed_file(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'data'
            path.write_bytes(b'{}')
            link = Path(directory) / 'link'
            link.symlink_to(path)
            fifo = Path(directory) / 'pipe'
            os.mkfifo(fifo)
            for bad, maximum in ((link, 10), (fifo, 10), (Path(directory), 10), (path, 1)):
                with self.assertRaises((OSError, ValueError)):
                    cli.read_file(str(bad), maximum, inventory.wire.Budget(10000))
            real_read = os.read
            def changed(fd, count):
                result = real_read(fd, count)
                path.write_bytes(b'{ }')
                return result
            with patch.object(cli.os, 'read', changed), self.assertRaises(ValueError):
                cli.read_file(str(path), 10, inventory.wire.Budget(10000))

    def test_partial_output_never_attempts_a_second_packet(self):
        stream = type('Stream', (), {})()
        calls = []
        stream.buffer = type('Buffer', (), {'write': lambda self, raw: calls.append(raw) or 1,
                                            'flush': lambda self: None})()
        with patch.object(cli.sys, 'stdout', stream):
            self.assertEqual(cli.publish(b'{}\n'), 2)
        self.assertEqual(calls, [b'{}\n'])


if __name__ == '__main__':
    unittest.main()
