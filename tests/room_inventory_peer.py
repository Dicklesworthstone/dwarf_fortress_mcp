"""Joined operations/1.4 protocol peer; fixtures, not a DFHack plugin.

The native roster and protobuf encoder are independent of production codecs.
"""
from __future__ import annotations

import hashlib
import socket
import struct
import threading

TOKEN = 'room-inventory-query-test-secret-00000000'
PAGE = 65536


def text(value):
    raw = value.encode('utf-8')
    return struct.pack('>H', len(raw)) + raw


def capture(request, *, padding=0, omit=(), folder=None):
    jobs = (b'DFMJ1200' + struct.pack('>II', 0, 200) + b'\1'
            + struct.pack('>iI', request.site, 10000) + text(folder or request.folder)
            + struct.pack('>I', 0))
    rows = []
    for i, slot in enumerate(request.slots):
        if slot.name in omit:
            continue
        rows.append((i+1, {'bed': 1, 'chair': 2, 'table': 3}[slot.kind], slot.kind.upper(),
                     -1 if slot.subtype is None else slot.subtype,
                     slot.material or (419, -1), slot.target))
    rows += [(100+i, 4, 'BAR', -1, (419, -1), (10, 10, 2)) for i in range(padding)]
    raw = (b'DFMO1400' + struct.pack('>I', len(jobs)) + jobs
           + struct.pack('>III', 1000, max([r[0] for r in rows] or [0])+1, 0)
           + struct.pack('>I', len(rows)))
    for identity, native, kind, subtype, material, position in rows:
        raw += (struct.pack('>Ii', identity, native) + text(kind)
                + struct.pack('>iiiIiiiI', subtype, *material, 1, *position, 64) + b'\0\0')
    return raw + struct.pack('>I', 0)


def number(value):
    result = []
    while value > 127:
        result.append(128 | (value & 127))
        value >>= 7
    return bytes(result+[value])


def encode(fields):
    out = b''
    for key, value in sorted(fields.items()):
        if isinstance(value, bytes):
            out += number(key*8+2) + number(len(value)) + value
        else:
            out += number(key*8) + number(value)
    return out


def decode(raw):
    position = 0
    def read_number():
        nonlocal position
        value, shift = 0, 0
        while True:
            byte = raw[position]; position += 1
            value |= (byte & 127) << shift
            if byte < 128:
                return value
            shift += 7
            assert shift <= 63
    result = {}
    while position < len(raw):
        key = read_number()
        tag, wire = key >> 3, key & 7
        assert tag not in result
        if wire == 2:
            width = read_number()
            value = raw[position:position+width]; position += width
            assert len(value) == width
        else:
            assert wire == 0
            value = read_number()
        result[tag] = value
    return result


class Peer:
    def __init__(self, raw, fault=None):
        self.raw, self.fault = raw, fault
        self.methods, self.pages, self.releases, self.connections = [], [], 0, 0
        self.errors = []
        self.stopping = threading.Event()
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0)); self.listener.listen(1)
        self.listener.settimeout(0.1)
        self.address = self.listener.getsockname()
        self.endpoint = f'{self.address[0]}:{self.address[1]}'
        self.connection = None
        self.thread = threading.Thread(target=self._run)
        self.thread.start()

    def environment(self):
        return {'DFMCP_ALLOW_UNADMITTED_FURNITURE_ALLOCATION': '1',
                'DFMCP_FURNITURE_ALLOCATION_ENDPOINT': self.endpoint,
                'DFMCP_OPERATIONS_PAGED_TOKEN': TOKEN}

    def _run(self):
        try:
            while not self.stopping.is_set():
                try:
                    self.connection, _ = self.listener.accept()
                except TimeoutError:
                    continue
                self.connections += 1
                with self.connection as conn:
                    conn.settimeout(5)
                    conn.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                    self._serve(conn)
                self.connection = None
        except (BrokenPipeError, ConnectionResetError):
            pass  # Expected when a client refuses an injected reply.
        except OSError as error:
            if not self.stopping.is_set(): self.errors.append(error)
        except BaseException as error:
            self.errors.append(error)

    def _serve(self, conn):
        def read(size):
            result = b''
            while len(result) < size:
                data = conn.recv(size-len(result))
                if not data:
                    raise EOFError()
                result += data
            return result
        def send(fields):
            payload = encode(fields)
            raw = struct.pack('<h2xi', -1, len(payload)) + payload
            # Fragment headers and payload without sleep-based scheduling assertions.
            for a, b in ((0,3),(3,8),(8,25),(25,len(raw))):
                if b>a: conn.sendall(raw[a:b])
        try:
            assert read(12) == b'DFHack?\n'+struct.pack('<i',1)
            conn.sendall(b'DFHack!\n'+struct.pack('<i',1))
            nonce, bound, offset = None, {}, 0
            token = b'room-capture-id!'
            while True:
                method, width = struct.unpack('<h2xi', read(8))
                assert 0 <= width <= 2048
                fields = decode(read(width))
                if method == 0:
                    name = fields[1].decode('ascii')
                    assert name in ('Handshake','ReadObservation') and name not in bound
                    assert fields == {1:name.encode('ascii'),2:b'dfmcp.operations.v1_4.Request',
                        3:b'dfmcp.operations.v1_4.Reply',4:b'dfmcp_operations_v1_4'}
                    bound[name] = len(bound)+2
                    self.methods.append(name)
                    send({1:bound[name]}); continue
                assert fields[1] == TOKEN.encode() and (fields[3],fields[4]) == (1,4)
                assert len(fields[2]) == 32
                nonce = nonce or fields[2]
                assert fields[2] == nonce
                assert {key:fields[key] for key in (5,6,7,8,11)} == {
                    5:4096,6:4096,7:65536,8:16*1024*1024,11:PAGE}
                reply = {1:1,2:0,3:nonce,4:1,5:4,6:73,7:b'DF-test',8:b'DFHack-test'}
                if method == bound['Handshake']:
                    assert set(fields)=={1,2,3,4,5,6,7,8,11}
                    send(reply); continue
                assert method == bound['ReadObservation']
                assert set(fields) == set(range(1,13))
                if fields[12]:
                    assert fields[12] == 1 and fields[9] == token and fields[10] == 0
                    assert offset == len(self.raw)
                    self.releases += 1
                    if self.fault == 'lost_release': return
                    reply[10] = b'wrong-capture-id' if self.fault == 'bad_release' else token
                    send(reply); continue
                assert fields[9] == (b'' if offset == 0 else token) and fields[10] == offset
                self.pages.append(offset)
                chunk = self.raw[offset:offset+PAGE]
                end = offset+len(chunk)
                reply.update({9:chunk,10:token,11:offset,12:len(self.raw),
                              13:hashlib.sha256(self.raw).digest(),14:int(end==len(self.raw))})
                if self.fault == 'lost_page': return
                if self.fault == 'bad_digest': reply[13] = b'x'*32
                if self.fault == 'page_offset': reply[11] += 1
                if self.fault == 'source_changed': reply[6] += 1
                if self.fault == 'extra_field': reply[15] = 1
                offset = end
                send(reply)
        except EOFError:
            return

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        self.stopping.set()
        if self.connection is not None:
            try: self.connection.shutdown(socket.SHUT_RDWR)
            except OSError: pass
        self.listener.close()
        self.thread.join(6)
        assert not self.thread.is_alive(), 'fixture left native work unjoined'
        if self.errors: raise self.errors[0]
