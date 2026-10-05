"""Independent map/1.5 + dig/1.16 TCP fixture, not DFHack or a game.

The transport is the existing joined test peer. Native dig observations, plans,
tokens and receipts below are assembled independently of production decoders.
"""
import hashlib
import struct

from room_terrain_peer import Peer, TOKEN as MAP_TOKEN, _decode, _encode
from room_terrain_fixtures import MANIFEST

DIG_TOKEN = b'room-dig-fixture-token-012345678901'
DIG_METHODS = ('Handshake', 'ReadDesignation', 'PrepareDesignation', 'CommitDesignation',
               'QueryDesignation', 'CancelDesignation')


def digest(domain, data):
    return hashlib.sha256(domain + b'\0' + data).digest()


def text(value):
    raw = value.encode()
    return struct.pack('>H', len(raw)) + raw


class DigPeer(Peer):
    def __init__(self, raw, region, *, already_dug=()):
        super().__init__(raw, region)
        self.generation, self.sequence, self.tick = 900, 0, 100
        self.folder, self.site, self.dimensions, self.paused = 'region1', 2, (32768,) * 3, True
        self.df_version, self.dfhack_version = 'test-df', 'test-dfhack'
        self.already_dug, self.designated = set(already_dug), set()
        self.effects, self.reads, self.commits, self.operations = {}, [], [], []
        self.map_reads = 0
        self.fault = None
        self.callback = None

    def observation(self, region):
        x, y, z, width, height = region
        coordinates = [(px, py, pz) for pz in range(z - 1, z + 2)
                       for py in range(y - 1, y + height + 1) for px in range(x - 1, x + width + 1)]
        header = (b'DFMDG016' + struct.pack('>QQQI', self.generation, self.sequence, self.tick, self.site)
                  + struct.pack('>3I', *self.dimensions) + struct.pack('>5I', *region)
                  + bytes([self.paused]) + text(self.folder) + struct.pack('>H', len(coordinates)))
        cells = []
        for point in coordinates:
            designated = point in self.designated
            # Floor flags cannot accidentally satisfy native natural-wall eligibility.
            flags = 0 if point in self.already_dug else 1
            cells.append(b'\x02' + struct.pack('>IIIIIIHHBBB', 42, 0, 0, 4000 if designated else 0,
                         0, 0, 10015, 10015, int(designated), 0, flags))
        return header + b''.join(cells)

    @staticmethod
    def after(record):
        raw = bytearray(record['raw'])
        generation, sequence, tick = struct.unpack('>QQQ', raw[8:32])
        struct.pack_into('>Q', raw, 16, sequence + 1)
        x, y, z, width, height = record['region']
        folder_length = struct.unpack('>H', raw[69:71])[0]
        offset = 73 + folder_length
        for pz in range(z - 1, z + 2):
            for py in range(y - 1, y + height + 1):
                for px in range(x - 1, x + width + 1):
                    assert raw[offset] == 2
                    if pz == z and x <= px < x + width and y <= py < y + height:
                        struct.pack_into('>I', raw, offset + 13, 4000)
                        raw[offset + 29] = 1
                    if pz == z and x // 16 <= px // 16 <= (x + width - 1) // 16 and y // 16 <= py // 16 <= (y + height - 1) // 16:
                        struct.pack_into('>I', raw, offset + 17, 0)
                        raw[offset + 31] |= 16
                    offset += 32
        assert offset == len(raw)
        return bytes(raw)

    @classmethod
    def receipt(cls, record):
        generation, sequence, tick = struct.unpack('>QQQ', record['raw'][8:32])
        state, reason = record['state'], record.get('reason', 0)
        count = record['region'][3] * record['region'][4] if state == 2 else 0
        after = hashlib.sha256(cls.after(record)).digest() if state == 2 else bytes(32)
        if record.get('bad_after'):
            after = b'X' * 32  # A freshly rehashed receipt must still fail exact readback.
        outcome = bytes([state, reason, int(state == 2)]) + struct.pack('>I', count) + after
        proof = digest(b'dfmcp-dig-designation-receipt/1', struct.pack('>Q', generation)
                       + text(record['key']) + record['plan'] + record['token'] + outcome) if state in (2, 4) else bytes(32)
        return (b'DFMDGE16' + record['raw'][8:32] + struct.pack('>5I', *record['region'])
                + bytes([record['hidden']]) + record['witness'] + record['plan'] + record['token']
                + outcome + proof + text(record['key']))

    def session(self, connection):
        assert self.read(connection, 12) == b'DFHack?\n' + struct.pack('<i', 1)
        self.send(connection, b'DFHack!\n' + struct.pack('<i', 1))
        methods, protocol, nonce, observed = [], None, None, None
        while not self.stop.is_set():
            method, length = struct.unpack('<h2xi', self.read(connection, 8))
            assert 1 <= length <= 2048
            fields = _decode(self.read(connection, length))
            if method == 0:
                if protocol is None:
                    protocol = {b'dfmcp_map_v1_5': 'map', b'dfmcp_dig_v1_16': 'dig'}[fields[4]]
                names = ('Handshake', 'ReadObservation') if protocol == 'map' else DIG_METHODS
                index = len(methods)
                assert index < len(names) and fields[1] == names[index].encode()
                prefix = b'dfmcp.map.v1_5' if protocol == 'map' else b'dfmcp.dig.v1_16'
                assert fields[2] == prefix + b'.Request' and fields[3] == prefix + b'.Reply'
                methods.append(names[index])
                self.binds.append((protocol, names[index]))
                self.reply(connection, {1: index + 2})
                continue
            assert method - 2 < len(methods)
            operation = methods[method - 2]
            assert nonce is None or nonce == fields[2]
            nonce = fields[2]
            assert len(nonce) == 32 and fields[3] == 1
            self.operations.append((protocol, operation))
            if protocol == 'map':
                assert fields[1] == MAP_TOKEN and fields[4] == 5
                assert tuple(fields[i] for i in range(5, 11)) == self.region.origin + self.region.size
                response = {1: 1, 2: 0, 3: nonce, 4: 1, 5: 5, 6: MANIFEST.generation,
                            7: MANIFEST.df_version.encode(), 8: MANIFEST.dfhack_version.encode()}
                if operation == 'ReadObservation':
                    self.map_reads += 1
                    response[9] = self.raw
                self.reply(connection, response)
                continue
            assert fields[1] == DIG_TOKEN and fields[4] == 16
            response = {1: 1, 2: 0, 3: nonce, 4: 1, 5: 16, 6: self.generation,
                        7: self.df_version.encode(), 8: self.dfhack_version.encode()}
            if operation == 'ReadDesignation':
                region = tuple(fields[i] for i in range(5, 10))
                raw = self.observation(region)
                observed = region, raw
                self.reads.append(region)
                response[9] = raw
            elif operation == 'PrepareDesignation':
                region = tuple(fields[i] for i in range(5, 10))
                assert observed is not None and region == observed[0]
                assert fields[10] in (0, 1)
                key = fields[11].decode()
                witness = hashlib.sha256(observed[1]).digest()
                plan = digest(b'dfmcp-dig-designation-plan/1', struct.pack('>5I', *region) + bytes([fields[10]]) + witness)
                assert fields[12] == witness and fields[13] == plan
                fresh = key not in self.effects
                if fresh:
                    self.effects[key] = {'key': key, 'raw': observed[1], 'region': region, 'witness': witness,
                        'plan': plan, 'token': digest(b'dfmcp-dig-designation-token/1',
                        struct.pack('>Q', self.generation) + text(key) + plan)[:16], 'hidden': fields[10], 'state': 0}
                record = self.effects[key]
                response[10], response[11] = self.receipt(record), int(not fresh)
            elif operation in ('CommitDesignation', 'QueryDesignation', 'CancelDesignation'):
                key = fields[11].decode()
                record = self.effects[key]
                assert fields[13] == record['plan']
                if operation != 'QueryDesignation':
                    assert fields[14] == record['token']
                if operation == 'CommitDesignation':
                    assert key not in self.commits, 'duplicate native dispatch'
                    self.commits.append(key)
                    if self.fault == 'refused_commit':
                        record['state'], record['reason'] = 4, 1
                    else:
                        record['state'] = 2
                        if self.fault == 'bad_readback':
                            record['bad_after'] = True
                        x, y, z, w, hh = record['region']
                        points = {(px, py, z) for py in range(y, y + hh) for px in range(x, x + w)}
                        assert not points & (self.designated | self.already_dug), 'substituted or duplicate targets'
                        self.designated.update(points)
                        self.sequence += 1
                elif operation == 'CancelDesignation' and record['state'] == 0:
                    record['state'], record['reason'] = 4, 2
                if not (operation == 'QueryDesignation' and self.fault == 'unknown_query'):
                    response[10] = self.receipt(record)
            else:
                assert operation == 'Handshake' and set(fields) == {1, 2, 3, 4}
            if self.callback:
                self.callback(operation)
            if (operation, self.fault) in (('CommitDesignation', 'lost_commit'), ('PrepareDesignation', 'lost_prepare')):
                return
            self.reply(connection, response)
