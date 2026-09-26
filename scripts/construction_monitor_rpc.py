"""Fixed query-only DFHack connection for receipt-linked construction samples.

One foreground owner, one shrinking allowance, no reconnect or game mutation.
Only the two existing native profiles' read methods are bound. Each fresh capture
is bracketed by the exact original receipt on this SAME TCP connection.
"""
from __future__ import annotations

from dataclasses import dataclass, field
import ipaddress
import os
import secrets
import socket
import struct
import time

from build_placement_wire import Rejected, integer, require, text
from construction_receipt import Goal, LinkedSample, Manifest, MAX_CAPTURE

OPT_IN = 'DFMCP_ALLOW_UNADMITTED_CONSTRUCTION_MONITOR'
ENDPOINT = 'DFMCP_CONSTRUCTION_MONITOR_ENDPOINT'
BUILD_TOKEN = 'DFMCP_BUILD_TOKEN'
OPERATIONS_TOKEN = 'DFMCP_OPERATIONS_PAGED_TOKEN'
ENVIRONMENT = (OPT_IN, ENDPOINT, BUILD_TOKEN, OPERATIONS_TOKEN)
PAGE = 65536
MAX_REPLY = PAGE + 4096
BINDINGS = (
    ('build', 'Handshake'), ('build', 'QueryPlacement'),
    ('operations', 'Handshake'), ('operations', 'ReadObservation'),
)
PROFILES = {'build': ('dfmcp_build_v1_19', 'dfmcp.build.v1_19', 19),
            'operations': ('dfmcp_operations_v1_4', 'dfmcp.operations.v1_4', 4)}


def endpoint(value: str) -> tuple[str, int]:
    require(type(value) is str and 1 <= len(value) <= 128 and value.count(':') == 1,
            'canonical numeric IPv4 loopback endpoint required')
    host, port = value.split(':')
    require(port.isascii() and port.isdecimal() and 1 <= len(port) <= 5, 'invalid endpoint port')
    try:
        address = ipaddress.IPv4Address(host)
    except ipaddress.AddressValueError as error:
        raise Rejected('numeric IPv4 loopback endpoint required') from error
    number = integer(int(port), 1, 65535)
    require(address.is_loopback and value == f'{address}:{number}', 'noncanonical or nonloopback endpoint')
    return str(address), number


@dataclass(frozen=True)
class Authority:
    address: tuple[str, int]
    build_token: bytes = field(repr=False)
    operations_token: bytes = field(repr=False)

    @classmethod
    def load(cls) -> Authority:
        require(all(not key.startswith('DFMCP_') or key in ENVIRONMENT for key in os.environ),
                'unrelated DFMCP mutation or admission environment')
        require(os.environ.get(OPT_IN) == '1', 'explicit construction-monitor development opt-in required')
        tokens = tuple(os.environ.get(key, '').encode('utf-8') for key in (BUILD_TOKEN, OPERATIONS_TOKEN))
        require(all(32 <= len(token) <= 256 and b'\0' not in token for token in tokens),
                'invalid query credential lengths')
        return cls(endpoint(os.environ.get(ENDPOINT, '127.0.0.1:5000')), *tokens)

    def guard(self) -> None:
        require(Authority.load() == self, 'operator query configuration changed')


class Budget:
    """Shared wall deadline and explicit work, network and custody allowances.

    Filesystem synchronization is cooperatively checked, not forcibly interrupted.
    None of these allowances can be renewed by individual pages or replay frames.
    """
    def __init__(self, timeout_ms: int):
        self.deadline = time.monotonic() + integer(timeout_ms, 1, 60000) / 1000
        self.calls = 272
        self.network_bytes = 20 * 1024 * 1024
        self.disk_bytes = 1024 * 1024 * 1024
        self.work_steps = 20_000_000

    def remaining(self) -> float:
        value = self.deadline - time.monotonic()
        require(value > 0, 'whole-operation wall deadline exhausted')
        return value

    def work(self) -> None:
        self.remaining()
        require(self.work_steps > 0, 'whole-operation work allowance exhausted')
        self.work_steps -= 1

    def reserve(self, name: str, count: int) -> None:
        require(name in ('calls', 'network_bytes', 'disk_bytes'), 'unknown budget class')
        integer(count, 0, 1024 * 1024 * 1024)
        self.remaining()
        left = getattr(self, name)
        require(count <= left, 'whole-operation allowance exhausted')
        setattr(self, name, left - count)


def varint(value: int) -> bytes:
    integer(value, 0, 2**64 - 1)
    out = bytearray()
    while value >= 128:
        out.append((value & 127) | 128)
        value >>= 7
    out.append(value)
    return bytes(out)


