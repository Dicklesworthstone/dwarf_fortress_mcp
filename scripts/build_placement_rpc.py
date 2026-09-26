"""Fixed, bounded furniture/1.19 transport with connection-owned one-shot dispatch.

This unadmitted developer transport is not a journal, lease or checkpoint. The
caller must persist intent and dispatch state before entering the effect method.
"""
from __future__ import annotations

from dataclasses import dataclass, field
import ipaddress
import os
import secrets
import socket
import struct
import time

from build_placement_wire import Capture, Plan, Record, Selection, Rejected, integer, require, successor

METHODS = ('Handshake', 'ReadPlacement', 'PreparePlacement', 'CommitPlacement',
           'QueryPlacement', 'CancelPlacement')
OPT_IN = 'DFMCP_ALLOW_UNADMITTED_BUILD_V1_19'
TOKEN = 'DFMCP_BUILD_TOKEN'
ENDPOINT = 'DFMCP_BUILD_ENDPOINT'
PLACE = 'DFMCP_BUILD_ALLOW_PLACE'
MAX_REPLY = 8192


def endpoint(value: str) -> tuple[str, int]:
    require(type(value) is str and 1 <= len(value) <= 128 and value.count(':') == 1,
            'numeric IPv4 loopback endpoint required')
    host, port = value.split(':')
    require(port.isascii() and port.isdecimal() and 1 <= len(port) <= 5, 'invalid port')
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
    token: bytes = field(repr=False)

    @classmethod
    def load(cls, placement: bool = False) -> Authority:
        require(all(not key.startswith('DFMCP_') or key in (OPT_IN, TOKEN, ENDPOINT, PLACE)
                    for key in os.environ), 'unrelated DFMCP authority/admission environment')
        require(os.environ.get(OPT_IN) == '1', 'explicit furniture development opt-in required')
        permission = os.environ.get(PLACE)
        require(permission in (None, '0', '1') and (not placement or permission == '1'),
                'explicit furniture placement permission required')
        token = os.environ.get(TOKEN, '').encode('utf-8')
        require(32 <= len(token) <= 256, 'invalid furniture credential length')
        return cls(endpoint(os.environ.get(ENDPOINT, '127.0.0.1:5000')), token)

    def guard(self, operation: str) -> None:
        require(operation in METHODS, 'operation outside furniture profile')
        current = Authority.load(operation in ('PreparePlacement', 'CommitPlacement'))
        require(current == self, 'operator endpoint or credentials changed')


class Budget:
    """One wall deadline, call allowance and total connection byte allowance."""
    def __init__(self, timeout_ms: int, max_calls: int = 32, max_bytes: int = 524288):
        self.deadline = time.monotonic() + integer(timeout_ms, 1, 60000) / 1000
        self.calls_left = integer(max_calls, 1, 32)
        self.bytes_left = integer(max_bytes, 1, 524288)

    def remaining(self) -> float:
        remaining = self.deadline - time.monotonic()
        require(remaining > 0, 'whole-operation deadline exhausted')
        return remaining

    def reserve_call(self) -> None:
        self.remaining()
        require(self.calls_left > 0, 'native-call allowance exhausted')
        self.calls_left -= 1

    def reserve_bytes(self, count: int) -> None:
        self.remaining()
        integer(count, 0, 524288)
        require(count <= self.bytes_left, 'connection-byte allowance exhausted')
        self.bytes_left -= count


def bounded_text(value: bytes) -> str:
    require(type(value) is bytes and 1 <= len(value) <= 128 and b'\0' not in value,
            'invalid software version')
    try:
        return value.decode('utf-8')
    except UnicodeDecodeError as error:
        raise Rejected('invalid software version encoding') from error


@dataclass(frozen=True)
class Manifest:
    generation: int
    df_version: str
    dfhack_version: str

    def __post_init__(self):
        integer(self.generation, 1, 2**64 - 2)
        for value in (self.df_version, self.dfhack_version):
            require(type(value) is str, 'invalid software version type')
            bounded_text(value.encode('utf-8'))

    def view(self) -> dict:
        return {'generation': self.generation, 'df_version': self.df_version,
                'dfhack_version': self.dfhack_version}


@dataclass(frozen=True)
class Reply:
    manifest: Manifest
    unresolved: bool
    retained_records: int
    capture: Capture | None = None
    record: Record | None = None
    replayed: bool | None = None


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
        integer(number, 1, 13)
        if type(value) is bytes:
            out += varint(number * 8 + 2) + varint(len(value)) + value
        else:
            out += varint(number * 8) + varint(value)
    require(len(out) <= 2048, 'request exceeds 2 KiB')
    return bytes(out)


