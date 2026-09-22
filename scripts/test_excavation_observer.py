#!/usr/bin/env python3
"""Real Python codec/reducer and joined loopback tests, not live DFHack."""
from __future__ import annotations

import hashlib
from pathlib import Path
import socket
import struct
import threading
import time
import unittest
from unittest.mock import patch

import excavation_observer as e

ROOT = Path(__file__).resolve().parents[1]
REGION = e.Region((15, 15, 2), (2, 2, 1))
MANIFEST = e.Manifest(7, 'df', 'dfhack')
TOKEN = b't' * 32


def raw_capture(tick=100, cells=None, region=REGION, folder='region1', site=1,
                dimensions=(64, 64, 8), paused=True):
    year, current = divmod(tick, 403200)
    raw = b'DFMM1500' + struct.pack('>IIBIH', year, current, int(paused), site, len(folder.encode())) + folder.encode()
    raw += struct.pack('>10I', *dimensions, *region.origin, *region.size, region.volume)
    cells = cells if cells is not None else [visible()] * region.volume
    return raw + b''.join(cells)


def visible(shape=3, depth=0, dig=0, **changes):
    fields = dict(tiletype=99, shape=shape, depth=depth, magma=0, traffic=0, dig=dig,
                  building=0, units=0, walkable=1, temperature1=10015, temperature2=10015)
    fields.update(changes)
    return b'\2' + struct.pack('>IBBBBBBBIHH', *fields.values())


def capture(tick=100, **kwargs):
    return e.decode_capture(raw_capture(tick, **kwargs), MANIFEST, kwargs.get('region', REGION))


def goal(**kwargs):
    return e.Goal(REGION, 'region1', 1, 500, **kwargs)


class Peer:
    """One connection with independent method/shape assertions, always joined."""
    def __init__(self, raw=None, manifest=MANIFEST, fault=None):
        self.raw, self.manifest, self.fault = raw or raw_capture(), manifest, fault
        self.calls, self.errors = [], []
        self.sock = socket.socket()
        self.sock.bind(('127.0.0.1', 0))
        self.sock.listen(1)
        self.sock.settimeout(3)
        self.address = '127.0.0.1:' + str(self.sock.getsockname()[1])
        self.connection = None
        self.thread = threading.Thread(target=self.run)

    @staticmethod
    def exact(sock, n):
        result = bytearray()
        while len(result) < n:
            part = sock.recv(n - len(result))
            if not part:
                raise EOFError
            result += part
        return bytes(result)

    def send(self, raw):
        for offset in range(0, len(raw), 11):
            self.connection.sendall(raw[offset:offset + 11])

    def reply(self, fields):
        raw = e.encode(fields)
        self.send(struct.pack('<h2xi', -1, len(raw)) + raw)

    def run(self):
        try:
            sock, _ = self.sock.accept()
            self.connection = sock
            sock.settimeout(3)
            with sock:
                assert self.exact(sock, 12) == b'DFHack?\n\x01\0\0\0'
                self.send(b'DFHack!\n\x01\0\0\0')
                bindings = 0
                while True:
                    method, n = struct.unpack('<h2xi', self.exact(sock, 8))
                    assert 0 <= n <= 2048
                    fields = e.decode(self.exact(sock, n), 11)
                    if method == 0:
                        assert bindings < 2
                        name = ('Handshake', 'ReadObservation')[bindings]
                        assert fields == {1: name.encode(), 2: b'dfmcp.map.v1_5.Request',
                                          3: b'dfmcp.map.v1_5.Reply', 4: b'dfmcp_map_v1_5'}
                        self.reply({1: 2 if self.fault == 'alias' else bindings + 2})
                        bindings += 1
                        continue
                    assert bindings == 2 and method in (2, 3)
                    self.calls.append(('Handshake', 'ReadObservation')[method - 2])
                    assert set(fields) == set(range(1, 12))
                    assert fields[1] == TOKEN and len(fields[2]) == 32
                    assert (fields[3], fields[4]) == (1, 5)
                    assert tuple(fields[k] for k in range(5, 11)) == REGION.origin + REGION.size
                    assert fields[11] == 1024
                    source = self.manifest
                    response = {1: 1, 2: 0, 3: fields[2], 4: 1, 5: 5, 6: source.generation,
                                7: source.df_version.encode(), 8: source.dfhack_version.encode()}
                    if method == 3:
                        if self.fault == 'drop':
                            return
                        response[9] = self.raw
                        if self.fault == 'nonce':
                            response[3] = b'n' * 32
                        if self.fault == 'source':
                            response[6] += 1
                        if self.fault == 'refuse':
                            response.pop(9)
                            response[1], response[2] = 0, 5
                        if self.fault == 'wire_type':
                            response[9] = 1
                        if self.fault == 'oversize':
                            self.send(struct.pack('<h2xi', -1, e.MAX_CAPTURE + 1025))
                            return
                        if self.fault == 'notifications':
                            for _ in range(9):
                                self.send(struct.pack('<h2xi', -3, 1) + b'x')
                    self.reply(response)
        except (EOFError, BrokenPipeError, ConnectionResetError):
            pass
        except BaseException as cause:
            self.errors.append(cause)
        finally:
            self.sock.close()

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_args):
        if self.connection is not None:
            try:
                self.connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        self.thread.join(5)
        if self.thread.is_alive():
            self.sock.close()
            self.thread.join(5)
        assert not self.thread.is_alive(), 'peer did not quiesce'
        assert not self.errors, repr(self.errors)


