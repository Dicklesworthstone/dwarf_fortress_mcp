"""Fixed foreground DFHack RPC client for excavation-run/1.18; never reconnects."""
from __future__ import annotations

from dataclasses import dataclass, field
import ipaddress
import os
import secrets
import socket
import struct
import time

from excavation_run_wire import Capture, Plan, Record, Region, Rejected, integer, require, successor, text

METHODS = ('Handshake', 'ObserveRun', 'PrepareRun', 'CommitRun', 'QueryRun', 'CancelRun')
OPT_IN = 'DFMCP_ALLOW_UNADMITTED_EXCAVATION_RUN_V1_18'
TOKEN = 'DFMCP_EXCAVATION_RUN_TOKEN'
ENDPOINT = 'DFMCP_EXCAVATION_RUN_ENDPOINT'
CLOCK = 'DFMCP_EXCAVATION_RUN_ALLOW_CLOCK'


def endpoint(value: str) -> tuple[str, int]:
    require(type(value) is str and value.count(':') == 1, 'numeric IPv4 loopback endpoint required')
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
    def load(cls, clock: bool = False) -> Authority:
        require(all(not key.startswith('DFMCP_') or key in (OPT_IN, TOKEN, ENDPOINT, CLOCK)
                    for key in os.environ), 'unrelated DFMCP authority/admission environment')
        require(os.environ.get(OPT_IN) == '1', 'explicit excavation development opt-in required')
        permission = os.environ.get(CLOCK)
        require(permission in (None, '0', '1') and (not clock or permission == '1'),
                'explicit excavation clock permission required')
        token = os.environ.get(TOKEN, '').encode('utf-8')
        require(32 <= len(token) <= 256, 'invalid excavation credential length')
        return cls(endpoint(os.environ.get(ENDPOINT, '127.0.0.1:5000')), token)

    def guard(self, operation: str) -> None:
        current = Authority.load(operation in ('PrepareRun', 'CommitRun'))
        require(current == self, 'operator endpoint or credentials changed')


class Budget:
    """One cooperative wall allowance and a nonrenewable native-call allowance."""
    def __init__(self, timeout_ms: int, max_calls: int = 64):
        integer(timeout_ms, 1, 60000)
        self.deadline = time.monotonic() + timeout_ms / 1000
        self.calls_left = integer(max_calls, 1, 64)

    def remaining(self) -> float:
        remaining = self.deadline - time.monotonic()
        require(remaining > 0, 'whole-operation deadline exhausted')
        return remaining

    def reserve_call(self) -> None:
        self.remaining()
        require(self.calls_left > 0, 'native-call allowance exhausted')
        self.calls_left -= 1


@dataclass(frozen=True)
class Manifest:
    generation: int
    df_version: str
    dfhack_version: str

    def __post_init__(self):
        integer(self.generation, 1, 2**64 - 2)
        for version in (self.df_version, self.dfhack_version):
            require(type(version) is str, 'invalid software version')
            text(version.encode('utf-8'), 128)

    def view(self) -> dict:
        return {'generation': self.generation, 'df_version': self.df_version, 'dfhack_version': self.dfhack_version}


@dataclass(frozen=True)
class Reply:
    manifest: Manifest
    owner_active: bool
    retained_records: int
    capture: Capture | None = None
    record: Record | None = None


def varint(value: int) -> bytes:
    integer(value, 0, 2**64 - 1)
    result = bytearray()
    while value >= 128:
        result.append((value & 127) | 128)
        value >>= 7
    result.append(value)
    return bytes(result)


def encode(fields: dict[int, int | bytes]) -> bytes:
    out = bytearray()
    for number, value in sorted(fields.items()):
        integer(number, 1, 19)
        if type(value) is bytes:
            out += varint(number * 8 + 2) + varint(len(value)) + value
        else:
            out += varint(number * 8) + varint(value)
    require(len(out) <= 2048, 'request exceeds 2 KiB')
    return bytes(out)


def decode(raw: bytes, maximum: int = 12, size_limit: int = 4096) -> dict:
    require(type(raw) is bytes and len(raw) <= size_limit, 'oversized reply')
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
            raise Rejected('unsupported wire type')
        result[key] = value
    return result


