"""Read-only map/1.5 evidence and sampled dry-floor goals. No mutation interface.

A satisfied floor goal is an observed condition, not attribution to a mining
operation, continuous stability, current safety, or authority to retry a dig.
"""
from __future__ import annotations

from dataclasses import dataclass, replace
import hashlib
import ipaddress
import json
import secrets
import socket
import struct
import time

MAX_CAPTURE = 1024 * 1024
MAX_TICK = (2**32 - 1) * 403200 + 403199
MAX_WIRE = 4 * 1024 * 1024
TERMINAL = frozenset(('satisfied', 'expired', 'invalidated', 'cancelled'))


class Rejected(ValueError):
    """A refused read or invalid evidence; never proof of nonapplication."""


def require(ok: bool, message: str) -> None:
    if not ok:
        raise Rejected(message)


def integer(value: object, low: int, high: int) -> int:
    require(type(value) is int and low <= value <= high, 'integer outside bounds')
    return value


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True,
                      allow_nan=False).encode('ascii')


def unique_object(pairs: list[tuple[str, object]]) -> dict:
    value = {}
    for key, item in pairs:
        require(key not in value, 'duplicate JSON field')
        value[key] = item
    return value


def text(raw: bytes, maximum: int) -> str:
    require(isinstance(raw, bytes) and 1 <= len(raw) <= maximum and b'\0' not in raw,
            'invalid bounded text')
    try:
        return raw.decode('utf-8')
    except UnicodeDecodeError as cause:
        raise Rejected('invalid UTF-8') from cause


@dataclass(frozen=True)
class Region:
    origin: tuple[int, int, int]
    size: tuple[int, int, int]

    def __post_init__(self) -> None:
        require(type(self.origin) is tuple and type(self.size) is tuple
                and len(self.origin) == len(self.size) == 3, 'invalid map region')
        for start, size in zip(self.origin, self.size):
            integer(start, 0, 32767)
            integer(size, 1, 128)
            require(start + size <= 32768, 'map region overflow')
        require(self.volume <= 16384, 'map region volume exceeds profile')

    @property
    def volume(self) -> int:
        x, y, z = self.size
        return x * y * z

    def json(self) -> dict:
        return {'origin': list(self.origin), 'size': list(self.size)}

    @classmethod
    def from_json(cls, value: object) -> Region:
        require(isinstance(value, dict) and set(value) == {'origin', 'size'}
                and type(value['origin']) is list and type(value['size']) is list,
                'invalid region fields')
        return cls(tuple(value['origin']), tuple(value['size']))


@dataclass(frozen=True)
class Manifest:
    generation: int
    df_version: str
    dfhack_version: str

    def __post_init__(self) -> None:
        integer(self.generation, 1, 2**64 - 2)
        for value in (self.df_version, self.dfhack_version):
            require(isinstance(value, str), 'invalid version type')
            text(value.encode('utf-8'), 128)

    def json(self) -> dict:
        return {'generation': self.generation, 'df_version': self.df_version,
                'dfhack_version': self.dfhack_version}

    @classmethod
    def from_json(cls, value: object) -> Manifest:
        require(isinstance(value, dict) and set(value) == {
            'generation', 'df_version', 'dfhack_version'}, 'invalid source manifest')
        return cls(**value)


class Reader:
    def __init__(self, raw: bytes):
        self.raw, self.offset = raw, 0

    def take(self, size: int) -> bytes:
        require(0 <= size <= len(self.raw) - self.offset, 'truncated map capture')
        value = self.raw[self.offset:self.offset + size]
        self.offset += size
        return value

    def number(self, fmt: str) -> int:
        return struct.unpack('>' + fmt, self.take(struct.calcsize('>' + fmt)))[0]

    def done(self) -> None:
        require(self.offset == len(self.raw), 'trailing map capture bytes')


@dataclass(frozen=True)
class Tile:
    presence: int
    # Empty for hidden/missing; never zero-filled attributes masquerading as facts.
    attributes: tuple[int, ...] = ()


