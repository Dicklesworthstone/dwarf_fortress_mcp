"""Joined synthetic furniture/1.19 peer for real client/journal regression tests.

No DFHack behavior or live compatibility is certified. Test source bytes pass
through the ordinary RPC, canonical receipt and private-file owner code.
"""
from __future__ import annotations

from dataclasses import replace
from pathlib import Path
import socket
import struct
import sys
import threading

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import build_placement_rpc as rpc
from build_placement_wire import Capture, Insertion, Item, Plan, Record, Selection, Tile

TOKEN = 'synthetic-recovery-test-token-00000000'


def placed(plan: Plan) -> Record:
    before = plan.before
    insertion = Insertion(before.next_building, before.next_job, before.selection.item,
        before.selection.kind, before.selection.target, before.item.material,
        before.item.material_index, 0, 1, True, True, True, False)
    return Record(plan, 'placed', 'none', before.expected_after(), insertion)


def response_bytes(fields: dict) -> bytes:
    # Reply payloads have their own larger bound; rpc.encode is request-only.
    data = bytearray()
    for number, value in sorted(fields.items()):
        if type(value) is bytes:
            data += rpc.varint(number * 8 + 2) + rpc.varint(len(value)) + value
        else:
            data += rpc.varint(number * 8) + rpc.varint(value)
    assert len(data) <= rpc.MAX_REPLY
    return bytes(data)


