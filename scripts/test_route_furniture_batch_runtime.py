"""Real sockets, private journals and CLI processes for original-route dispatch.

The joined peer independently encodes framing, protobuf, map and inventory.
Native placement receipts use the strict fixture codec, not a real DFHack game.
Every connection is joined before the test owner closes; there is no daemon.
"""
from contextlib import contextmanager
from dataclasses import replace
import hashlib
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

from build_placement_wire import Plan, Record
import build_placement_rpc as native
from furniture_plan import canonical
import route_furniture_allocation as allocation
import route_furniture_batch as runtime
from route_furniture_batch_store import Batch
from route_furniture_review import create
from test_route_furniture_frontier import fixture, capture, receipt, source, terrain


def protobuf(fields):
    def varint(value):
        out = bytearray()
        while value > 127:
            out.append((value & 127) | 128)
            value >>= 7
        return bytes(out) + bytes([value])
    out = bytearray()
    for key, value in sorted(fields.items()):
        if isinstance(value, bytes):
            out += varint(key * 8 + 2) + varint(len(value)) + value
        else:
            out += varint(key * 8) + varint(value)
    return bytes(out)


def fields(raw):
    offset, result = 0, {}
    def varint():
        nonlocal offset
        value, shift = 0, 0
        while True:
            assert offset < len(raw) and shift < 70
            byte = raw[offset]
            offset += 1
            value |= (byte & 127) << shift
            if byte < 128:
                return value
            shift += 7
    while offset < len(raw):
        tag = varint()
        key, kind = tag >> 3, tag & 7
        assert key not in result and kind in (0, 2)
        value = varint()
        if kind == 2:
            value, size = raw[offset:offset + value], value
            offset += size
        result[key] = value
    assert offset == len(raw)
    return result


def read(sock, width):
    out = bytearray()
    while len(out) < width:
        data = sock.recv(width - len(out))
        if not data:
            raise EOFError
        out += data
    return bytes(out)


def text(value):
    value = value.encode()
    return struct.pack('>H', len(value)) + value


def inventory(placed, *, flags=64, padding=0):
    jobs = b'DFMJ1200' + struct.pack('>IIBiI', 0, 10, 1, 2, 100 + placed) + text('region1') + struct.pack('>I', 0)
    raw = b'DFMO1400' + struct.pack('>I', len(jobs)) + jobs
    raw += struct.pack('>III', 200 + placed, max(300, 1000 + padding), 0)
    selected = [(10, 30, 'BED', (1, 1, 5)), (20, 31, 'CHAIR', (1, 3, 5))][placed:]
    selected += [(1000 + i, 90, 'BOULDER', (2, 2, 5)) for i in range(padding)]
    raw += struct.pack('>I', len(selected))
    for identity, native_type, kind, position in selected:
        raw += struct.pack('>Ii', identity, native_type) + text(kind)
        raw += struct.pack('>iiiIiiiI', -1, 419, -1, 1, *position, flags) + b'\0\0'
    return raw + struct.pack('>I', 0)