@dataclass(frozen=True)
class Capture:
    raw: bytes
    manifest: Manifest
    region: Region
    tick: int
    paused: bool
    site: int
    folder: str
    dimensions: tuple[int, int, int]
    tiles: tuple[Tile, ...]

    @property
    def witness(self) -> str:
        identity = canonical(self.manifest.json())
        return hashlib.sha256(b'dfmcp-map-goal-evidence/1\0' + struct.pack('>I', len(identity))
                              + identity + self.raw).hexdigest()

    def binding(self) -> dict:
        return {'manifest': self.manifest.json(), 'region': self.region.json(),
                'folder': self.folder, 'site': self.site, 'dimensions': list(self.dimensions)}


def decode_capture(raw: bytes, manifest: Manifest, region: Region) -> Capture:
    require(isinstance(raw, bytes) and 1 <= len(raw) <= MAX_CAPTURE, 'invalid capture extent')
    r = Reader(raw)
    require(r.take(8) == b'DFMM1500', 'not a map/1.5 capture')
    year, year_tick = r.number('I'), r.number('I')
    integer(year_tick, 0, 403199)
    paused = bool(integer(r.number('B'), 0, 1))
    site = integer(r.number('I'), 0, 2**31 - 1)
    folder = text(r.take(r.number('H')), 512)
    dimensions = tuple(integer(r.number('I'), 1, 32768) for _ in range(3))
    origin = tuple(r.number('I') for _ in range(3))
    size = tuple(r.number('I') for _ in range(3))
    require(Region(origin, size) == region and r.number('I') == region.volume,
            'capture substituted selection or omitted cells')
    require(all(a + b <= c for a, b, c in zip(origin, size, dimensions)), 'selection outside map')
    tiles = []
    for _ in range(region.volume):
        presence = integer(r.number('B'), 0, 2)
        attributes = ()
        if presence == 2:
            # tiletype, shape, depth, magma, traffic, dig, building, units,
            # walkable, temperature1, temperature2 -- unchanged native layout.
            attributes = struct.unpack('>IBBBBBBBIHH', r.take(19))
            for value, maximum in zip(attributes[1:8], (8, 7, 1, 3, 7, 7, 3)):
                integer(value, 0, maximum)
        tiles.append(Tile(presence, attributes))
    r.done()
    return Capture(raw, manifest, region, year * 403200 + year_tick, paused, site,
                   folder, dimensions, tuple(tiles))


def classify(capture: Capture) -> dict[str, int]:
    counts = dict.fromkeys(('floor_goal', 'wall', 'other_shape', 'wet_floor',
                           'designated_floor', 'hidden', 'missing', 'active_designations'), 0)
    for tile in capture.tiles:
        if tile.presence != 2:
            counts['hidden' if tile.presence == 1 else 'missing'] += 1
            continue
        _, shape, depth, _, _, dig, *_ = tile.attributes
        counts['active_designations'] += int(dig != 0)
        category = ('wall' if shape == 2 else 'other_shape' if shape != 3 else
                    'wet_floor' if depth != 0 else 'designated_floor' if dig != 0 else 'floor_goal')
        counts[category] += 1
    return counts


@dataclass(frozen=True)
class Goal:
    region: Region
    folder: str
    site: int
    deadline_tick: int
    stable_ticks: int = 10
    required_samples: int = 2
    max_gap_ticks: int = 1200

    def __post_init__(self) -> None:
        require(isinstance(self.folder, str), 'invalid goal fortress')
        text(self.folder.encode('utf-8'), 512)
        integer(self.site, 0, 2**31 - 1)
        integer(self.deadline_tick, 0, MAX_TICK)
        integer(self.stable_ticks, 0, 403200)
        integer(self.required_samples, 1, 128)
        integer(self.max_gap_ticks, 1, 403200)
        require(self.region.size[2] == 1 and max(self.region.size[:2]) <= 8,
                'floor goal is one 1..8 by 1..8 rectangle')

    def json(self) -> dict:
        return {'region': self.region.json(), 'folder': self.folder, 'site': self.site,
                'deadline_tick': self.deadline_tick, 'stable_ticks': self.stable_ticks,
                'required_samples': self.required_samples, 'max_gap_ticks': self.max_gap_ticks}

    @classmethod
    def from_json(cls, value: object) -> Goal:
        require(isinstance(value, dict) and set(value) == {'region', 'folder', 'site',
            'deadline_tick', 'stable_ticks', 'required_samples', 'max_gap_ticks'}, 'invalid goal fields')
        return cls(**{**value, 'region': Region.from_json(value['region'])})