class Peer:
    """All server threads and sockets belong to, and close with, this context."""
    def __init__(self):
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(8)
        self.listener.settimeout(.1)
        self.address = self.listener.getsockname()
        self.endpoint = f'{self.address[0]}:{self.address[1]}'
        self.generation, self.sequence, self.tick = 7, 0, 100
        self.next_building, self.next_job, self.building_count = 1000, 2000, 30
        self.records, self.calls, self.errors = {}, [], []
        self.commit_mode, self.query_mode = 'placed', 'resolve'
        self.applied = 0
        self.connections = 0
        self.active = None
        self.fragment = 0
        self.hook = None
        self.stopping = threading.Event()
        self.thread = threading.Thread(target=self.serve, name='furniture-recovery-peer')

    def environment(self, *, control=True):
        return {rpc.OPT_IN: '1', rpc.TOKEN: TOKEN, rpc.ENDPOINT: self.endpoint,
                rpc.PLACE: '1' if control else '0'}

    def capture(self, selection: Selection) -> Capture:
        floor = Tile(2, 42, 3)
        item = Item(presence=2, pos=(5 + selection.item, 5, 2), kind=selection.kind,
                    native_type=selection.kind, subtype=-1, material=0, material_index=1,
                    on_ground=True, ground=floor)
        return Capture(self.generation, self.sequence, self.tick, 3, (256, 256, 16),
                       self.next_building, self.next_job, self.building_count,
                       'recovery-fixture', True, True, True, selection, (floor,) * 9, item)

    @staticmethod
    def read(sock, size, *, eof=False):
        data = bytearray()
        while len(data) < size:
            part = sock.recv(size - len(data))
            if not part:
                if eof and not data:
                    return None
                raise EOFError('incomplete synthetic-peer input')
            data += part
        return bytes(data)

    def send(self, sock, raw):
        if self.fragment:
            for offset in range(0, len(raw), self.fragment):
                sock.sendall(raw[offset:offset + self.fragment])
        else:
            sock.sendall(raw)

    def connection(self, sock):
        assert self.read(sock, 12) == b'DFHack?\n' + struct.pack('<i', 1)
        self.send(sock, b'DFHack!\n' + struct.pack('<i', 1))
        methods = {name: index + 2 for index, name in enumerate(rpc.METHODS)}
        while not self.stopping.is_set():
            header = self.read(sock, 8, eof=True)
            if header is None:
                return
            method, size = struct.unpack('<h2xi', header)
            assert 0 <= size <= 2048
            fields = rpc.decode(self.read(sock, size))
            if method == 0:
                name = fields[1].decode('ascii')
                assert fields == {1: name.encode(), 2: b'dfmcp.build.v1_19.Request',
                                  3: b'dfmcp.build.v1_19.Reply', 4: b'dfmcp_build_v1_19'}
                raw = response_bytes({1: methods[name]})
            else:
                name = next(name for name, value in methods.items() if value == method)
                assert fields[1] == TOKEN.encode() and fields[3] == 1 and fields[4] == 19
                key = fields.get(10, b'').decode('ascii')
                self.calls.append((name, key))
                if self.hook is not None:
                    self.hook(name, fields)
                out = {1: 1, 2: 0, 3: fields[2], 4: 1, 5: 19, 6: self.generation,
                       7: b'synthetic-df', 8: b'synthetic-dfhack'}
                if name == 'ReadPlacement':
                    selected = Selection(*(fields[n] for n in range(5, 10)))
                    out[9] = self.capture(selected).raw
                elif name == 'PreparePlacement':
                    selected = Selection(*(fields[n] for n in range(5, 10)))
                    plan = Plan(key, self.capture(selected))
                    assert fields[11] == plan.before.witness and fields[12] == plan.digest
                    replay = key in self.records
                    self.records.setdefault(key, Record(plan, 'prepared', 'none'))
                    out.update({10: self.records[key].raw, 11: int(replay)})
                elif name == 'CommitPlacement':
                    original = self.records[key]
                    assert original.phase == 'prepared'
                    assert fields[12] == original.plan.digest and fields[13] == original.plan.token
                    self.applied += 1
                    self.sequence += 1
                    self.next_building += 1
                    self.next_job += 1
                    self.building_count += 1
                    value = (Record(original.plan, 'indeterminate', 'native_failure')
                             if self.commit_mode == 'indeterminate' else placed(original.plan))
                    self.records[key] = value
                    if self.commit_mode == 'lost':
                        return  # Effect retained by this peer, acknowledgment lost.
                    out[10] = value.raw
                elif name == 'QueryPlacement':
                    value = self.records.get(key)
                    if value is not None:
                        assert fields[12] == value.plan.digest
                        if self.query_mode == 'lost':
                            return
                        if self.query_mode == 'resolve' and value.phase == 'indeterminate':
                            value = placed(value.plan)
                            self.records[key] = value
                        if self.query_mode == 'refused':
                            value = Record(value.plan, 'refused', 'stale')
                        if self.query_mode == 'wrong_plan':
                            value = placed(Plan(value.plan.key + '-other', value.plan.before))
                        if self.query_mode != 'absent':
                            out[10] = value.raw
                            if self.query_mode == 'malformed':
                                out[10] = value.raw[:-1] + bytes([value.raw[-1] ^ 1])
                elif name == 'CancelPlacement':
                    value = self.records[key]
                    assert fields[12] == value.plan.digest and fields[13] == value.plan.token
                    if value.phase == 'prepared':
                        value = Record(value.plan, 'cancelled', 'cancelled')
                        self.records[key] = value
                    out[10] = value.raw
                else:
                    assert name == 'Handshake'
                out[12] = int(any(r.phase == 'indeterminate' for r in self.records.values()))
                out[13] = len(self.records)
                raw = response_bytes(out)
            self.send(sock, struct.pack('<h2xi', -1, len(raw)) + raw)

    def serve(self):
        while not self.stopping.is_set():
            try:
                sock, _ = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                if not self.stopping.is_set():
                    self.errors.append('unexpected listener error')
                return
            self.connections += 1
            self.active = sock
            try:
                with sock:
                    sock.settimeout(3)
                    self.connection(sock)
            except (EOFError, ConnectionResetError, BrokenPipeError):
                pass  # Deliberate client refusal/interruption closes its source.
            except BaseException as error:
                if not self.stopping.is_set():
                    self.errors.append(error)
            finally:
                self.active = None

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.stopping.set()
        self.listener.close()
        if self.active is not None:
            try:
                self.active.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        self.thread.join(4)
        assert not self.thread.is_alive(), 'synthetic peer did not join'
        assert not self.errors, self.errors
