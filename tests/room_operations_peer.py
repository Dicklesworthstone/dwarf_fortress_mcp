"""Joined synthetic operations/1.4 + original furniture receipt query peer.

Operations bytes are independently assembled from fixture values. Placement
receipts come from the production-codec placement peer; no live game is present.
"""
import hashlib
import struct

import build_placement_rpc as placement_rpc
from room_furniture_peer import Peer as PlacementPeer, proto

PAGE = 65536
SNAPSHOT = b's' * 16


def text(value):
    raw = value.encode('utf-8')
    return struct.pack('>H', len(raw)) + raw


def uints(*values):
    return struct.pack('>' + 'I' * len(values), *values)


class Peer(PlacementPeer):
    def __init__(self, handoff):
        self.mode = 'placement'
        self.operations_generation = 7
        self.operations_tick = 1000
        self.operations_folder = 'region1'
        self.operations_site = 2
        self.operations_padding = 0
        self.unverified_items = set()
        self.missing_buildings = set()
        self.inventory_omitted = set()
        self.read_sessions = []
        self.operations_fault = None
        self.on_read_event = None
        super().__init__(handoff)

    def operations(self):
        jobs = (b'DFMJ1200' + uints(self.operations_tick // 403200, self.operations_tick % 403200)
                + b'\x01' + uints(self.operations_site, self.next_job) + text(self.operations_folder) + uints(0))
        installed = {r.insertion.item: r for r in self.records.values() if r.phase == 'placed'}
        buildings = sorted((r for r in installed.values() if r.insertion.building not in self.missing_buildings),
                           key=lambda r: r.insertion.building)
        maximum_item = max([1000] + list(self.items) + [10000 + self.operations_padding]) + 1
        raw = bytearray(b'DFMO1400' + uints(len(jobs)) + jobs
                        + uints(self.next_building, maximum_item, len(buildings)))
        for r in buildings:
            p = r.insertion
            x, y, z = p.pos
            raw += uints(p.building, p.kind) + text(('', 'Bed', 'Chair', 'Table')[p.kind])
            raw += struct.pack('>7i', x, y, x, y, z, p.max_stage, p.max_stage)
        items = [(identity, row) for identity, row in sorted(self.items.items())
                 if identity not in self.inventory_omitted]
        raw += uints(len(items) + self.operations_padding)
        for identity, row in items:
            c = row.candidate
            record = installed.get(identity)
            holder = (record.insertion.building if record is not None
                      and record.insertion.building not in self.missing_buildings
                      and identity not in self.unverified_items else None)
            raw += uints(identity, row.native_type) + text(c.kind.upper())
            raw += struct.pack('>iiiIiiiI', c.subtype, *c.material, 1, *c.position,
                               256 if holder is not None else 64)
            raw += b'\0' + (b'\0' if holder is None else b'\1' + uints(holder))
        for identity in range(10000, 10000 + self.operations_padding):
            raw += uints(identity, 777) + text('BLOCKS')
            raw += struct.pack('>iiiIiiiI', -1, 0, -1, 1, 5, 5, 2, 64) + b'\0\0'
        return bytes(raw + uints(0))

    def connection(self, sock):
        if self.mode == 'placement':
            return super().connection(sock)
        assert self.mode in ('monitor', 'allocation')
        assert self.read(sock, 12) == b'DFHack?\n' + struct.pack('<i', 1)
        self.send(sock, b'DFHack!\n' + struct.pack('<i', 1))
        methods, events, capture, nonce = {}, [], None, None
        self.read_sessions.append(events)
        while not self.stop.is_set():
            method, size = struct.unpack('<h2xi', self.read(sock, 8))
            assert 0 <= size <= 2048
            f = placement_rpc.decode(self.read(sock, size))
            if method == 0:
                family = {b'dfmcp_build_v1_19': 'build', b'dfmcp_operations_v1_4': 'operations'}[f[4]]
                name = f[1].decode()
                assert family == 'operations' or self.mode == 'monitor'
                assert name in (('Handshake', 'QueryPlacement') if family == 'build' else ('Handshake', 'ReadObservation'))
                package = 'dfmcp.build.v1_19' if family == 'build' else 'dfmcp.operations.v1_4'
                assert f[2] == (package + '.Request').encode() and f[3] == (package + '.Reply').encode()
                assert (family, name) not in methods.values()
                identity = len(methods) + 2
                methods[identity] = family, name
                response = {1: identity}
            else:
                family, name = methods[method]
                assert f[1] == (b't' * 32 if family == 'build' else b'o' * 32)
                assert len(f[2]) == 32 and (nonce is None or nonce == f[2])
                nonce = f[2]
                assert f[3] == 1 and f[4] == (19 if family == 'build' else 4)
                response = {1: 1, 2: 0, 3: nonce, 4: 1, 5: f[4],
                            6: self.generation if family == 'build' else self.operations_generation,
                            7: self.df.encode(), 8: self.dfhack.encode()}
                if family == 'build':
                    response.update({12: 0, 13: len(self.records)})
                    if name == 'QueryPlacement':
                        key = f[10].decode()
                        record = self.records[key]
                        assert f[12] == record.plan.digest
                        response[10] = record.raw
                        if self.operations_fault == 'receipt_after' and ('operations', 'release') in events:
                            response[10] = record.raw[:-1] + bytes([record.raw[-1] ^ 1])
                        events.append((family, name, key))
                    else:
                        events.append((family, name))
                elif name == 'Handshake':
                    events.append((family, name))
                else:
                    assert tuple(f[i] for i in (5, 6, 7, 8, 11)) == (4096, 4096, 65536, 16777216, PAGE)
                    if f[12]:
                        assert f[9] == SNAPSHOT and f[10] == 0 and capture is not None
                        events.append((family, 'release'))
                        response[10] = b'x' * 16 if self.operations_fault == 'release' else SNAPSHOT
                    else:
                        offset = f[10]
                        if capture is None:
                            assert offset == 0 and f[9] == b''
                            capture = self.operations()
                        else:
                            assert f[9] == SNAPSHOT
                        events.append((family, 'page', offset))
                        piece = capture[offset:offset + PAGE]
                        response.update({9: piece, 10: SNAPSHOT, 11: offset, 12: len(capture),
                                         13: hashlib.sha256(capture).digest(),
                                         14: int(offset + len(piece) == len(capture))})
                        if self.operations_fault == 'digest':
                            response[13] = b'x' * 32
                        if self.operations_fault == 'source':
                            response[6] += 1
                        if self.operations_fault == 'lost':
                            return
                if self.on_read_event:
                    self.on_read_event(events[-1])
            payload = proto(response)
            self.send(sock, struct.pack('<h2xi', -1, len(payload)) + payload)