@dataclass(frozen=True)
class Progress:
    first: Capture
    latest: Capture
    status: str
    streak: int = 0
    since_tick: int | None = None
    observations: int = 0
    interruption: str | None = None

    def interrupted(self, reason: str) -> Progress:
        require(self.status not in TERMINAL, 'terminal goal is immutable')
        return replace(self, status='unknown', streak=0, since_tick=None, interruption=reason)


def advance(goal: Goal, prior: Progress | None, capture: Capture) -> Progress:
    # Do not trust a caller-constructed derived Capture over its native bytes.
    capture = decode_capture(capture.raw, capture.manifest, goal.region)
    if prior is not None:
        require(prior.status not in TERMINAL, 'terminal goal is immutable')
        if capture.binding() != prior.first.binding() or capture.tick < prior.latest.tick:
            return replace(prior, status='invalidated', streak=0, since_tick=None,
                           interruption='source_identity_or_clock_changed')
    else:
        require(capture.folder == goal.folder and capture.site == goal.site,
                'observed fortress differs from explicit goal')
        require(capture.tick <= goal.deadline_tick, 'goal deadline already passed at creation')
    counts = classify(capture)
    first = capture if prior is None else prior.first
    observations = 1 if prior is None else prior.observations + 1
    if capture.tick > goal.deadline_tick:
        return Progress(first, capture, 'expired', observations=observations)
    if counts['hidden'] or counts['missing']:
        return Progress(first, capture, 'unknown', observations=observations)
    if counts['floor_goal'] != goal.region.volume:
        return Progress(first, capture, 'pending', observations=observations)
    gap = prior is not None and capture.tick - prior.latest.tick > goal.max_gap_ticks
    continuing = prior is not None and prior.streak > 0 and not gap
    since = prior.since_tick if continuing else capture.tick
    streak = prior.streak + int(capture.tick > prior.latest.tick) if continuing else 1
    satisfied = streak >= goal.required_samples and capture.tick - since >= goal.stable_ticks
    return Progress(first, capture, 'satisfied' if satisfied else 'stabilizing', streak, since,
                    observations, 'sample_gap_reset' if gap else None)


def endpoint(raw: str) -> tuple[str, int]:
    require(isinstance(raw, str) and len(raw) <= 128 and raw.count(':') == 1,
            'expected numeric IPv4 loopback endpoint')
    host, port = raw.split(':')
    try:
        address = ipaddress.IPv4Address(host)
        number = int(port)
    except ValueError as cause:
        raise Rejected('invalid numeric loopback endpoint') from cause
    integer(number, 1, 65535)
    require(address.is_loopback and f'{address}:{number}' == raw, 'noncanonical or non-loopback endpoint')
    return str(address), number


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
    for field, value in sorted(fields.items()):
        integer(field, 1, 11)
        if isinstance(value, bytes):
            out += varint(field << 3 | 2) + varint(len(value)) + value
        else:
            out += varint(field << 3) + varint(value)
    return bytes(out)


def decode(raw: bytes, maximum: int = 9) -> dict[int, int | bytes]:
    r = Reader(raw)
    def number() -> int:
        start, value = r.offset, 0
        for shift in range(0, 70, 7):
            byte = r.number('B')
            require(shift < 63 or byte <= 1, 'protobuf overflow')
            value |= (byte & 127) << shift
            if byte < 128:
                require(raw[start:r.offset] == varint(value), 'nonminimal protobuf integer')
                return value
        raise Rejected('unterminated protobuf integer')
    fields = {}
    while r.offset < len(raw):
        tag = number()
        field, wire = tag >> 3, tag & 7
        require(1 <= field <= maximum and field not in fields and wire in (0, 2),
                'unknown/duplicate protobuf field or wire type')
        fields[field] = number() if wire == 0 else r.take(number())
    return fields


