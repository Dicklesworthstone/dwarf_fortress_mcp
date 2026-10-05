"""Joined protocol double for foreground-wait process tests, not a live game.

Operations and protobuf bytes are encoded independently from the client codecs.
Placed-record values are explicit semantic fixtures using the existing model.
"""
from __future__ import annotations

import hashlib
import os
from pathlib import Path
import socket
import struct
import sys
import threading

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
from build_placement_wire import Capture, Insertion, Item, Plan, Record, Selection, Tile

BUILD_TOKEN = 'b' * 40
OPS_TOKEN = 'o' * 40
SOFTWARE = (b'test-df', b'test-dfhack')
BUILD_GENERATION = 7
OPS_GENERATION = 11


def records(count=1):
    out = []
    for index in range(count):
        kind = 1 + index % 3
        selected = Selection(kind, 42 + index, 10 + 2 * index, 10, 2)
        floor = Tile(presence=2, tiletype=1, shape=3)
        item = Item(presence=2, pos=(5, 5, 2), kind=kind, native_type=kind,
                    subtype=-1, material=419, material_index=-1, on_ground=True, ground=floor)
        before = Capture(BUILD_GENERATION, index * 2, 100, 2, (128, 128, 10),
                         100 + index, 200 + index, index, 'region1', True, True, True,
                         selected, (floor,) * 9, item)
        plan = Plan('wait-target-' + str(index), before)
        insertion = Insertion(before.next_building, before.next_job, selected.item, kind,
                              selected.target, 419, -1, 0, 1, True, True, True, False)
        out.append(Record(plan, 'placed', 'none', before.expected_after(), insertion))
    return tuple(out)


def u32(value):
    return struct.pack('>I', value)


def i32(value):
    return struct.pack('>i', value)


def text(value):
    data = value.encode('utf-8')
    return struct.pack('>H', len(data)) + data


def reference(value):
    return b'\0' if value is None else b'\1' + u32(value)