class Client:
    """One pinned source, region and connection; only fresh prepare grants one commit.

    Importing/querying Prepared evidence does not grant permission. The caller
    must durably publish dispatch intent before commit; this is the transport,
    not the journal or a production clock lease.
    """
    def __init__(self, authority: Authority, budget: Budget, region: Region):
        self.authority, self.budget, self.region = authority, budget, region
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
                    2: b'dfmcp.excavation_run.v1_18.Request', 3: b'dfmcp.excavation_run.v1_18.Reply',
                    4: b'dfmcp_excavation_run_v1_18'})), 1)
                require(set(bound) == {1}, 'invalid native method binding')
                method = integer(bound[1], 2, 32767)
                require(method not in self._methods.values(), 'aliased native method IDs')
                self._methods[name] = method
            self._call('Handshake')
        except BaseException:
            self.close()
            raise

    def _send(self, raw: bytes) -> None:
        self._socket.settimeout(self.budget.remaining())
        self._socket.sendall(raw)

    def _read(self, size: int) -> bytes:
        out = bytearray()
        while len(out) < size:
            self._socket.settimeout(self.budget.remaining())
            part = self._socket.recv(size - len(out))
            require(bool(part), 'native reply incomplete')
            out += part
        self.budget.remaining()
        return bytes(out)

    def _frame(self, method: int, payload: bytes) -> bytes:
        require(not self._closed and len(payload) <= 2048, 'closed/oversized native call')
        self.budget.reserve_call()
        self._send(struct.pack('<h2xi', method, len(payload)) + payload)
        notifications, total = 0, 0
        while True:
            kind, width = struct.unpack('<h2xi', self._read(8))
            require(kind in (-1, -3), 'native transport refused request')
            require(0 <= width <= (4096 if kind == -1 else 65536), 'native reply frame too large')
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
            response = decode(self._frame(self._methods[operation], encode({1: self.authority.token,
                2: self._nonce, 3: 1, 4: 18, **(fields or {})})))
            require(set(range(1, 9)) <= set(response), 'required reply field missing')
            for n in (1, 2, 4, 5, 6):
                require(type(response[n]) is int, 'wrong numeric wire type')
            for n in (3, 7, 8):
                require(type(response[n]) is bytes, 'wrong text wire type')
            require(response[3] == self._nonce and response[4] == 1 and response[5] == 18,
                    'reply nonce or profile changed')
            accepted = integer(response[1], 0, 1)
            code = integer(response[2], 0, 8)
            if not accepted:
                require(code != 0 and set(response) == set(range(1, 9)) and response[6] == 0
                        and response[7] == response[8] == b'', 'noncanonical refusal')
                raise Rejected('native request refused; effect outcome remains unproved')
            required = set(range(1, 9)) | {11, 12}
            if operation == 'ObserveRun':
                required.add(9)
            if operation in ('PrepareRun', 'CommitRun', 'CancelRun') or (operation == 'QueryRun' and 10 in response):
                required.add(10)
            require(code == 0 and set(response) == required, 'unexpected success fields')
            manifest = Manifest(response[6], text(response[7], 128), text(response[8], 128))
            require(self.manifest is None or manifest == self.manifest, 'native source/manifest changed; reconnect only for recovery')
            owner = bool(integer(response[11], 0, 1))
            retained = integer(response[12], 0, 256)
            require(not owner or retained > 0, 'owner without retained record')
            capture = Capture.decode(response[9]) if 9 in response else None
            record = Record.decode(response[10], plan) if 10 in response else None
            if capture is not None:
                require(capture.region == self.region and capture.generation == manifest.generation,
                        'observation region/source mismatch')
            if record is not None:
                require(plan is not None and record.plan.before.generation <= manifest.generation and retained > 0,
                        'invalid retained record source')
                if record.phase in ('prepared', 'running', 'stopping'):
                    require(record.plan.before.generation == manifest.generation, 'active evidence from historical source')
                if record.phase in ('running', 'stopping'):
                    require(owner, 'running record without native owner')
                previous = self._records.get(record.plan.key)
                if previous is not None:
                    successor(previous, record)
                require(previous is not None or len(self._records) < 32, 'connection record allowance exhausted')
                self._records[record.plan.key] = record
            elif plan is not None:
                require(plan.key not in self._records, 'known native record disappeared')
            self.manifest = manifest
            # Read authorization can change during a response, too.
            self.authority.guard('QueryRun')
            self.budget.remaining()
            return Reply(manifest, owner, retained, capture, record)
        except BaseException:
            self.close()
            raise

    def _check_plan(self, plan: Plan) -> None:
        require(plan.before.region == self.region, 'plan outside connection region')
        require(Plan(plan.key, plan.spec, plan.before) == plan, 'invalid plan')

    @staticmethod
    def _keyed(plan: Plan, token: bool = False) -> dict:
        fields = {5: plan.key.encode('ascii'), 9: plan.digest}
        if token:
            fields[10] = plan.token
        return fields

    def observe(self) -> Reply:
        return self._call('ObserveRun', dict(zip(range(11, 16), self.region.values())))

    def prepare(self, plan: Plan) -> Reply:
        self._check_plan(plan)
        require(not self._commit_attempted and self._permit is None and not self._records,
                'connection already has retained work; prepare cannot manufacture a replay permit')
        require(plan.before.generation == self.manifest.generation, 'stale source before preparation')
        fields = {**self._keyed(plan), 6: plan.spec.game_ticks, 7: plan.spec.wall_ms, 8: plan.before.raw,
                  **dict(zip(range(11, 16), self.region.values())),
                  **dict(zip(range(16, 20), plan.spec.values()[2:]))}
        reply = self._call('PrepareRun', fields, plan)
        if reply.record.phase == 'prepared':
            self._permit = plan
        return reply

    def commit(self, plan: Plan) -> Reply:
        self._check_plan(plan)
        require(not self._commit_attempted and self._permit == plan, 'no fresh one-shot commit permit')
        self._permit = None
        self._commit_attempted = True  # Consume before any possibly failing I/O.
        reply = self._call('CommitRun', self._keyed(plan, True), plan)
        if reply.record.phase == 'prepared':
            self.close()
            raise Rejected('commit returned contradictory Prepared evidence')
        return reply

    def query(self, plan: Plan) -> Reply:
        self._check_plan(plan)
        return self._call('QueryRun', self._keyed(plan), plan)

    def cancel(self, plan: Plan) -> Reply:
        self._check_plan(plan)
        self._permit = None
        reply = self._call('CancelRun', self._keyed(plan, True), plan)
        if reply.record.phase in ('prepared', 'running'):
            self.close()
            raise Rejected('cancellation did not enter stopping or terminal state')
        return reply

    def close(self) -> None:
        self._closed = True
        self._permit = None
        self._socket.close()

    def __enter__(self) -> Client:
        return self

    def __exit__(self, *_args):
        self.close()