def decode(raw: bytes, maximum: int = 13) -> dict:
    require(type(raw) is bytes and len(raw) <= MAX_REPLY, 'oversized native reply')
    offset, result = 0, {}

    def number() -> int:
        nonlocal offset
        value = 0
        for index in range(10):
            require(offset < len(raw), 'truncated varint')
            byte = raw[offset]
            offset += 1
            require(index < 9 or byte <= 1, 'varint overflow')
            value |= (byte & 127) << (7 * index)
            if byte < 128:
                require(index == 0 or byte != 0, 'nonminimal varint')
                return value
        raise Rejected('unterminated varint')

    while offset < len(raw):
        tag = number()
        key, wire_type = tag >> 3, tag & 7
        require(1 <= key <= maximum and key not in result, 'unknown or duplicate field')
        if wire_type == 0:
            value = number()
        elif wire_type == 2:
            width = number()
            require(width <= len(raw) - offset, 'truncated byte field')
            value = raw[offset:offset + width]
            offset += width
        else:
            raise Rejected('unsupported protobuf wire type')
        result[key] = value
    return result


class Client:
    """An exact selection/source connection. Import, query and replay grant no dispatch."""
    def __init__(self, authority: Authority, budget: Budget, selection: Selection):
        selection.encode()
        self.authority, self.budget, self.selection = authority, budget, selection
        self.manifest = None
        self._permit = None
        self._commit_attempted = False
        self._records = {}
        self._methods = {}
        self._closed = False
        self._nonce = secrets.token_bytes(32)
        self._socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            authority.guard('Handshake')
            self._socket.settimeout(budget.remaining())
            self._socket.connect(authority.address)
            self._send(b'DFHack?\n' + struct.pack('<i', 1))
            require(self._read(12) == b'DFHack!\n' + struct.pack('<i', 1), 'wrong native greeting')
            for name in METHODS:
                authority.guard('Handshake')
                bound = decode(self._frame(0, encode({1: name.encode('ascii'),
                    2: b'dfmcp.build.v1_19.Request', 3: b'dfmcp.build.v1_19.Reply',
                    4: b'dfmcp_build_v1_19'})), 1)
                require(set(bound) == {1}, 'invalid native method binding')
                method = integer(bound[1], 2, 32767)
                require(method not in self._methods.values(), 'aliased native method IDs')
                self._methods[name] = method
            self._call('Handshake')
        except BaseException:
            self.close()
            raise

    def _send(self, raw: bytes) -> None:
        self.budget.reserve_bytes(len(raw))
        self._socket.settimeout(self.budget.remaining())
        self._socket.sendall(raw)

    def _read(self, size: int) -> bytes:
        self.budget.reserve_bytes(size)
        out = bytearray()
        while len(out) < size:
            self._socket.settimeout(self.budget.remaining())
            part = self._socket.recv(size - len(out))
            require(bool(part), 'native reply incomplete')
            out += part
        self.budget.remaining()
        return bytes(out)

    def _frame(self, method: int, payload: bytes) -> bytes:
        require(not self._closed and len(payload) <= 2048, 'closed or oversized native call')
        self.budget.reserve_call()
        self._send(struct.pack('<h2xi', method, len(payload)) + payload)
        notifications, total = 0, 0
        while True:
            kind, width = struct.unpack('<h2xi', self._read(8))
            require(kind in (-1, -3), 'native transport refused request')
            require(0 <= width <= (MAX_REPLY if kind == -1 else 65536), 'native reply frame too large')
            if kind == -3:
                notifications += 1
                total += width
                require(notifications <= 8 and total <= 262144, 'native notification allowance exhausted')
            raw = self._read(width)
            if kind == -1:
                return raw

    def _call(self, operation: str, fields: dict | None = None, plan: Plan | None = None) -> Reply:
        try:
            self.authority.guard(operation)
            supplied = fields or {}
            require(all(type(n) is int and 5 <= n <= 13 for n in supplied), 'cannot override credentials')
            response = decode(self._frame(self._methods[operation], encode({1: self.authority.token,
                2: self._nonce, 3: 1, 4: 19, **supplied})))
            require(set(range(1, 9)) <= set(response), 'required reply field missing')
            for n in (1, 2, 4, 5, 6):
                require(type(response[n]) is int, 'wrong numeric wire type')
            for n in (3, 7, 8):
                require(type(response[n]) is bytes, 'wrong text wire type')
            require(response[3] == self._nonce and response[4] == 1 and response[5] == 19,
                    'reply nonce or profile changed')
            accepted = integer(response[1], 0, 1)
            code = integer(response[2], 0, 8)
            if not accepted:
                require(code != 0 and set(response) == set(range(1, 9)) and response[6] == 0
                        and response[7] == response[8] == b'', 'noncanonical native refusal')
                raise Rejected('native request refused; effect outcome remains unproved')
            required = set(range(1, 9)) | {12, 13}
            if operation == 'ReadPlacement':
                required.add(9)
            if operation in ('PreparePlacement', 'CommitPlacement', 'CancelPlacement') or (operation == 'QueryPlacement' and 10 in response):
                required.add(10)
            if operation == 'PreparePlacement':
                required.add(11)
            require(code == 0 and set(response) == required, 'unexpected success fields')
            manifest = Manifest(response[6], bounded_text(response[7]), bounded_text(response[8]))
            require(self.manifest is None or manifest == self.manifest, 'native incarnation or software changed')
            unresolved = bool(integer(response[12], 0, 1))
            retained = integer(response[13], 0, 256)
            require(not unresolved or retained > 0, 'unresolved work without a retained record')
            capture = Capture.decode(response[9]) if 9 in response else None
            record = Record.decode(response[10], plan) if 10 in response else None
            replayed = bool(integer(response[11], 0, 1)) if 11 in response else None
            if capture is not None:
                require(capture.selection == self.selection and capture.generation == manifest.generation,
                        'observation selection or source mismatch')
            if record is not None:
                require(plan is not None and record.plan == plan and retained > 0
                        and record.plan.before.generation <= manifest.generation, 'receipt plan or source mismatch')
                require(record.phase != 'indeterminate' or unresolved, 'indeterminate effect without native fence')
                if operation == 'PreparePlacement' and not replayed:
                    require(record.phase == 'prepared' and not unresolved
                            and record.plan.before.generation == manifest.generation,
                            'fresh preparation has historical evidence')
                previous = self._records.get(plan.key)
                if previous is not None:
                    successor(previous, record)
                require(previous is not None or len(self._records) < 32, 'connection record allowance exhausted')
                self._records[plan.key] = record
            elif plan is not None:
                require(plan.key not in self._records, 'known native record disappeared')
            self.authority.guard('QueryPlacement')
            self.budget.remaining()
            self.manifest = manifest
            return Reply(manifest, unresolved, retained, capture, record, replayed)
        except BaseException:
            self.close()
            raise

    def _check_plan(self, plan: Plan) -> None:
        require(Plan(plan.key, plan.before) == plan and plan.before.selection == self.selection,
                'invalid or out-of-selection plan')

    @staticmethod
    def _keyed(plan: Plan, token: bool = False) -> dict:
        fields = {10: plan.key.encode('ascii'), 12: plan.digest}
        if token:
            fields[13] = plan.token
        return fields

    def _selection_fields(self) -> dict:
        value = self.selection
        return dict(zip(range(5, 10), (value.kind, value.item, value.x, value.y, value.z)))

    def observe(self) -> Reply:
        return self._call('ReadPlacement', self._selection_fields())

    def prepare(self, plan: Plan) -> Reply:
        self._check_plan(plan)
        require(not self._closed and not self._commit_attempted and self._permit is None and not self._records,
                'connection already has retained work; cannot manufacture a replay permit')
        require(plan.before.generation == self.manifest.generation, 'stale source before preparation')
        reply = self._call('PreparePlacement', {**self._selection_fields(), **self._keyed(plan),
                           11: plan.before.witness}, plan)
        if reply.record.phase == 'prepared' and reply.replayed is False:
            self._permit = plan
        return reply

    def commit(self, plan: Plan) -> Reply:
        self._check_plan(plan)
        require(not self._closed and not self._commit_attempted and self._permit == plan,
                'no fresh connection-owned one-shot commit permit')
        self._permit = None
        self._commit_attempted = True
        reply = self._call('CommitPlacement', self._keyed(plan, True), plan)
        if reply.record.phase == 'prepared':
            self.close()
            raise Rejected('commit returned contradictory Prepared evidence')
        return reply

    def query(self, plan: Plan) -> Reply:
        self._check_plan(plan)
        # A recovery read relinquishes a preparation permit; it cannot restore one.
        self._permit = None
        return self._call('QueryPlacement', self._keyed(plan), plan)

    def cancel(self, plan: Plan) -> Reply:
        self._check_plan(plan)
        self._permit = None
        reply = self._call('CancelPlacement', self._keyed(plan, True), plan)
        if reply.record.phase == 'prepared':
            self.close()
            raise Rejected('cancellation did not retire native preparation')
        return reply

    def close(self) -> None:
        self._closed = True
        self._permit = None
        self._socket.close()

    def __enter__(self) -> Client:
        return self

    def __exit__(self, *_args):
        self.close()