def capture(selected, tick, *, stages=None, filler=0, missing_item=None, removal=False):
    stages = [1] * len(selected) if stages is None else stages
    jobs = b'DFMJ1200' + u32(tick // 403200) + u32(tick % 403200) + b'\0'
    jobs += i32(2) + u32(10000) + text('region1') + u32(int(removal))
    if removal:
        p = selected[0].insertion
        jobs += (u32(9000) + i32(50) + text('DestroyBuilding') + text('') + b'\0\0'
                 + b''.join(i32(v) for v in p.pos) + reference(None) + reference(p.building)
                 + i32(-1) + u32(0) + u32(0))
    out = bytearray(b'DFMO1400' + u32(len(jobs)) + jobs + u32(10000) + u32(100000))
    out += u32(len(selected))
    for record, stage in zip(selected, stages):
        p = record.insertion
        x, y, z = p.pos
        out += u32(p.building) + i32(p.kind) + text(('', 'Bed', 'Chair', 'Table')[p.kind])
        out += b''.join(i32(v) for v in (x, y, x, y, z, stage, 1))
    items = [(r.insertion.item, r) for r in selected if r.insertion.item != missing_item]
    out += u32(len(items) + filler)
    for identity, record in items:
        p, item = record.insertion, record.plan.before.item
        out += u32(identity) + i32(item.native_type) + text(('', 'BED', 'CHAIR', 'TABLE')[p.kind])
        out += i32(item.subtype) + i32(item.material) + i32(item.material_index) + u32(1)
        out += b''.join(i32(v) for v in p.pos) + u32(256) + reference(None) + reference(p.building)
    for index in range(filler):
        out += u32(2000 + index) + i32(4) + text('WOOD')
        out += i32(-1) + i32(419) + i32(-1) + u32(1)
        out += i32(1) * 3 + u32(64) + reference(None) + reference(None)
    out += u32(0)
    return bytes(out)


def varint(value):
    data = bytearray()
    while value > 127:
        data.append((value & 127) | 128)
        value >>= 7
    data.append(value)
    return bytes(data)


def protobuf(values):
    out = bytearray()
    for field, value in sorted(values.items()):
        if isinstance(value, bytes):
            out += varint(field * 8 + 2) + varint(len(value)) + value
        else:
            out += varint(field * 8) + varint(value)
    return bytes(out)


def parse(raw):
    offset = 0
    def number():
        nonlocal offset
        result, shift = 0, 0
        while True:
            value = raw[offset]
            offset += 1
            result |= (value & 127) << shift
            if value < 128:
                return result
            shift += 7
            assert shift < 70
    fields = {}
    while offset < len(raw):
        tag = number()
        key, kind = tag // 8, tag % 8
        assert key not in fields
        if kind == 0:
            value = number()
        else:
            assert kind == 2
            width = number()
            value = raw[offset:offset + width]
            offset += width
        fields[key] = value
    assert offset == len(raw)
    return fields


def receive(sock, size):
    raw = bytearray()
    while len(raw) < size:
        part = sock.recv(size - len(raw))
        if not part:
            return None
        raw += part
    return bytes(raw)


class Peer:
    def __init__(self, selected, captures, *, faults=None, generations=None, hook=None):
        self.selected, self.captures = selected, captures
        self.faults = faults or {}
        self.generations = generations or {}
        self.hook = hook
        self.connections, self.calls, self.errors = 0, [], []
        self.stopping = threading.Event()
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(4)
        self.listener.settimeout(.1)
        self.address = self.listener.getsockname()
        self.worker = threading.Thread(target=self.serve)
        self.worker.start()

    def environment(self):
        env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
        env.update(DFMCP_ALLOW_UNADMITTED_CONSTRUCTION_MONITOR='1',
                   DFMCP_CONSTRUCTION_MONITOR_ENDPOINT=f'{self.address[0]}:{self.address[1]}',
                   DFMCP_BUILD_TOKEN=BUILD_TOKEN, DFMCP_OPERATIONS_PAGED_TOKEN=OPS_TOKEN,
                   PYTHONDONTWRITEBYTECODE='1')
        return env

    def serve(self):
        try:
            while not self.stopping.is_set():
                try:
                    connection, _ = self.listener.accept()
                except socket.timeout:
                    continue
                index = self.connections
                self.connections += 1
                with connection:
                    connection.settimeout(5)
                    connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                    self.handle(connection, index)
        except BaseException as error:
            self.errors.append(error)

    def handle(self, sock, index):
        assert index < len(self.captures), 'unexpected automatic acquisition/retry'
        raw = self.captures[index]
        fault = self.faults.get(index)
        token = index.to_bytes(16, 'big')
        assert receive(sock, 12) == b'DFHack?\n' + struct.pack('<i', 1)
        sock.sendall(b'DFHack!\n' + struct.pack('<i', 1))
        bindings = [(b'dfmcp_build_v1_19', b'Handshake', b'dfmcp.build.v1_19'),
                    (b'dfmcp_build_v1_19', b'QueryPlacement', b'dfmcp.build.v1_19'),
                    (b'dfmcp_operations_v1_4', b'Handshake', b'dfmcp.operations.v1_4'),
                    (b'dfmcp_operations_v1_4', b'ReadObservation', b'dfmcp.operations.v1_4')]
        bound, receipt_count, offset, released = 0, 0, 0, False
        nonce = None
        while True:
            header = receive(sock, 8)
            if header is None:
                return
            method, length = struct.unpack('<h2xi', header)
            assert 0 <= length <= 2048
            payload = receive(sock, length)
            assert payload is not None
            req = parse(payload)
            self.calls.append((index, method))
            if self.hook:
                self.hook(index, method, req)
            if method == 0:
                assert bound < 4
                plugin, name, package = bindings[bound]
                assert req == {1: name, 2: package + b'.Request', 3: package + b'.Reply', 4: plugin}
                reply = {1: bound + 2}
                bound += 1
            else:
                assert bound == 4 and method in (2, 3, 4, 5)
                family = 'build' if method in (2, 3) else 'operations'
                assert req[1] == (BUILD_TOKEN if family == 'build' else OPS_TOKEN).encode()
                assert req[3] == 1 and req[4] == (19 if family == 'build' else 4)
                assert len(req[2]) == 32 and (nonce is None or nonce == req[2])
                nonce = req[2]
                generation = BUILD_GENERATION if family == 'build' else self.generations.get(index, OPS_GENERATION)
                reply = {1: 1, 2: 0, 3: nonce, 4: 1, 5: req[4], 6: generation,
                         7: SOFTWARE[0], 8: SOFTWARE[1]}
                if family == 'build':
                    reply.update({12: 0, 13: len(self.selected)})
                if method == 3:
                    selected = self.selected[receipt_count % len(self.selected)]
                    assert req[10] == selected.plan.key.encode() and req[12] == selected.plan.digest
                    assert (receipt_count < len(self.selected) and offset == 0) or released
                    receipt_count += 1
                    if fault == 'lost_final_receipt' and receipt_count == 2 * len(self.selected):
                        return
                    reply[10] = selected.raw
                elif method == 5:
                    assert req[5] == 4096 and req[6] == 4096 and req[7] == 65536
                    assert req[8] == 16 * 1024 * 1024 and req[11] == 65536
                    assert receipt_count == len(self.selected)
                    if req[12]:
                        assert req[9] == token and req[10] == 0 and offset == len(raw)
                        if fault == 'lost_release':
                            return
                        reply[10] = b'x' * 16 if fault == 'bad_release' else token
                        released = True
                    else:
                        assert req[10] == offset and req[9] == (b'' if offset == 0 else token)
                        width = min(65536, len(raw) - offset)
                        reply.update({9: raw[offset:offset + width], 10: token, 11: offset,
                                      12: len(raw), 13: hashlib.sha256(raw).digest(),
                                      14: int(offset + width == len(raw))})
                        if fault == 'bad_digest':
                            reply[13] = b'x' * 32
                        offset += width
            encoded = protobuf(reply)
            packet = struct.pack('<h2xi', -1, len(encoded)) + encoded
            # Exercise fragmented framing without sleeps or timing assertions.
            sock.sendall(packet[:5])
            sock.sendall(packet[5:])

    def close(self):
        self.stopping.set()
        self.worker.join(6)
        self.listener.close()
        if self.worker.is_alive():
            raise AssertionError('test peer did not quiesce')
        if self.errors:
            raise self.errors[0]

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