class MapClient:
    """Two fixed read-only methods, one connection, one absolute deadline."""
    def __init__(self, address: str, token: bytes, region: Region, timeout_ms: int = 10000):
        target = endpoint(address)
        integer(timeout_ms, 1, 60000)
        require(isinstance(token, bytes) and 32 <= len(token) <= 256, 'invalid map credential bound')
        self.region, self.token, self.nonce = region, token, secrets.token_bytes(32)
        self.deadline = time.monotonic() + timeout_ms / 1000
        self.left, self.closed, self.sampled = MAX_WIRE, False, False
        self.manifest = None
        self.methods = []
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            self.sock.settimeout(self.remaining())
            self.sock.connect(target)
            self.send(b'DFHack?\n' + struct.pack('<i', 1))
            require(self.read(12) == b'DFHack!\n' + struct.pack('<i', 1), 'invalid DFHack greeting')
            for name in ('Handshake', 'ReadObservation'):
                binding = decode(self.frame(0, encode({1: name.encode(), 2: b'dfmcp.map.v1_5.Request',
                    3: b'dfmcp.map.v1_5.Reply', 4: b'dfmcp_map_v1_5'})), 1)
                require(set(binding) == {1}, 'invalid map binding')
                method = integer(binding[1], 2, 32767)
                require(method not in self.methods, 'aliased map methods')
                self.methods.append(method)
            self.invoke(False)
        except BaseException:
            self.close()
            raise

    def __enter__(self) -> MapClient:
        return self

    def __exit__(self, *_args) -> None:
        self.close()

    def close(self) -> None:
        if not self.closed:
            self.closed = True
            self.sock.close()

    def remaining(self) -> float:
        require(not self.closed, 'map stream is fenced')
        value = self.deadline - time.monotonic()
        require(value > 0, 'absolute map deadline exhausted')
        return value

    def charge(self, size: int) -> None:
        require(0 <= size <= self.left, 'map connection byte allowance exhausted')
        self.left -= size

    def send(self, raw: bytes) -> None:
        self.charge(len(raw))
        self.sock.settimeout(self.remaining())
        self.sock.sendall(raw)

    def read(self, size: int) -> bytes:
        self.charge(size)
        out = bytearray()
        while len(out) < size:
            self.sock.settimeout(self.remaining())
            part = self.sock.recv(size - len(out))
            require(bool(part), 'truncated map RPC')
            out += part
        return bytes(out)

    def frame(self, method: int, request: bytes) -> bytes:
        require(len(request) <= 2048, 'oversized map request')
        self.send(struct.pack('<h2xi', method, len(request)) + request)
        count, total = 0, 0
        while True:
            kind, size = struct.unpack('<h2xi', self.read(8))
            require(kind in (-1, -3) and 0 <= size <= (MAX_CAPTURE + 1024 if kind == -1 else 65536),
                    'invalid map response frame')
            if kind == -3:
                count += 1
                total += size
                require(count <= 8 and total <= 262144, 'map notification allowance exhausted')
            raw = self.read(size)
            if kind == -1:
                self.remaining()
                return raw

    def invoke(self, observe: bool) -> Capture | None:
        try:
            self.remaining()
            require(not observe or not self.sampled, 'one capture per map connection')
            if observe:
                self.sampled = True
            fields = {1: self.token, 2: self.nonce, 3: 1, 4: 5,
                      **dict(zip(range(5, 11), self.region.origin + self.region.size)),
                      11: max(1024, 575 + 20 * self.region.volume)}
            response = decode(self.frame(self.methods[int(observe)], encode(fields)))
            require(set(range(1, 9)) <= set(response), 'incomplete map reply')
            for key in (1, 2, 4, 5, 6):
                require(type(response[key]) is int, 'invalid map scalar wire type')
            for key in (3, 7, 8):
                require(type(response[key]) is bytes, 'invalid map text wire type')
            require(response[3] == self.nonce and response[4] == 1 and response[5] == 5,
                    'map nonce or protocol mismatch')
            accepted = integer(response[1], 0, 1)
            code = integer(response[2], 0, 5)
            if not accepted:
                require(code != 0 and set(response) == set(range(1, 9)), 'malformed native refusal')
                raise Rejected('native map read refused; no goal outcome inferred')
            require(code == 0 and set(response) == set(range(1, 10 if observe else 9)),
                    'unexpected map payload or success code')
            manifest = Manifest(response[6], text(response[7], 128), text(response[8], 128))
            require(self.manifest is None or self.manifest == manifest, 'map source changed within connection')
            capture = decode_capture(response[9], manifest, self.region) if observe else None
            self.remaining()
            self.manifest = manifest
            return capture
        except BaseException:
            self.close()
            raise

    def observe(self) -> Capture:
        capture = self.invoke(True)
        require(capture is not None, 'map capture missing')
        return capture
