"""Joined bounded synthetic TCP peer for the real map/1.5 client. No DFHack."""
from __future__ import annotations
import socket
import struct
import threading

from room_layout_capture_fixtures import MANIFEST, capture, tile
import excavation_observer as native

TOKEN = 'fixture-map-query-token-0123456789abcdef'


class Peer:
    def __init__(self, region, *, mode='ok', changes=None, default=None, capture_options=None):
        self.region, self.mode = region, mode
        self.changes = {(10, 10, 2): tile(3)} if changes is None else changes
        self.payload = capture(region, self.changes, default=default, **(capture_options or {}))
        self.sock = socket.socket()
        self.sock.bind(('127.0.0.1', 0))
        self.sock.listen(2)
        self.sock.settimeout(5)
        self.address = '127.0.0.1:' + str(self.sock.getsockname()[1])
        self.calls, self.errors, self.connections = [], [], 0
        self.closed = threading.Event()
        self.worker = threading.Thread(target=self.run)

    def __enter__(self):
        self.worker.start()
        return self

    def __exit__(self, *_):
        self.worker.join(7)
        self.sock.close()
        if self.worker.is_alive():
            raise AssertionError('synthetic map peer did not quiesce')
        if self.errors:
            raise AssertionError(self.errors)

    @staticmethod
    def check(condition, message):
        if not condition:
            raise AssertionError(message)

    @staticmethod
    def read(stream, count):
        out = b''
        while len(out) < count:
            chunk = stream.recv(count - len(out))
            if not chunk:
                raise AssertionError('client closed before expected request')
            out += chunk
        return out

    def frame(self, stream):
        method, size = struct.unpack('<h2xi', self.read(stream, 8))
        self.check(0 <= size <= 2048, 'client request exceeded bound')
        return method, native.decode(self.read(stream, size), 11)

    @staticmethod
    def reply(stream, fields):
        raw = native.encode(fields)
        stream.sendall(struct.pack('<h2xi', -1, len(raw)) + raw)

    def ended(self, stream):
        self.check(stream.recv(1) == b'', 'unexpected second read or other native request')
        self.closed.set()

    def run(self):
        try:
            with self.sock.accept()[0] as stream:
                self.connections += 1
                stream.settimeout(5)
                self.check(self.read(stream, 12) == b'DFHack?\n' + struct.pack('<i', 1), 'wrong greeting')
                stream.sendall(b'DFHack!\n' + struct.pack('<i', 1))
                for index, name in enumerate(('Handshake', 'ReadObservation')):
                    method, fields = self.frame(stream)
                    self.check(method == 0 and fields == {
                        1: name.encode(), 2: b'dfmcp.map.v1_5.Request',
                        3: b'dfmcp.map.v1_5.Reply', 4: b'dfmcp_map_v1_5'}, 'wrong fixed binding')
                    self.reply(stream, {1: 2 if self.mode == 'alias' else index + 2})
                    if self.mode == 'alias' and index == 1:
                        self.ended(stream)
                        return
                nonce = None
                for index, name in enumerate(('Handshake', 'ReadObservation')):
                    method, fields = self.frame(stream)
                    self.check(method == index + 2 and set(fields) == set(range(1, 12)), 'wrong native invocation')
                    self.check(fields[1] == TOKEN.encode(), 'wrong fixture token')
                    self.check(fields[3] == 1 and fields[4] == 5, 'wrong requested protocol')
                    self.check(tuple(fields[i] for i in range(5, 11)) == self.region.origin + self.region.size,
                               'request did not select the exact complete padded region')
                    self.check(fields[11] == max(1024, 575 + 20 * self.region.volume), 'wrong payload bound')
                    self.check(type(fields[2]) is bytes and len(fields[2]) == 32, 'wrong nonce')
                    self.check(nonce is None or fields[2] == nonce, 'changed nonce within one read')
                    nonce = fields[2]
                    self.calls.append(name)
                    response = {1: 1, 2: 0, 3: nonce, 4: 1, 5: 5, 6: MANIFEST.generation,
                                7: MANIFEST.df_version.encode(), 8: MANIFEST.dfhack_version.encode()}
                    if index:
                        if self.mode == 'lost':
                            self.closed.set()
                            return
                        response[9] = self.payload
                        if self.mode == 'nonce': response[3] = b'x' * 32
                        elif self.mode == 'protocol': response[5] = 6
                        elif self.mode == 'generation': response[6] += 1
                        elif self.mode == 'refused':
                            response.pop(9); response.update({1: 0, 2: 1})
                        elif self.mode == 'malformed': response[9] = self.payload + b'x'
                        elif self.mode == 'notification_limit':
                            for _ in range(9):
                                stream.sendall(struct.pack('<h2xi', -3, 0))
                            self.ended(stream)
                            return
                        elif self.mode == 'truncated':
                            encoded = native.encode(response)
                            stream.sendall(struct.pack('<h2xi', -1, len(encoded)) + encoded[:-3])
                            self.closed.set()
                            return
                    self.reply(stream, response)
                self.ended(stream)
        except BaseException as cause:
            self.errors.append(repr(cause))