class Peer:
    def __init__(self):
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen()
        self.listener.settimeout(0.05)
        self.address = self.listener.getsockname()
        self.stop = threading.Event()
        self.lock = threading.RLock()
        self.connections, self.workers, self.errors, self.trace = [], [], [], []
        self.placed, self.records, self.commits = 0, {}, 0
        self.fault, self.padding = None, 0
        self.thread = threading.Thread(target=self.accept)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.stop.set()
        self.thread.join(2)
        self.listener.close()
        for sock in self.connections:
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        for thread in self.workers:
            thread.join(2)
        assert not self.thread.is_alive() and not any(t.is_alive() for t in self.workers)
        if self.errors:
            raise AssertionError(self.errors)

    def accept(self):
        while not self.stop.is_set():
            try:
                sock, _ = self.listener.accept()
            except socket.timeout:
                continue
            sock.settimeout(3)
            self.connections.append(sock)
            worker = threading.Thread(target=self.serve, args=(sock,))
            self.workers.append(worker)
            worker.start()

    def serve(self, sock):
        names, nonce, retained, reads, released = {}, None, None, 0, False
        try:
            assert read(sock, 12) == b'DFHack?\n' + struct.pack('<i', 1)
            sock.sendall(b'DFHack!\n' + struct.pack('<i', 1))
            while True:
                method, length = struct.unpack('<h2xi', read(sock, 8))
                assert 0 <= length <= 2048
                request = fields(read(sock, length))
                if method == 0:
                    assert set(request) == {1, 2, 3, 4}
                    plugin = request[4]
                    packages = {b'dfmcp_build_v1_19': b'dfmcp.build.v1_19',
                                b'dfmcp_map_v1_5': b'dfmcp.map.v1_5',
                                b'dfmcp_operations_v1_4': b'dfmcp.operations.v1_4'}
                    package = packages[plugin]
                    assert request[2] == package + b'.Request' and request[3] == package + b'.Reply'
                    name = request[1].decode('ascii')
                    allowed = (('Handshake', 'ReadPlacement', 'PreparePlacement', 'CommitPlacement', 'QueryPlacement', 'CancelPlacement')
                               if plugin == b'dfmcp_build_v1_19' else ('Handshake', 'ReadObservation'))
                    assert name in allowed and (plugin, name) not in names.values()
                    if names:
                        first = next(iter(names.values()))[0]
                        assert (first == b'dfmcp_build_v1_19') == (plugin == b'dfmcp_build_v1_19')
                    identity = len(names) + 2
                    names[identity] = plugin, name
                    reply = {1: identity}
                else:
                    plugin, name = names[method]
                    build, mapping = plugin == b'dfmcp_build_v1_19', plugin == b'dfmcp_map_v1_5'
                    minor = 19 if build else 5 if mapping else 4
                    assert len(names) == (6 if build else 4)
                    assert request[1] == (b'B' * 32 if build else b'M' * 32 if mapping else b'O' * 32)
                    assert request[3] == 1 and request[4] == minor and len(request[2]) == 32
                    nonce = request[2] if nonce is None else nonce
                    assert nonce == request[2]
                    reply = {1: 1, 2: 0, 3: nonce, 4: 1, 5: minor,
                             6: 11 if build else 7 if mapping else 9, 7: b'df', 8: b'dfhack'}
                    with self.lock:
                        self.trace.append((minor, name))
                        fault = self.fault
                        if build:
                            reply.update({12: 0, 13: len(self.records)})
                            if name == 'Handshake':
                                assert set(request) == {1, 2, 3, 4}
                            elif name == 'ReadPlacement':
                                assert set(request) == set(range(1, 10))
                                step = request[5] - 1
                                before = replace(capture(step, sequence=self.placed), next_job=100+self.placed,
                                                 next_building=200+self.placed, building_count=self.placed)
                                assert tuple(request[i] for i in range(5, 10)) == before.selection.values()
                                if fault == 'native_changed_after_prepare' and self.records:
                                    before = replace(before, sequence=before.sequence + 1)
                                reply[9] = before.raw
                            else:
                                key = request[10].decode('ascii')
                                if name == 'PreparePlacement':
                                    before = replace(capture(request[5]-1, sequence=self.placed), next_job=100+self.placed,
                                                     next_building=200+self.placed, building_count=self.placed)
                                    assert request[11] == before.witness and set(request) == set(range(1, 13))
                                    plan = Plan(key, before)
                                    assert request[12] == plan.digest
                                    replayed = key in self.records
                                    self.records.setdefault(key, Record(plan, 'prepared', 'none'))
                                    reply[10], reply[11] = self.records[key].raw, int(replayed)
                                    if fault == 'lost_prepare':
                                        return
                                elif name == 'QueryPlacement':
                                    assert set(request) == {1, 2, 3, 4, 10, 12}
                                    if key in self.records:
                                        assert request[12] == self.records[key].plan.digest
                                        reply[10] = self.records[key].raw
                                else:
                                    assert set(request) == {1, 2, 3, 4, 10, 12, 13}
                                    record = self.records[key]
                                    assert (request[12], request[13]) == (record.plan.digest, record.plan.token)
                                    if name == 'CommitPlacement':
                                        assert record.phase == 'prepared'
                                        self.commits += 1
                                        base = Record.decode(receipt(record.plan.before))
                                        record = Record(record.plan, 'placed', 'none', base.after, base.insertion)
                                        self.placed += 1
                                    else:
                                        assert name == 'CancelPlacement' and record.phase == 'prepared'
                                        record = Record(record.plan, 'cancelled', 'cancelled')
                                    self.records[key] = record
                                    reply[10] = record.raw
                                    if name == 'CommitPlacement' and fault == 'lost_commit':
                                        return
                                reply[13] = len(self.records)
                        elif mapping:
                            assert set(request) == set(range(1, 12))
                            assert tuple(request[i] for i in range(5, 11)) == (1, 1, 5, 5, 3, 1)
                            assert request[11] == 1024
                            if fault == 'map_generation':
                                reply[6] = 99
                            if name == 'ReadObservation':
                                reads += 1
                                assert reads <= 2 and (reads == 1 or released)
                                blocked = (2, 7, 12) if fault == 'block_after_prepare' and self.records else ()
                                if self.placed:
                                    blocked += (4,)
                                reply[9] = terrain(blocked=blocked)
                                if fault == 'map_nonce':
                                    reply[3] = b'X' * 32
                        else:
                            assert all(request[k] == v for k, v in {5:4096, 6:4096, 7:65536,
                                       8:16777216, 11:65536}.items())
                            if name == 'Handshake':
                                assert set(request) == set(range(1, 9)) | {11}
                            else:
                                assert set(request) == set(range(1, 13)) and reads == 1 and not released
                                if request[12]:
                                    assert request[12] == 1 and request[9] == b'T' * 16 and request[10] == 0
                                    released = True
                                    if fault == 'lost_release':
                                        return
                                    reply[10] = (b'F' if fault == 'wrong_release' else b'T') * 16
                                else:
                                    if retained is None:
                                        assert request[9] == b'' and request[10] == 0
                                        retained = inventory(self.placed, flags=65 if fault == 'item_flags' else 64,
                                                             padding=self.padding)
                                    else:
                                        assert request[9] == b'T' * 16
                                    offset = request[10]
                                    assert offset % 65536 == 0 and offset < len(retained)
                                    chunk = retained[offset:offset+65536]
                                    reply.update({9:chunk, 10:b'T'*16, 11:offset, 12:len(retained),
                                                  13:hashlib.sha256(retained).digest(),
                                                  14:int(offset+len(chunk)==len(retained))})
                raw = protobuf(reply)
                sock.sendall(struct.pack('<h2xi', -1, len(raw)) + raw)
        except (EOFError, BrokenPipeError, ConnectionResetError):
            pass
        except OSError as error:
            if not self.stop.is_set():
                self.errors.append(repr(error))
        except BaseException as error:
            self.errors.append(repr(error))
        finally:
            sock.close()

    def environment(self, *, recovery=False):
        env = {k:v for k,v in os.environ.items() if not k.startswith('DFMCP_')}
        endpoint = f'{self.address[0]}:{self.address[1]}'
        if recovery:
            env.update({native.OPT_IN:'1', native.TOKEN:'B'*32, native.ENDPOINT:endpoint})
        else:
            env.update({runtime.OPT_IN:'1', runtime.ENDPOINT:endpoint, runtime.PLACE:'1',
                        native.TOKEN:'B'*32, runtime.MAP_TOKEN:'M'*32, runtime.OPS_TOKEN:'O'*32})
        env['PYTHONDONTWRITEBYTECODE'] = '1'
        return env

    def review(self):
        prior, items, raw = fixture()
        _, request, _, _ = prior.unpack()
        report = allocation.allocate_routes(request, items, raw)
        current = replace(source(raw), endpoint=f'{self.address[0]}:{self.address[1]}')
        return create(request, report, current, {10:30, 20:31})


