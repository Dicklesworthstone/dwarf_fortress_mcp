"""Joined three-profile TCP fixture; independently encoded map/operations data.

Placement receipts deliberately use the production codec, not a real DFHack
plugin. Every native request is checked, including source, nonce and read order.
"""
from contextlib import contextmanager
from dataclasses import replace
import hashlib
import os
import socket
import struct
import threading
from unittest.mock import patch

import construction_plan as c
from room_furniture_handoff import RoomFurnitureHandoff
import room_readiness as r
import room_readiness_rpc as rpc
import room_readiness_fixtures as f

BUILD_TOKEN, OPS_TOKEN, MAP_TOKEN = b'b' * 32, b'o' * 32, b'm' * 32
PROFILES = {
    b'dfmcp_build_v1_19': ('build', b'dfmcp.build.v1_19', 19, BUILD_TOKEN, f.BUILD),
    b'dfmcp_operations_v1_4': ('operations', b'dfmcp.operations.v1_4', 4, OPS_TOKEN, f.OPERATIONS),
    b'dfmcp_map_v1_5': ('map', b'dfmcp.map.v1_5', 5, MAP_TOKEN, f.MAP),
}


def varint(n):
    out = bytearray()
    while n > 127:
        out.append((n & 127) | 128)
        n >>= 7
    out.append(n)
    return bytes(out)


def encode(fields):
    out = bytearray()
    for key, value in sorted(fields.items()):
        if isinstance(value, bytes):
            out += varint(key * 8 + 2) + varint(len(value)) + value
        else:
            out += varint(key * 8) + varint(value)
    return out


def decode(raw):
    offset = 0
    def integer():
        nonlocal offset
        n, shift = 0, 0
        while True:
            assert offset < len(raw) and shift < 70
            b = raw[offset]
            offset += 1
            n |= (b & 127) << shift
            if not b & 128:
                return n
            shift += 7
    out = {}
    while offset < len(raw):
        tag = integer()
        key, kind = tag >> 3, tag & 7
        assert key not in out
        if kind == 0:
            value = integer()
        else:
            assert kind == 2
            size = integer()
            value = raw[offset:offset + size]
            assert len(value) == size
            offset += size
        out[key] = value
    return out