def encode(fields: dict[int, int | bytes]) -> bytes:
    out = bytearray()
    for number, value in sorted(fields.items()):
        integer(number, 1, 14)
        if type(value) is bytes:
            out += varint(number * 8 + 2) + varint(len(value)) + value
        else:
            out += varint(number * 8) + varint(value)
    require(len(out) <= 2048, 'oversized fixed-profile request')
    return bytes(out)


def decode(raw: bytes, maximum: int = 14) -> dict[int, int | bytes]:
    require(type(raw) is bytes and len(raw) <= MAX_REPLY, 'oversized native reply')
    offset, fields = 0, {}

    def number() -> int:
        nonlocal offset
        value = 0
        for index in range(10):
            require(offset < len(raw), 'truncated protobuf varint')
            byte = raw[offset]
            offset += 1
            require(index < 9 or byte <= 1, 'protobuf varint overflow')
            value |= (byte & 127) << (7 * index)
            if byte < 128:
                require(index == 0 or byte != 0, 'nonminimal protobuf varint')
                return value
        raise Rejected('unterminated protobuf varint')

    while offset < len(raw):
        tag = number()
        key, kind = tag >> 3, tag & 7
        require(1 <= key <= maximum and key not in fields, 'unknown or duplicate protobuf field')
        if kind == 0:
            value = number()
        elif kind == 2:
            width = number()
            require(width <= len(raw) - offset, 'truncated protobuf bytes')
            value = raw[offset:offset + width]
            offset += width
        else:
            raise Rejected('unsupported protobuf wire type')
        fields[key] = value
    return fields