@contextmanager
def world():
    with Peer() as peer, tempfile.TemporaryDirectory() as folder:
        path = str(Path(folder).resolve() / 'batch')
        os.mkdir(path, 0o700)
        with patch.dict(os.environ, peer.environment(), clear=True):
            auth = runtime.Authority.load(True)
            state = runtime.initialize(path, peer.review(), auth, runtime.Budget())
            yield peer, path, state['batch_id'], auth


class RuntimeTests(unittest.TestCase):
    def test_actual_clients_finish_two_steps_after_reopen_without_substitution(self):
        with world() as (peer, path, identity, auth):
            peer.padding = 1600  # Strict inventory must read multiple pages.
            for expected in ('bed', 'chair'):
                with Batch(path, identity, runtime.Budget()) as batch:
                    review = runtime.review_step(batch, auth)
                    self.assertEqual(review['next_step'], expected)
                with Batch(path, identity, runtime.Budget(), writable=True) as batch:
                    result = runtime.advance(batch, review['expected_plan'], review['confirm_review'], auth)
                    self.assertEqual(result['advanced_step'], expected)
            self.assertEqual(result['status'], 'all_placed')
            self.assertEqual(peer.commits, 2)
            self.assertEqual(len(peer.records), 2)
            self.assertEqual(sum(name == (5, 'ReadObservation') for name in peer.trace), 12)
            with Batch(path, identity, runtime.Budget()) as batch:
                self.assertEqual(batch.audit()['placed'], 2)

    def test_final_route_refusal_keeps_dispatch_uncertain_and_build_only_cancel_works(self):
        with world() as (peer, path, identity, auth):
            with Batch(path, identity, runtime.Budget()) as batch:
                review = runtime.review_step(batch, auth)
            peer.fault = 'block_after_prepare'
            with Batch(path, identity, runtime.Budget(), writable=True) as batch:
                with self.assertRaises(ValueError):
                    runtime.advance(batch, review['expected_plan'], review['confirm_review'], auth)
                self.assertTrue(next(iter(batch.effects.journals.values())).state.dispatched)
            self.assertEqual(peer.commits, 0)
            with patch.dict(os.environ, peer.environment(recovery=True), clear=True):
                with Batch(path, identity, runtime.Budget(), writable=True) as batch:
                    out = runtime.recover(batch, 'bed', native.Authority.load(), cancel=True)
                    self.assertEqual(out['status'], 'halted_cancelled')
                    self.assertFalse(out['advance_allowed'])
            self.assertEqual(peer.commits, 0)

    def test_lost_commit_reply_recovers_original_receipt_without_second_dispatch(self):
        with world() as (peer, path, identity, auth):
            with Batch(path, identity, runtime.Budget()) as batch:
                review = runtime.review_step(batch, auth)
            peer.fault = 'lost_commit'
            with Batch(path, identity, runtime.Budget(), writable=True) as batch:
                with self.assertRaises(ValueError):
                    runtime.advance(batch, review['expected_plan'], review['confirm_review'], auth)
            with patch.dict(os.environ, peer.environment(recovery=True), clear=True):
                with Batch(path, identity, runtime.Budget(), writable=True) as batch:
                    out = runtime.recover(batch, 'bed', native.Authority.load())
                    self.assertEqual(out['placed'], 1)
            self.assertEqual(peer.commits, 1)

    def test_native_release_nonce_and_generation_failures_create_no_child(self):
        for fault in ('wrong_release', 'lost_release', 'map_nonce', 'map_generation'):
            with self.subTest(fault=fault), world() as (peer, path, identity, auth):
                with Batch(path, identity, runtime.Budget()) as batch:
                    review = runtime.review_step(batch, auth)
                peer.fault = fault
                with Batch(path, identity, runtime.Budget(), writable=True) as batch:
                    with self.assertRaises(ValueError):
                        runtime.advance(batch, review['expected_plan'], review['confirm_review'], auth)
                    self.assertEqual(batch.effects.journals, {})
                    self.assertEqual(batch.entries, [])
                self.assertEqual(peer.commits, 0)
                self.assertNotIn((19, 'PreparePlacement'), peer.trace)

    def test_final_native_capture_change_blocks_commit_and_preserves_original_intent(self):
        with world() as (peer, path, identity, auth):
            with Batch(path, identity, runtime.Budget()) as batch:
                reviewed = runtime.review_step(batch, auth)
            peer.fault = 'native_changed_after_prepare'
            with Batch(path, identity, runtime.Budget(), writable=True) as batch:
                with self.assertRaises(ValueError):
                    runtime.advance(batch, reviewed['expected_plan'], reviewed['confirm_review'], auth)
                journal = next(iter(batch.effects.journals.values()))
                self.assertTrue(journal.state.dispatched)
                self.assertEqual(journal.state.plan.digest.hex(), reviewed['expected_plan'])
            self.assertEqual(peer.commits, 0)

    def test_lost_preparation_reply_recovers_without_route_tokens_or_retry(self):
        with world() as (peer, path, identity, auth):
            with Batch(path, identity, runtime.Budget()) as batch:
                reviewed = runtime.review_step(batch, auth)
            peer.fault = 'lost_prepare'
            with Batch(path, identity, runtime.Budget(), writable=True) as batch:
                with self.assertRaises(ValueError):
                    runtime.advance(batch, reviewed['expected_plan'], reviewed['confirm_review'], auth)
                journal = next(iter(batch.effects.journals.values()))
                self.assertFalse(journal.state.dispatched)
                self.assertEqual(len(batch.entries), 1)
            with patch.dict(os.environ, peer.environment(recovery=True), clear=True):
                with Batch(path, identity, runtime.Budget(), writable=True) as batch:
                    queried = runtime.recover(batch, 'bed', native.Authority.load())
                    self.assertEqual(queried['status'], 'pending_recovery')
                    cancelled = runtime.recover(batch, 'bed', native.Authority.load(), cancel=True)
                    self.assertEqual(cancelled['status'], 'halted_cancelled')
            self.assertEqual(peer.commits, 0)
            self.assertEqual(peer.trace.count((19, 'PreparePlacement')), 1)

    def test_existing_read_only_inventory_run_still_accepts_only_its_own_profile(self):
        import route_furniture_inventory as client
        import construction_monitor_rpc as wire
        with Peer() as peer:
            env = {k:v for k,v in peer.environment().items() if not k.startswith('DFMCP_')}
            env.update({client.OPT_IN:'1', client.ENDPOINT:f'{peer.address[0]}:{peer.address[1]}',
                        client.MAP_TOKEN:'M'*32, wire.OPERATIONS_TOKEN:'O'*32})
            with patch.dict(os.environ, env, clear=True):
                _, request, _, _ = peer.review().unpack()
                result = json.loads(client.run(request, client.Region((1,1,5), (5,3,1)),
                                               client.Authority.load(), wire.Budget(10000)))
            self.assertTrue(result['ok'])
            self.assertEqual(result['result']['status'], 'allocated')
            self.assertFalse(any(minor == 19 for minor, _ in peer.trace))

    def test_new_inventory_exclusion_stops_review_without_native_preparation(self):
        with world() as (peer, path, identity, auth):
            peer.fault = 'item_flags'
            with Batch(path, identity, runtime.Budget()) as batch:
                out = runtime.review_step(batch, auth)
                self.assertIsNone(out['confirm_review'])
                self.assertIsNone(out['expected_plan'])
            self.assertEqual(peer.commits, 0)
            self.assertNotIn((19, 'PreparePlacement'), peer.trace)

    def test_original_read_only_environment_still_rejects_combined_credentials(self):
        import route_furniture_inventory as inventory_client
        with world() as (peer, path, identity, auth):
            with self.assertRaises(ValueError):
                inventory_client.Authority.load()
            with self.assertRaises(ValueError):
                native.Authority.load()
            with Batch(path, identity, runtime.Budget()) as batch:
                self.assertIsNotNone(runtime.review_step(batch, auth)['confirm_review'])

    def test_real_cli_init_review_advance_and_offline_inspect(self):
        with Peer() as peer, tempfile.TemporaryDirectory() as folder:
            root = Path(folder).resolve()
            path = root / 'batch'
            path.mkdir(mode=0o700)
            saved = peer.review()
            artifact = root / 'original.json'
            artifact.write_bytes(saved.raw)
            script = Path(runtime.__file__).resolve()
            def cli(op, *args, offline=False):
                env = peer.environment()
                if offline:
                    env = {k:v for k,v in env.items() if not k.startswith('DFMCP_')}
                done = subprocess.run([sys.executable, str(script), op, '--directory', str(path), *args],
                                      capture_output=True, timeout=10, env=env)
                self.assertEqual(done.returncode, 0, (done.stdout, done.stderr))
                self.assertEqual(done.stderr, b'')
                packet = json.loads(done.stdout)
                self.assertTrue(packet['ok'])
                self.assertEqual(done.stdout.count(b'\n'), 1)
                for token in (b'B'*32, b'O'*32, b'M'*32):
                    self.assertNotIn(token, done.stdout)
                return packet['result']
            initial = cli('init', '--review-file', str(artifact), '--expected-review', saved.digest)
            identity = initial['batch_id']
            reviewed = cli('review', '--batch-id', identity)
            advanced = cli('advance', '--batch-id', identity, '--expected-plan', reviewed['expected_plan'],
                           '--confirm-review', reviewed['confirm_review'])
            self.assertEqual(advanced['placed'], 1)
            before = len(peer.trace)
            final = cli('inspect', '--batch-id', identity, offline=True)
            self.assertEqual(final['placed'], 1)
            self.assertEqual(len(peer.trace), before)
            self.assertEqual(artifact.read_bytes(), saved.raw)
            self.assertEqual(peer.commits, 1)


if __name__ == '__main__':
    unittest.main()
