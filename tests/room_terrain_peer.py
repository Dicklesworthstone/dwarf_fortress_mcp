"""Joined, fragmented, strict map/1.5 TCP fixture. NOT a DFHack plugin or game."""
from __future__ import annotations

import socket
import struct
import threading
import time

import excavation_observer as e
from room_terrain_fixtures import MANIFEST

TOKEN = b'room-survey-fixture-token-0123456789'


def _varint(value):
    out = bytearray()
    while value > 127:
        out.append(128 | (value & 127))
        value >>= 7
    return bytes(out + bytes([value]))


def _encode(fields):
    out = bytearray()
    for key, value in sorted(fields.items()):
        if isinstance(value, bytes):
            out += _varint(key * 8 + 2) + _varint(len(value)) + value
        else:
            out += _varint(key * 8) + _varint(value)
    return bytes(out)


def _decode(raw):
    offset = 0
    def integer():
        nonlocal offset
        value, shift = 0, 0
        while True:
            assert offset < len(raw) and shift < 70
            byte = raw[offset]
            offset += 1
            value |= (byte & 127) << shift
            if not byte & 128:
                return value
            shift += 7
    result = {}
    while offset < len(raw):
        tag = integer()
        key, kind = tag >> 3, tag & 7
        assert key not in result
        if kind == 0:
            result[key] = integer()
        else:
            assert kind == 2
            length = integer()
            assert offset + length <= len(raw)
            result[key] = raw[offset:offset + length]
            offset += length
    return result


class Peer:
    def __init__(self, raw, region, *, fault=None, manifest=MANIFEST,
                 on_handshake=None, on_observation=None, delay=0):
        self.raw, self.region, self.fault, self.manifest = raw, region, fault, manifest
        self.on_handshake, self.on_observation, self.delay = on_handshake, on_observation, delay
        self.accepted = self.handshakes = self.observations = self.fragments = 0
        self.binds, self.requests, self.errors = [], [], []
        self.stop = threading.Event()
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(4)
        self.listener.settimeout(0.05)
        self.address = f'127.0.0.1:{self.listener.getsockname()[1]}'
        self.connection = None
        self.thread = threading.Thread(target=self.serve)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.stop.set()
        if self.connection is not None:
            try:
                self.connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        self.thread.join(3)
        self.listener.close()
        assert not self.thread.is_alive(), 'fixture peer did not drain'
        assert not self.errors, self.errors

    @staticmethod
    def read(connection, size):
        out = bytearray()
        while len(out) < size:
            part = connection.recv(size - len(out))
            if not part:
                raise EOFError
            out.extend(part)
        return bytes(out)

    def send(self, connection, raw):
        # Force fragmented writes, including the greeting and every frame header.
        offset = 0
        for width in (1, 2, 5, 13):
            part = raw[offset:offset + width]
            if part:
                connection.sendall(part)
                self.fragments += 1
                offset += len(part)
        while offset < len(raw):
            part = raw[offset:offset + 257]
            connection.sendall(part)
            self.fragments += 1
            offset += len(part)

    def reply(self, connection, fields):
        payload = _encode(fields)
        self.send(connection, struct.pack('<h2xi', -1, len(payload)) + payload)

    def serve(self):
        while not self.stop.is_set():
            try:
                connection, _ = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            self.connection = connection
            self.accepted += 1
            connection.settimeout(2)
            connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            try:
                with connection:
                    self.session(connection)
            except (EOFError, BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                pass  # Client-side refusal or intentional lost reply.
            except OSError as error:
                if not self.stop.is_set():
                    self.errors.append(repr(error))
            except BaseException as error:
                self.errors.append(repr(error))
            finally:
                self.connection = None

    def session(self, connection):
        assert self.read(connection, 12) == b'DFHack?\n' + struct.pack('<i', 1)
        self.send(connection, b'DFHack!\n' + struct.pack('<i', 1))
        methods = []
        nonce = None
        while not self.stop.is_set():
            method, size = struct.unpack('<h2xi', self.read(connection, 8))
            assert 1 <= size <= 2048
            fields = _decode(self.read(connection, size))
            if method == 0:
                index = len(methods)
                assert index < 2
                name = ('Handshake', 'ReadObservation')[index]
                assert fields == {1: name.encode(), 2: b'dfmcp.map.v1_5.Request',
                                  3: b'dfmcp.map.v1_5.Reply', 4: b'dfmcp_map_v1_5'}
                self.binds.append(name)
                identity = 2 if self.fault == 'alias_binding' else index + 2
                methods.append(identity)
                self.reply(connection, {1: identity})
                continue
            assert len(methods) == 2 and method in (2, 3)
            assert set(fields) == set(range(1, 12))
            assert fields[1] == TOKEN and fields[3] == 1 and fields[4] == 5
            assert isinstance(fields[2], bytes) and len(fields[2]) == 32
            assert nonce is None or nonce == fields[2]
            nonce = fields[2]
            assert tuple(fields[i] for i in range(5, 11)) == self.region.origin + self.region.size
            assert fields[11] == max(1024, 575 + 20 * self.region.volume)
            self.requests.append(method)
            response = {1: 1, 2: 0, 3: nonce, 4: 1, 5: 5, 6: self.manifest.generation,
                        7: self.manifest.df_version.encode(), 8: self.manifest.dfhack_version.encode()}
            if method == 2:
                self.handshakes += 1
                assert self.handshakes == 1
                if self.on_handshake:
                    self.on_handshake()
                if self.fault == 'refuse_handshake':
                    response[1], response[2] = 0, 1
                if self.fault == 'handshake_payload':
                    response[9] = self.raw
                self.reply(connection, response)
                continue
            self.observations += 1
            assert self.observations == 1
            if self.on_observation:
                self.on_observation()
            if self.delay:
                time.sleep(self.delay)
            if self.fault == 'lost_reply':
                return
            response[9] = self.raw
            if self.fault == 'nonce':
                response[3] = b'x' * 32
            if self.fault == 'generation':
                response[6] += 1
            if self.fault == 'software':
                response[7] = b'changed-software'
            if self.fault == 'profile':
                response[5] = 6
            if self.fault == 'native_refusal':
                response[1], response[2] = 0, 3
                del response[9]
            if self.fault == 'notifications':
                for _ in range(9):
                    self.send(connection, struct.pack('<h2xi', -3, 1) + b'x')
            if self.fault == 'oversized_header':
                self.send(connection, struct.pack('<h2xi', -1, e.MAX_CAPTURE + 1025))
                return
            payload = _encode(response)
            if self.fault == 'duplicate_field':
                payload += b'\x08\x01'
            header = struct.pack('<h2xi', -1, len(payload))
            if self.fault == 'truncated_reply':
                self.send(connection, header + payload[:-1])
                return
            self.send(connection, header + payload)