class ObserverTests(unittest.TestCase):
    def test_exact_existing_native_fixture(self):
        path = ROOT / 'crates/dfmcp-adapter/tests/fixtures/map_v1_5.hex'
        raw_file = path.read_bytes()
        git = hashlib.sha1(b'blob ' + str(len(raw_file)).encode() + b'\0' + raw_file).hexdigest()
        self.assertEqual(git, 'a2fcaae9b2fd4519241618f68b8aad612601e9bb')
        raw = bytes.fromhex(raw_file.decode().strip())
        value = e.decode_capture(raw, MANIFEST, e.Region((14, 15, 1), (4, 3, 2)))
        self.assertEqual(len(raw), 455)
        self.assertEqual(value.tick, 105 * 403200 + 3)
        self.assertEqual((len(value.tiles), value.folder, value.site), (24, 'region1', 1))
        self.assertEqual(e.classify(value), dict(floor_goal=16, wall=0, other_shape=2,
                         wet_floor=1, designated_floor=0, hidden=1, missing=4, active_designations=0))
        self.assertTrue(all(not t.attributes for t in value.tiles if t.presence != 2))

    def test_all_truncations_trailing_bytes_and_counts_fail(self):
        raw = raw_capture()
        for length in range(len(raw)):
            with self.assertRaises(e.Rejected):
                e.decode_capture(raw[:length], MANIFEST, REGION)
        for bad in (raw + b'\0', raw.replace(b'DFMM1500', b'DFMDG016', 1),
                    raw_capture(cells=[visible()] * 3), raw_capture(cells=[visible()] * 5)):
            with self.assertRaises(e.Rejected):
                e.decode_capture(bad, MANIFEST, REGION)
        with self.assertRaises(e.Rejected):
            e.decode_capture(raw, MANIFEST, e.Region((14, 15, 2), (2, 2, 1)))

    def test_closed_native_fields_and_boundaries(self):
        for field, bad in (('shape', 9), ('depth', 8), ('magma', 2), ('traffic', 4),
                           ('dig', 8), ('building', 8), ('units', 4)):
            with self.assertRaises(e.Rejected):
                capture(cells=[visible(**{field: bad})] * 4)
        for bad in (b'\3', b'\xff', b'\1' + visible()[1:]):
            with self.assertRaises(e.Rejected):
                capture(cells=[bad] * 4)
        for fields in (dict(folder='bad\0folder'), dict(folder='x' * 513),
                       dict(site=2**31), dict(dimensions=(16, 64, 8))):
            with self.assertRaises(e.Rejected):
                capture(**fields)
        self.assertEqual(capture(tick=e.MAX_TICK).tick, e.MAX_TICK)
        raw = bytearray(raw_capture())
        struct.pack_into('>I', raw, 12, 403200)
        with self.assertRaises(e.Rejected):
            e.decode_capture(bytes(raw), MANIFEST, REGION)

    def test_maximum_region_decodes_without_clipping(self):
        region = e.Region((0, 0, 0), (128, 128, 1))
        value = capture(region=region, dimensions=(128, 128, 1))
        self.assertEqual(len(value.tiles), 16384)
        self.assertLess(len(value.raw), e.MAX_CAPTURE)
        for origin, size in (((0, 0, 0), (128, 128, 2)), ((32767, 0, 0), (2, 1, 1)),
                             ((True, 0, 0), (1, 1, 1)), ((0, 0, 0), (0, 1, 1))):
            with self.assertRaises(e.Rejected):
                e.Region(origin, size)

    def test_every_shape_liquid_and_designation_combination(self):
        # 9 * 8 * 8 cells. The goal is exactly dry FLOOR and dig=0.
        for shape in range(9):
            for depth in range(8):
                for dig in range(8):
                    value = capture(cells=[visible(shape, depth, dig)] * 4)
                    counts = e.classify(value)
                    self.assertEqual(counts['floor_goal'], 4 * int((shape, depth, dig) == (3, 0, 0)))
                    self.assertEqual(counts['active_designations'], 4 * int(dig != 0))
                    self.assertEqual(sum(v for k, v in counts.items() if k != 'active_designations'), 4)

    def test_distinct_advancing_ticks_and_sampled_stability(self):
        g = goal()
        state = e.advance(g, None, capture(100))
        self.assertEqual((state.status, state.streak), ('stabilizing', 1))
        for _ in range(20):
            state = e.advance(g, state, capture(100))
            self.assertEqual(state.streak, 1)
        state = e.advance(g, state, capture(109))
        self.assertEqual(state.status, 'stabilizing')
        state = e.advance(g, state, capture(110))
        self.assertEqual((state.status, state.streak, state.since_tick), ('satisfied', 3, 100))
        with self.assertRaises(e.Rejected):
            e.advance(g, state, capture(111))

    def test_hidden_missing_contradictions_and_interruption_reset_streak(self):
        g = goal()
        first = e.advance(g, None, capture(100))
        for cell, status in ((b'\0', 'unknown'), (b'\1', 'unknown'), (visible(2), 'pending'),
                              (visible(3, 1), 'pending'), (visible(3, 0, 1), 'pending')):
            state = e.advance(g, first, capture(105, cells=[cell] + [visible()] * 3))
            self.assertEqual((state.status, state.streak, state.since_tick), (status, 0, None))
            state = e.advance(g, state, capture(110))
            self.assertEqual((state.status, state.streak, state.since_tick), ('stabilizing', 1, 110))
        state = first.interrupted('failed_read')
        self.assertEqual(e.advance(g, state, capture(120)).streak, 1)

    def test_gap_and_deadline_do_not_manufacture_completion(self):
        g = goal(max_gap_ticks=5)
        first = e.advance(g, None, capture(100))
        state = e.advance(g, first, capture(110))
        self.assertEqual((state.status, state.streak, state.since_tick), ('stabilizing', 1, 110))
        self.assertEqual(state.interruption, 'sample_gap_reset')
        first = e.advance(goal(), None, capture(490))
        self.assertEqual(e.advance(goal(), first, capture(500)).status, 'satisfied')
        self.assertEqual(e.advance(goal(), first, capture(501)).status, 'expired')

    def test_source_clock_and_fortress_changes_invalidate_not_complete(self):
        g = goal()
        first = e.advance(g, None, capture(100))
        cases = [capture(99), capture(110, folder='other'), capture(110, site=2),
                 capture(110, dimensions=(65, 64, 8))]
        for source in (e.Manifest(8, 'df', 'dfhack'), e.Manifest(7, 'new-df', 'dfhack')):
            cases.append(e.decode_capture(raw_capture(110), source, REGION))
        for changed in cases:
            self.assertEqual(e.advance(g, first, changed).status, 'invalidated')
        with self.assertRaises(e.Rejected):
            e.advance(g, None, capture(folder='other'))
        self.assertNotEqual(first.latest.witness, cases[-1].witness)

    def test_goal_strict_schema_and_roundtrip(self):
        self.assertEqual(e.Goal.from_json(goal().json()), goal())
        for field, value in (('required_samples', True), ('required_samples', 0),
                             ('max_gap_ticks', 0), ('stable_ticks', -1), ('deadline_tick', e.MAX_TICK + 1)):
            with self.assertRaises(e.Rejected):
                e.Goal.from_json({**goal().json(), field: value})
        with self.assertRaises(e.Rejected):
            e.Goal.from_json({**goal().json(), 'extra': 0})
        with self.assertRaises(e.Rejected):
            e.Goal(e.Region((0, 0, 0), (9, 1, 1)), 'region1', 1, 500)

    def test_actual_fragmented_loopback_is_read_only_and_single_capture(self):
        with Peer() as peer, e.MapClient(peer.address, TOKEN, REGION, 3000) as client:
            self.assertEqual(client.observe(), capture())
            with self.assertRaises(e.Rejected):
                client.observe()
            self.assertTrue(client.closed)
        self.assertEqual(peer.calls, ['Handshake', 'ReadObservation'])

    def test_wire_failures_fence_without_reconnect(self):
        for fault in ('nonce', 'source', 'refuse', 'drop', 'wire_type', 'oversize', 'notifications'):
            with self.subTest(fault=fault), Peer(fault=fault) as peer:
                with e.MapClient(peer.address, TOKEN, REGION, 3000) as client:
                    with self.assertRaises(e.Rejected):
                        client.observe()
                    self.assertTrue(client.closed)
                    with self.assertRaises(e.Rejected):
                        client.observe()
                self.assertEqual(peer.calls, ['Handshake', 'ReadObservation'])
        with Peer(fault='alias') as peer, self.assertRaises(e.Rejected):
            e.MapClient(peer.address, TOKEN, REGION, 3000)

    def test_local_validation_deadline_and_byte_limits_prevent_work(self):
        for address in ('localhost:5000', '8.8.8.8:5000', '127.0.0.1:0', '127.0.0.1:05000'):
            with patch.object(e.socket, 'socket', side_effect=AssertionError('socket opened')):
                with self.assertRaises(e.Rejected):
                    e.MapClient(address, TOKEN, REGION)
        for fault in ('deadline', 'bytes'):
            with Peer() as peer, e.MapClient(peer.address, TOKEN, REGION, 3000) as client:
                if fault == 'deadline':
                    client.deadline = time.monotonic() - 1
                else:
                    client.left = 0
                with self.assertRaises(e.Rejected):
                    client.observe()
                self.assertTrue(client.closed)
            self.assertEqual(peer.calls, ['Handshake'])

    def test_canonical_protobuf_and_overflow_rejection(self):
        self.assertEqual(e.encode({1: 1, 2: b'ab'}), b'\x08\x01\x12\x02ab')
        self.assertEqual(e.decode(b'\x08\x01\x12\x02ab'), {1: 1, 2: b'ab'})
        for raw in (b'\x08\x80\0', b'\x08\1\x08\1', b'\0\0', b'\x0b', b'\x08' + b'\xff' * 10,
                    b'\x12\5ab', b'\x50\0'):
            with self.assertRaises(e.Rejected):
                e.decode(raw)


if __name__ == '__main__':
    unittest.main(verbosity=2)