class Client:
    def __init__(self, authority: Authority, goal: Goal, budget: Budget):
        goal.encode()
        authority.guard()
        budget.remaining()
        self.authority, self.goal, self.budget = authority, goal, budget
        self.nonce, self.methods = secrets.token_bytes(32), {}
        self.closed, self.used, self.notifications_left = False, False, 2 * 1024 * 1024
        self.manifests = {}
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            self.socket.settimeout(budget.remaining())
            self.socket.connect(authority.address)
            self._send(b'DFHack?\n' + struct.pack('<i', 1))
            require(self._read(12) == b'DFHack!\n' + struct.pack('<i', 1), 'wrong native greeting')
            for family, name in BINDINGS:
                plugin, package, _ = PROFILES[family]
                reply = decode(self._frame(0, encode({1: name.encode('ascii'),
                    2: (package + '.Request').encode('ascii'), 3: (package + '.Reply').encode('ascii'),
                    4: plugin.encode('ascii')})), 1)
                require(set(reply) == {1}, 'invalid native read-method binding')
                identity = integer(reply[1], 2, 32767)
                require(identity not in self.methods.values(), 'aliased native read methods')
                self.methods[family, name] = identity
            self._call('build', 'Handshake', {}, set(range(1, 9)) | {12, 13})
            require(self.manifests['build'].generation == goal.record.plan.before.generation,
                    'original placement incarnation no longer available')
            self._call('operations', 'Handshake', {}, set(range(1, 9)))
            require(self.manifests['build'].software == self.manifests['operations'].software,
                    'native software disagreement on one connection')
        except BaseException:
            self.close()
            raise

    def _send(self, raw: bytes) -> None:
        self.budget.reserve('network_bytes', len(raw))
        self.socket.settimeout(self.budget.remaining())
        self.socket.sendall(raw)

    def _read(self, count: int) -> bytes:
        self.budget.reserve('network_bytes', count)
        out = bytearray()
        while len(out) < count:
            self.socket.settimeout(self.budget.remaining())
            part = self.socket.recv(count - len(out))
            require(bool(part), 'incomplete native read')
            out += part
        self.budget.remaining()
        return bytes(out)

    def _frame(self, method: int, payload: bytes) -> bytes:
        require(not self.closed and len(payload) <= 2048, 'closed or oversized native request')
        self.authority.guard()
        self.budget.reserve('calls', 1)
        self._send(struct.pack('<h2xi', method, len(payload)) + payload)
        notifications, notified = 0, 0
        while True:
            kind, size = struct.unpack('<h2xi', self._read(8))
            require(kind in (-1, -3), 'native transport refused query')
            require(0 <= size <= (MAX_REPLY if kind == -1 else 65536), 'oversized native frame')
            if kind == -3:
                notifications += 1
                notified += size
                require(notifications <= 8 and notified <= 262144 and size <= self.notifications_left,
                        'native notification allowance exhausted')
                self.notifications_left -= size
            raw = self._read(size)
            if kind == -1:
                return raw

    def _call(self, family: str, name: str, extra: dict, expected_fields: set) -> dict:
        try:
            token = self.authority.build_token if family == 'build' else self.authority.operations_token
            fields = {1: token, 2: self.nonce, 3: 1, 4: PROFILES[family][2]}
            if family == 'operations':
                fields.update({5: 4096, 6: 4096, 7: 65536, 8: MAX_CAPTURE, 11: PAGE})
            require(not (set(extra) & set(fields)), 'cannot override fixed query fields')
            fields.update(extra)
            reply = decode(self._frame(self.methods[family, name], encode(fields)))
            require(set(range(1, 9)) <= set(reply), 'missing native reply fields')
            for n in (1, 2, 4, 5, 6):
                require(type(reply[n]) is int, 'wrong numeric reply type')
            for n in (3, 7, 8):
                require(type(reply[n]) is bytes, 'wrong native byte-field type')
            require(reply[3] == self.nonce and reply[4] == 1 and reply[5] == PROFILES[family][2],
                    'native nonce or protocol changed')
            accepted, code = integer(reply[1], 0, 1), integer(reply[2], 0, 8)
            require(accepted == 1 and code == 0, 'native query refused; sample remains unestablished')
            require(set(reply) == expected_fields, 'noncanonical native success shape')
            manifest = Manifest(reply[6], text(reply[7], 128), text(reply[8], 128))
            manifest.encode()
            require(family not in self.manifests or self.manifests[family] == manifest,
                    'native incarnation/software changed during capture')
            self.manifests[family] = manifest
            if family == 'build':
                unresolved = integer(reply[12], 0, 1)
                retained = integer(reply[13], 0, 256)
                require(not unresolved or retained > 0, 'native uncertainty without retained work')
            self.authority.guard()
            self.budget.remaining()
            return reply
        except BaseException:
            self.close()
            raise

    def _receipt(self) -> tuple[Manifest, bytes]:
        reply = self._call('build', 'QueryPlacement',
                           {10: self.goal.record.plan.key.encode('ascii'), 12: self.goal.record.plan.digest},
                           set(range(1, 9)) | {10, 12, 13})
        require(reply[13] > 0 and reply[10] == self.goal.receipt,
                'original placed record not retained byte-for-byte')
        return self.manifests['build'], reply[10]

    def capture_once(self) -> LinkedSample:
        require(not self.used and not self.closed, 'acquisition cannot be retried on this connection')
        self.used = True
        try:
            before, before_record = self._receipt()
            token, offset, identity, pieces = b'', 0, None, []
            for _ in range(MAX_CAPTURE // PAGE):
                reply = self._call('operations', 'ReadObservation', {9: token, 10: offset, 12: 0}, set(range(1, 15)))
                for n in (9, 10, 13):
                    require(type(reply[n]) is bytes, 'wrong native page byte-field type')
                require(len(reply[10]) == 16 and len(reply[13]) == 32, 'invalid native page identity width')
                total = integer(reply[12], 1, MAX_CAPTURE)
                integer(reply[11], 0, total)
                complete = bool(integer(reply[14], 0, 1))
                current = reply[10], total, reply[13]
                require(identity is None or identity == current, 'mixed retained captures')
                identity = current
                require(reply[11] == offset and offset < total, 'native page skipped or replayed')
                width = min(PAGE, total - offset)
                require(len(reply[9]) == width and complete == (offset + width == total),
                        'partial or contradictory native page')
                pieces.append(reply[9])
                offset += width
                token = reply[10]
                if complete:
                    break
            require(identity is not None and offset == identity[1], 'native capture incomplete')
            raw = b''.join(pieces)
            import hashlib
            require(hashlib.sha256(raw).digest() == identity[2], 'whole native capture digest mismatch')
            release = self._call('operations', 'ReadObservation', {9: token, 10: 0, 12: 1}, set(range(1, 9)) | {10})
            require(release[10] == token, 'native release acknowledgment changed token')
            after, after_record = self._receipt()
            sample = LinkedSample(before, before_record, self.manifests['operations'], raw, after, after_record)
            sample.validate(self.goal, self.budget.work)
            self.authority.guard()
            self.budget.remaining()
            return sample
        except BaseException:
            self.close()
            raise

    def close(self) -> None:
        self.closed = True
        self.socket.close()

    def __enter__(self) -> Client:
        return self

    def __exit__(self, *_args):
        self.close()


def acquire(authority: Authority, goal: Goal, budget: Budget) -> LinkedSample:
    with Client(authority, goal, budget) as client:
        return client.capture_once()
