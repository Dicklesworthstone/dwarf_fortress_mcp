"""Joined furniture/1.19 TCP test peer, not a DFHack plugin or live game.

Exercises real framing, clients and journals. Payloads use the production wire
codec, so these tests are integration evidence, not independent codec proofs.
"""
from dataclasses import replace
import socket
import struct
import threading

import build_placement_rpc as rpc
import build_placement_wire as w


def proto(fields):
    out = bytearray()
    for key, value in sorted(fields.items()):
        if type(value) is bytes:
            out += rpc.varint(key * 8 + 2) + rpc.varint(len(value)) + value
        else:
            out += rpc.varint(key * 8) + rpc.varint(value)
    return bytes(out)


class Peer:
    def __init__(self, handoff):
        self.items = {r.candidate.id: r for r in handoff.allocation.selections}
        self.records, self.placed, self.calls, self.commits = {}, {}, [], []
        self.generation, self.sequence, self.tick = 987, 4, 900
        self.next_building, self.next_job = 300, 200
        self.folder, self.site, self.dimensions = 'region1', 2, (256, 256, 16)
        self.df, self.dfhack = 'test-df', 'test-dfhack'
        self.fault = None
        self.before_reply = None
        self.change = lambda capture: capture
        self.errors, self.connections = [], 0
        self.stop = threading.Event()
        self.server = socket.socket()
        self.server.bind(('127.0.0.1', 0))
        self.server.listen(4)
        self.server.settimeout(.05)
        self.address = self.server.getsockname()
        self.thread = threading.Thread(target=self.serve)
        self.thread.start()

    def capture(self, selected):
        x, y, z = selected.target
        tiles = tuple(w.Tile(2, 42, 3, occupied=(px, py, z) in self.placed,
                            building=self.placed.get((px, py, z)))
                      for py in range(y - 1, y + 2) for px in range(x - 1, x + 2))
        row = self.items[selected.item]
        c = row.candidate
        available = selected.item not in {r.plan.before.selection.item for r in self.records.values()
                                         if r.phase == 'placed'}
        item = w.Item(2, c.position, selected.kind, row.native_type, c.subtype, *c.material,
                      on_ground=available, in_job=not available, ground=w.Tile(2, 42, 3))
        return self.change(w.Capture(self.generation, self.sequence, self.tick, self.site, self.dimensions,
            self.next_building, self.next_job, len(self.placed), self.folder, True,
            selected.target not in self.placed, True, selected, tiles, item))

    @staticmethod
    def read(sock, size):
        raw = bytearray()
        while len(raw) < size:
            part = sock.recv(size - len(raw))
            if not part:
                raise EOFError
            raw += part
        return bytes(raw)

    @staticmethod
    def send(sock, raw):
        for offset in range(0, len(raw), 37):
            sock.sendall(raw[offset:offset + 37])

    def serve(self):
        while not self.stop.is_set():
            try:
                sock, _ = self.server.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            self.connections += 1
            with sock:
                sock.settimeout(2)
                sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                try:
                    self.connection(sock)
                except (EOFError, ConnectionError):
                    pass
                except BaseException as error:
                    self.errors.append(error)

    def connection(self, sock):
        assert self.read(sock, 12) == b'DFHack?\n' + struct.pack('<i', 1)
        self.send(sock, b'DFHack!\n' + struct.pack('<i', 1))
        while not self.stop.is_set():
            method, size = struct.unpack('<h2xi', self.read(sock, 8))
            assert 0 <= size <= 2048
            f = rpc.decode(self.read(sock, size))
            if method == 0:
                assert f[2] == b'dfmcp.build.v1_19.Request'
                assert f[3] == b'dfmcp.build.v1_19.Reply' and f[4] == b'dfmcp_build_v1_19'
                payload = proto({1: rpc.METHODS.index(f[1].decode()) + 2})
            else:
                operation = rpc.METHODS[method - 2]
                self.calls.append(operation)
                assert f[1] == b't' * 32 and f[3] == 1 and f[4] == 19
                extra = {}
                if operation == 'ReadPlacement':
                    extra[9] = self.capture(w.Selection(*(f[i] for i in range(5, 10)))).raw
                elif operation == 'PreparePlacement':
                    key = f[10].decode()
                    fresh = key not in self.records
                    if fresh:
                        plan = w.Plan(key, self.capture(w.Selection(*(f[i] for i in range(5, 10)))))
                        assert f[11] == plan.before.witness and f[12] == plan.digest
                        self.records[key] = w.Record(plan, 'prepared', 'none')
                    extra[10], extra[11] = self.records[key].raw, int(not fresh)
                elif operation in ('CommitPlacement', 'QueryPlacement', 'CancelPlacement'):
                    key = f[10].decode()
                    record = self.records.get(key)
                    if record is not None:
                        assert record.plan.digest == f[12]
                        if operation != 'QueryPlacement':
                            assert f[13] == record.plan.token
                        if operation == 'CommitPlacement':
                            assert key not in self.commits, 'duplicate native placement attempt'
                            assert record.phase == 'prepared'
                            self.commits.append(key)
                            before = record.plan.before
                            assert before.raw == self.capture(before.selection).raw
                            insertion = w.Insertion(before.next_building, before.next_job, before.selection.item,
                                before.selection.kind, before.selection.target, before.item.material,
                                before.item.material_index, 0, 3, True, True, True, False)
                            record = w.Record(record.plan, 'placed', 'none', before.expected_after(), insertion)
                            self.placed[before.selection.target] = before.next_building
                            self.next_building += 1
                            self.next_job += 1
                            self.sequence += 1
                        elif operation == 'CancelPlacement' and record.phase == 'prepared':
                            record = w.Record(record.plan, 'cancelled', 'cancelled')
                        self.records[key] = record
                        if not (operation == 'QueryPlacement' and self.fault == 'missing'):
                            extra[10] = record.raw
                response = {1: 1, 2: 0, 3: f[2], 4: 1, 5: 19, 6: self.generation,
                            7: self.df.encode(), 8: self.dfhack.encode(), 12: 0, 13: len(self.records), **extra}
                if self.before_reply:
                    self.before_reply(operation)
                if (operation, self.fault) in (('CommitPlacement', 'lost_commit'), ('PreparePlacement', 'lost_prepare')):
                    return
                payload = proto(response)
            self.send(sock, struct.pack('<h2xi', -1, len(payload)) + payload)

    def close(self):
        self.stop.set()
        self.server.close()
        self.thread.join(5)
        assert not self.thread.is_alive(), 'peer failed to drain'
        if self.errors:
            raise self.errors[0]