class Peer:
    def __init__(self, large=False, *, extra_items=0):
        self.server = socket.socket()
        self.server.bind(('127.0.0.1', 0))
        self.server.listen(4)
        self.server.settimeout(.05)
        self.address = self.server.getsockname()
        self.address_text = f'{self.address[0]}:{self.address[1]}'
        h = f.handoff(f.room(large), self.address_text)
        self.goal = r.Goal(h, c.Goal(f.receipts(h), 10000, stable_span=10))
        self.tick, self.extra_items = 300, extra_items
        self.map_overrides, self.ops_options = {}, {}
        self.fault = None
        self.callback = None
        self.connections, self.fragments = 0, 0
        self.calls, self.binds, self.errors = [], [], []
        self.stop = threading.Event()
        self.connection_closed = threading.Event()
        self.active = None
        self.thread = threading.Thread(target=self.serve)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.stop.set()
        if self.active is not None:
            try:
                self.active.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        self.thread.join(3)
        self.server.close()
        assert not self.thread.is_alive(), 'readiness test peer failed to join'
        assert not self.errors, self.errors

    @staticmethod
    def read(sock, size):
        raw = bytearray()
        while len(raw) < size:
            part = sock.recv(size - len(raw))
            if not part:
                raise EOFError()
            raw += part
        return bytes(raw)

    def send(self, sock, raw):
        for start in range(0, len(raw), 113):
            sock.sendall(raw[start:start + 113])
            self.fragments += 1

    def reply(self, sock, value):
        payload = encode(value)
        self.send(sock, struct.pack('<h2xi', -1, len(payload)) + payload)

    def serve(self):
        while not self.stop.is_set():
            try:
                sock, _ = self.server.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            self.active = sock
            self.connection_closed.clear()
            self.connections += 1
            sock.settimeout(3)
            sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            try:
                with sock:
                    self.session(sock)
            except (EOFError, ConnectionError, OSError):
                pass
            except BaseException as error:
                self.errors.append(repr(error))
            finally:
                self.active = None
                self.connection_closed.set()

    def session(self, sock):
        assert self.read(sock, 12) == b'DFHack?\n' + struct.pack('<i', 1)
        self.send(sock, b'DFHack!\n' + struct.pack('<i', 1))
        methods, nonce, queries, maps, offset, released = {}, None, 0, 0, 0, False
        records = self.goal.condition.receipts
        region = self.goal.terrain_goal().region
        map_raw = f.map_capture(self.goal, self.tick, overrides=self.map_overrides)
        ops_raw = f.operations(self.goal, self.tick, extra_items=self.extra_items, **self.ops_options)
        token = b'R' * 16
        while not self.stop.is_set():
            method, width = struct.unpack('<h2xi', self.read(sock, 8))
            assert 1 <= width <= 2048
            fields = decode(self.read(sock, width))
            if method == 0:
                assert set(fields) == {1, 2, 3, 4}
                profile = PROFILES[fields[4]]
                family, package, *_ = profile
                name = fields[1].decode()
                assert name in ('Handshake', 'QueryPlacement' if family == 'build' else 'ReadObservation')
                assert fields[2] == package + b'.Request' and fields[3] == package + b'.Reply'
                identity = len(methods) + 2
                methods[identity] = profile, name
                self.binds.append((family, name))
                self.reply(sock, {1: 2 if self.fault == 'alias_map' and family == 'map' else identity})
                continue
            profile, name = methods[method]
            family, _, minor, credential, manifest = profile
            assert fields[1] == credential and fields[3] == 1 and fields[4] == minor
            assert len(fields[2]) == 32 and (nonce is None or nonce == fields[2])
            nonce = fields[2]
            response = {1: 1, 2: 0, 3: nonce, 4: 1, 5: minor, 6: manifest.generation,
                        7: manifest.df_version.encode(), 8: manifest.dfhack_version.encode()}
            operation = (family, name)
            if family == 'build':
                response.update({12: 0, 13: len(records)})
                if name == 'QueryPlacement':
                    assert queries < 2 * len(records)
                    record = self.goal.condition.goals[queries % len(records)].record
                    assert fields[10] == record.plan.key.encode() and fields[12] == record.plan.digest
                    assert maps == (0 if queries < len(records) else 2)
                    response[10] = record.raw
                    if self.fault == 'receipt_after' and queries == len(records):
                        response[10] = record.raw[:-1] + b'X'
                    queries += 1
            elif family == 'map':
                assert tuple(fields[i] for i in range(5, 11)) == region.origin + region.size
                assert fields[11] == max(1024, 575 + 20 * region.volume)
                if name == 'ReadObservation':
                    assert maps < 2 and queries == len(records)
                    assert not maps or released
                    maps += 1
                    operation = ('map', 'before' if maps == 1 else 'after')
                    response[9] = map_raw
                    if self.fault == 'nonce_map':
                        response[3] = b'X' * 32
                    if self.fault == 'oversize_map':
                        self.send(sock, struct.pack('<h2xi', -1, fields[11] + 1025))
                        return
                    if maps == 2:
                        if self.fault == 'lost_map_after':
                            return
                        if self.fault == 'changed_map':
                            response[9] = map_raw[:-1] + bytes([map_raw[-1] ^ 1])
                        if self.fault == 'map_generation':
                            response[6] += 1
                        if self.fault == 'map_tick':
                            response[9] = f.map_capture(self.goal, self.tick + 1)
                if self.fault == 'map_software':
                    response[7] = b'changed'
            elif name == 'ReadObservation':
                assert queries == len(records) and maps == 1
                assert fields[5] == 4096 and fields[6] == 4096 and fields[7] == 65536
                assert fields[8] == 16 * 1024 * 1024 and fields[11] == 65536
                if fields[12] == 1:
                    assert offset == len(ops_raw) and not released and fields[9] == token and fields[10] == 0
                    released = True
                    operation = ('operations', 'release')
                    response[10] = b'X' * 16 if self.fault == 'release' else token
                else:
                    assert not released and fields[10] == offset and fields[9] == (b'' if offset == 0 else token)
                    part = ops_raw[offset:offset + 65536]
                    response.update({9: part, 10: token, 11: offset, 12: len(ops_raw),
                        13: hashlib.sha256(ops_raw).digest(), 14: int(offset + len(part) == len(ops_raw))})
                    offset += len(part)
                    if self.fault == 'page_digest':
                        response[13] = b'X' * 32
            self.calls.append(operation)
            if self.callback:
                self.callback(operation)
            self.reply(sock, response)


@contextmanager
def environment(peer):
    env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
    env.update({rpc.OPT_IN: '1', rpc.ENDPOINT: peer.address_text,
                rpc.wire.BUILD_TOKEN: BUILD_TOKEN.decode(), rpc.wire.OPERATIONS_TOKEN: OPS_TOKEN.decode(),
                rpc.MAP_TOKEN: MAP_TOKEN.decode()})
    with patch.dict(os.environ, env, clear=True):
        yield
