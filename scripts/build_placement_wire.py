"""Strict, effect-free furniture/1.19 canonical values and retained evidence.

The C++ field order in bridge/common/build_placement.h is the wire contract.
Hashes identify evidence, not authority. A placed receipt proves historical
construction-job registration at stage zero, not completed or usable furniture.
Beads: df-dfhack-bridge-plane-c-pic.4 and df-dfhack-bridge-plane-c-pic.5.
"""
from __future__ import annotations

from dataclasses import dataclass, replace
import hashlib
import json
import re
import struct

MAX_U64 = 2**64 - 1
MAX_I32 = 2**31 - 1
MIN_I32 = -(2**31)
MAX_U32 = 2**32 - 1
MAX_TICK = MAX_U32 * 403200 + 403199
MAX_BUILDINGS = 65536
MAX_CAPTURE_BYTES = 2048
MAX_RECORD_BYTES = 6144
PHASES = ('prepared', 'indeterminate', 'placed', 'refused', 'cancelled')
REASONS = ('none', 'stale', 'expired', 'source_changed', 'cancelled', 'native_failure')
KINDS = ('other', 'bed', 'chair', 'table')


class Rejected(ValueError):
    """Malformed or contradictory evidence; never proof of nonapplication."""


def require(ok: bool, message: str) -> None:
    if not ok:
        raise Rejected(message)


def integer(value: object, low: int, high: int) -> int:
    require(type(value) is int and low <= value <= high, 'integer outside profile bounds')
    return value


def flag(value: object) -> int:
    require(type(value) is bool, 'noncanonical Boolean')
    return int(value)


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(',', ':'),
                      ensure_ascii=True, allow_nan=False).encode('ascii')


def digest(domain: str, raw: bytes) -> bytes:
    require(type(domain) is str and '\0' not in domain and type(raw) is bytes,
            'invalid digest input')
    try:
        return hashlib.sha256(domain.encode('ascii') + b'\0' + raw).digest()
    except UnicodeError as error:
        raise Rejected('invalid digest domain') from error


def exact_hex(value: object, size: int | None = None, maximum: int = MAX_RECORD_BYTES) -> bytes:
    require(type(value) is str and 0 < len(value) <= maximum * 2 and len(value) % 2 == 0
            and re.fullmatch('[0-9a-f]+', value) is not None, 'noncanonical hex')
    require(size is None or len(value) == size * 2, 'wrong hex width')
    return bytes.fromhex(value)


def field(raw: bytes) -> bytes:
    require(type(raw) is bytes and len(raw) <= 65535, 'invalid bounded field')
    return struct.pack('>H', len(raw)) + raw


def key_text(key: str) -> bytes:
    require(type(key) is str and re.fullmatch('[A-Za-z0-9_.-]{1,128}', key) is not None,
            'invalid idempotency key')
    return field(key.encode('ascii'))


def text(raw: bytes, maximum: int) -> str:
    require(type(raw) is bytes and 1 <= len(raw) <= maximum and b'\0' not in raw,
            'invalid bounded text')
    try:
        return raw.decode('utf-8', errors='strict')
    except UnicodeError as error:
        raise Rejected('invalid UTF-8') from error


def text_bytes(value: str, maximum: int) -> bytes:
    require(type(value) is str, 'invalid text type')
    try:
        raw = value.encode('utf-8', errors='strict')
    except UnicodeError as error:
        raise Rejected('invalid UTF-8') from error
    text(raw, maximum)
    return raw


def u32(value: int) -> bytes:
    return struct.pack('>I', integer(value, 0, MAX_U32))


def i32(value: int) -> bytes:
    return struct.pack('>i', integer(value, MIN_I32, MAX_I32))


def coordinates(value: tuple[int, int, int]) -> bytes:
    require(type(value) is tuple and len(value) == 3, 'invalid coordinate tuple')
    return b''.join(u32(integer(n, 0, 32767)) for n in value)


class Reader:
    def __init__(self, raw: bytes, maximum: int):
        require(type(raw) is bytes and len(raw) <= maximum, 'oversized/nonbyte record')
        self.raw, self.offset = raw, 0

    def take(self, size: int) -> bytes:
        require(type(size) is int and 0 <= size <= len(self.raw) - self.offset, 'truncated record')
        start = self.offset
        self.offset += size
        return self.raw[start:self.offset]

    def number(self, width: int, *, signed: bool = False) -> int:
        return int.from_bytes(self.take(width), 'big', signed=signed)

    def flag(self) -> bool:
        return bool(integer(self.number(1), 0, 1))

    def bounded(self, maximum: int) -> bytes:
        return self.take(integer(self.number(2), 1, maximum))

    def finish(self) -> None:
        require(self.offset == len(self.raw), 'trailing record bytes')


@dataclass(frozen=True)
class Selection:
    kind: int
    item: int
    x: int
    y: int
    z: int

    def __post_init__(self):
        self.encode()

    @property
    def target(self) -> tuple[int, int, int]:
        return self.x, self.y, self.z

    def values(self) -> tuple[int, ...]:
        return self.kind, self.item, self.x, self.y, self.z

    def encode(self) -> bytes:
        integer(self.kind, 1, 3)
        integer(self.item, 0, MAX_I32 - 1)
        integer(self.x, 1, 32766)
        integer(self.y, 1, 32766)
        return bytes([self.kind]) + u32(self.item) + coordinates(self.target)

    @classmethod
    def read(cls, r: Reader) -> Selection:
        return cls(r.number(1), *(r.number(4) for _ in range(4)))

    @classmethod
    def decode(cls, raw: bytes) -> Selection:
        r = Reader(raw, 17)
        value = cls.read(r)
        r.finish()
        return value

    def view(self) -> dict:
        return {'kind': KINDS[self.kind], 'item_id': self.item, 'target': list(self.target)}


@dataclass(frozen=True)
class Tile:
    presence: int = 0
    tiletype: int = 0
    shape: int = 0
    liquid: int = 0
    dig: int = 0
    occupancy_other: int = 0
    occupied: bool = False
    building: int | None = None

    def __post_init__(self):
        self.encode()

    def encode(self) -> bytes:
        integer(self.presence, 0, 2)
        integer(self.tiletype, 0, MAX_I32)
        integer(self.shape, 0, 8)
        integer(self.liquid, 0, 7)
        integer(self.dig, 0, 7)
        integer(self.occupancy_other, 0, MAX_U32)
        flag(self.occupied)
        if self.building is not None:
            integer(self.building, 0, MAX_I32 - 1)
        if self.presence != 2:
            require(not any((self.tiletype, self.shape, self.liquid, self.dig,
                             self.occupancy_other, self.occupied)) and self.building is None,
                    'redacted tile contains attributes')
            return bytes([self.presence])
        return (bytes([2]) + u32(self.tiletype) + bytes([self.shape, self.liquid, self.dig])
                + u32(self.occupancy_other) + bytes([self.occupied, self.building is not None])
                + (u32(self.building) if self.building is not None else b''))

    @classmethod
    def read(cls, r: Reader) -> Tile:
        presence = integer(r.number(1), 0, 2)
        if presence != 2:
            return cls(presence)
        tiletype, shape, liquid, dig, other = r.number(4), r.number(1), r.number(1), r.number(1), r.number(4)
        occupied = r.flag()
        building = r.number(4) if r.flag() else None
        return cls(presence, tiletype, shape, liquid, dig, other, occupied, building)

    @classmethod
    def decode(cls, raw: bytes) -> Tile:
        r = Reader(raw, 18)
        value = cls.read(r)
        r.finish()
        return value

    @property
    def dry(self) -> bool:
        return self.presence == 2 and self.liquid == 0

    @property
    def empty_floor(self) -> bool:
        return (self.dry and self.shape == 3 and self.dig == 0 and not self.occupied
                and self.building is None and self.occupancy_other == 0)

    def view(self) -> dict:
        out = {'presence': ('missing', 'hidden', 'visible')[self.presence]}
        if self.presence == 2:
            out.update(tiletype=self.tiletype, shape=self.shape, liquid=self.liquid, dig=self.dig,
                       occupancy_other=self.occupancy_other, occupied=self.occupied, building_id=self.building)
        return out


@dataclass(frozen=True)
class Item:
    presence: int = 0
    pos: tuple[int, int, int] = (0, 0, 0)
    kind: int = 0
    native_type: int = 0
    subtype: int = 0
    material: int = 0
    material_index: int = 0
    quality: int = 0
    wear: int = 0
    other_flags: int = 0
    other_refs: int = 0
    on_ground: bool = False
    in_job: bool = False
    jobs: tuple[int, ...] = ()
    ground: Tile = Tile()

    def __post_init__(self):
        self.encode()

    def encode(self) -> bytes:
        integer(self.presence, 0, 2)
        coordinates(self.pos)
        integer(self.kind, 0, 3)
        for n in (self.native_type, self.quality, self.wear):
            integer(n, 0, MAX_I32)
        for n in (self.subtype, self.material, self.material_index):
            integer(n, MIN_I32, MAX_I32)
        integer(self.other_flags, 0, MAX_U32)
        integer(self.other_refs, 0, 4096)
        flag(self.on_ground)
        flag(self.in_job)
        require(type(self.jobs) is tuple and len(self.jobs) <= 8, 'invalid item job list')
        previous = -1
        for job in self.jobs:
            integer(job, 0, MAX_I32 - 1)
            require(job > previous, 'unordered/duplicate item jobs')
            previous = job
        require(type(self.ground) is Tile, 'invalid item ground')
        ground = self.ground.encode()
        if self.presence != 2:
            require(self.pos == (0, 0, 0) and not any((self.kind, self.native_type, self.subtype,
                    self.material, self.material_index, self.quality, self.wear, self.other_flags,
                    self.other_refs, self.on_ground, self.in_job, self.jobs)) and ground == b'\0',
                    'redacted item contains attributes')
            return bytes([self.presence])
        require(self.ground.presence == 2, 'visible item requires visible ground')
        return (bytes([2]) + coordinates(self.pos) + bytes([self.kind]) + u32(self.native_type)
                + i32(self.subtype) + i32(self.material) + i32(self.material_index)
                + b''.join(u32(n) for n in (self.quality, self.wear, self.other_flags, self.other_refs))
                + bytes([self.on_ground, self.in_job, len(self.jobs)])
                + b''.join(u32(n) for n in self.jobs) + ground)

    @classmethod
    def read(cls, r: Reader) -> Item:
        presence = integer(r.number(1), 0, 2)
        if presence != 2:
            return cls(presence)
        pos = tuple(r.number(4) for _ in range(3))
        kind, native_type = r.number(1), r.number(4)
        subtype, material, material_index = (r.number(4, signed=True) for _ in range(3))
        quality, wear, other_flags, other_refs = (r.number(4) for _ in range(4))
        on_ground, in_job = r.flag(), r.flag()
        jobs = tuple(r.number(4) for _ in range(integer(r.number(1), 0, 8)))
        return cls(presence, pos, kind, native_type, subtype, material, material_index,
                   quality, wear, other_flags, other_refs, on_ground, in_job, jobs, Tile.read(r))

    @classmethod
    def decode(cls, raw: bytes) -> Item:
        r = Reader(raw, 100)
        value = cls.read(r)
        r.finish()
        return value

    def available(self, kind: int) -> bool:
        integer(kind, 1, 3)
        return (self.presence == 2 and self.kind == kind and self.on_ground and not self.in_job
                and not self.other_flags and not self.other_refs and not self.jobs and self.wear == 0
                and self.material >= 0 and self.ground.dry and self.ground.shape == 3
                and not self.ground.occupied)

    def view(self) -> dict:
        out = {'presence': ('missing', 'hidden', 'visible')[self.presence]}
        if self.presence == 2:
            out.update(pos=list(self.pos), kind=KINDS[self.kind], native_type=self.native_type,
                       subtype=self.subtype, material=self.material, material_index=self.material_index,
                       quality=self.quality, wear=self.wear, other_flags=self.other_flags,
                       other_refs=self.other_refs, on_ground=self.on_ground, in_job=self.in_job,
                       jobs=list(self.jobs), ground=self.ground.view())
        return out


@dataclass(frozen=True)
class Capture:
    generation: int
    sequence: int
    tick: int
    site: int
    dimensions: tuple[int, int, int]
    next_building: int
    next_job: int
    building_count: int
    folder: str
    paused: bool
    free_tile: bool
    supported: bool
    selection: Selection
    tiles: tuple[Tile, ...]
    item: Item

    def __post_init__(self):
        self.encode()

    def encode(self) -> bytes:
        integer(self.generation, 1, MAX_U64 - 1)
        integer(self.sequence, 0, MAX_U64 - 1)
        integer(self.tick, 0, MAX_TICK)
        for n in (self.site, self.next_building, self.next_job):
            integer(n, 0, MAX_I32)
        integer(self.building_count, 0, MAX_BUILDINGS)
        require(type(self.dimensions) is tuple and len(self.dimensions) == 3, 'invalid dimensions')
        for d in self.dimensions:
            integer(d, 1, 32768)
        folder = text_bytes(self.folder, 512)
        for b in (self.paused, self.free_tile, self.supported):
            flag(b)
        require(type(self.selection) is Selection, 'invalid selection')
        selection = self.selection.encode()
        p = self.selection.target
        require(p[0] + 1 < self.dimensions[0] and p[1] + 1 < self.dimensions[1]
                and p[2] < self.dimensions[2], 'target context outside map')
        require(type(self.tiles) is tuple and len(self.tiles) == 9
                and all(type(t) is Tile for t in self.tiles), 'capture needs nine ordered tiles')
        require(type(self.item) is Item, 'invalid captured item')
        if self.item.presence == 2:
            require(all(p < d for p, d in zip(self.item.pos, self.dimensions)), 'item outside map')
        out = (b'DFMBC019' + struct.pack('>QQQ', self.generation, self.sequence, self.tick)
               + b''.join(u32(n) for n in (self.site, *self.dimensions,
                                          self.next_building, self.next_job, self.building_count))
               + field(folder) + bytes([self.paused, self.free_tile, self.supported])
               + selection + b''.join(t.encode() for t in self.tiles) + self.item.encode())
        require(len(out) <= MAX_CAPTURE_BYTES, 'oversized capture')
        return out

    @classmethod
    def decode(cls, raw: bytes) -> Capture:
        r = Reader(raw, MAX_CAPTURE_BYTES)
        require(r.take(8) == b'DFMBC019', 'wrong capture profile')
        generation, sequence, tick = (r.number(8) for _ in range(3))
        site = r.number(4)
        dimensions = tuple(r.number(4) for _ in range(3))
        next_building, next_job, building_count = (r.number(4) for _ in range(3))
        folder = text(r.bounded(512), 512)
        paused, free_tile, supported = r.flag(), r.flag(), r.flag()
        selection = Selection.read(r)
        tiles = tuple(Tile.read(r) for _ in range(9))
        item = Item.read(r)
        r.finish()
        out = cls(generation, sequence, tick, site, dimensions, next_building, next_job, building_count,
                  folder, paused, free_tile, supported, selection, tiles, item)
        require(out.encode() == raw, 'noncanonical capture')
        return out

    @property
    def raw(self) -> bytes:
        return self.encode()

    @property
    def witness(self) -> bytes:
        return hashlib.sha256(self.raw).digest()

    @property
    def identity(self) -> tuple:
        return self.generation, self.site, self.dimensions, self.folder

    @property
    def blockers(self) -> tuple[str, ...]:
        self.encode()
        checks = (
            (self.paused, 'game_not_paused'), (self.free_tile, 'target_not_free'),
            (self.supported, 'target_not_supported'),
            (self.sequence < MAX_U64 - 1, 'sequence_exhausted'),
            (self.next_building < MAX_I32, 'building_id_exhausted'),
            (self.next_job < MAX_I32, 'job_id_exhausted'),
            (self.building_count < MAX_BUILDINGS, 'building_capacity_exhausted'),
            (self.tiles[4].empty_floor, 'target_not_empty_floor'),
            (all(t.presence == 2 for t in self.tiles), 'context_not_fully_visible'),
            (all(t.presence != 2 or t.liquid == 0 for t in self.tiles), 'context_liquid'),
            (any(self.tiles[i].empty_floor for i in (1, 3, 5, 7)), 'no_empty_floor_neighbor'),
            (self.item.available(self.selection.kind), 'item_unavailable'),
        )
        return tuple(reason for ok, reason in checks if not ok)

    @property
    def eligible(self) -> bool:
        return not self.blockers

    def expected_after(self) -> Capture:
        require(self.eligible, 'placement requires an eligible capture')
        tiles = list(self.tiles)
        tiles[4] = replace(tiles[4], occupied=True, building=self.next_building)
        return replace(self, sequence=self.sequence + 1, next_building=self.next_building + 1,
                       next_job=self.next_job + 1, building_count=self.building_count + 1,
                       free_tile=False, tiles=tuple(tiles),
                       item=replace(self.item, in_job=True, jobs=(self.next_job,)))

    def view(self) -> dict:
        return {'capture_digest': self.witness.hex(), 'generation': self.generation,
                'sequence': self.sequence, 'tick': self.tick, 'site': self.site,
                'dimensions': list(self.dimensions), 'folder': self.folder,
                'next_building_id': self.next_building, 'next_job_id': self.next_job,
                'building_count': self.building_count, 'paused': self.paused,
                'free_tile': self.free_tile, 'supported': self.supported,
                'selection': self.selection.view(), 'tiles': [t.view() for t in self.tiles],
                'item': self.item.view(), 'eligible': self.eligible, 'blockers': list(self.blockers),
                'pathfinding_proved': False, 'structural_safety_proved': False}


def blockers(capture: Capture) -> tuple[str, ...]:
    require(type(capture) is Capture, 'invalid capture')
    return capture.blockers


def expected_after(capture: Capture) -> Capture:
    require(type(capture) is Capture, 'invalid capture')
    return capture.expected_after()


def plan_for(selection: Selection, witness: bytes) -> bytes:
    require(type(selection) is Selection and type(witness) is bytes and len(witness) == 32,
            'invalid plan input')
    return digest('dfmcp-build-plan/1', selection.encode() + witness)


def token_for(key: str, plan: bytes) -> bytes:
    require(type(plan) is bytes and len(plan) == 32, 'invalid plan digest')
    return digest('dfmcp-build-token/1', key_text(key) + plan)[:16]


@dataclass(frozen=True)
class Plan:
    key: str
    before: Capture

    def __post_init__(self):
        key_text(self.key)
        require(type(self.before) is Capture and self.before.eligible, 'ineligible furniture plan')

    @property
    def digest(self) -> bytes:
        return plan_for(self.before.selection, self.before.witness)

    @property
    def token(self) -> bytes:
        return token_for(self.key, self.digest)


@dataclass(frozen=True)
class Insertion:
    building: int
    job: int
    item: int
    kind: int
    pos: tuple[int, int, int]
    material: int
    material_index: int
    stage: int
    max_stage: int
    linked: bool
    construct_job: bool
    exact_item_link: bool
    suspended: bool

    def __post_init__(self):
        self.encode()

    def encode(self) -> bytes:
        for n in (self.building, self.job, self.item):
            integer(n, 0, MAX_I32 - 1)
        integer(self.kind, 1, 3)
        integer(self.max_stage, 1, 32)
        integer(self.stage, 0, self.max_stage)
        flags = bytes(flag(b) for b in (self.linked, self.construct_job, self.exact_item_link, self.suspended))
        return (b'DFMBI019' + u32(self.building) + u32(self.job) + u32(self.item) + bytes([self.kind])
                + coordinates(self.pos) + i32(self.material) + i32(self.material_index)
                + u32(self.stage) + u32(self.max_stage) + flags)

    @classmethod
    def decode(cls, raw: bytes) -> Insertion:
        r = Reader(raw, 53)
        require(r.take(8) == b'DFMBI019', 'wrong insertion profile')
        building, job, item = (r.number(4) for _ in range(3))
        kind, pos = r.number(1), tuple(r.number(4) for _ in range(3))
        material, material_index = r.number(4, signed=True), r.number(4, signed=True)
        stage, max_stage = r.number(4), r.number(4)
        value = cls(building, job, item, kind, pos, material, material_index, stage, max_stage,
                    *(r.flag() for _ in range(4)))
        r.finish()
        return value

    @property
    def raw(self) -> bytes:
        return self.encode()

    def matches(self, before: Capture) -> bool:
        self.encode()
        require(type(before) is Capture, 'invalid insertion source')
        return (self.building == before.next_building and self.job == before.next_job
                and self.item == before.selection.item and self.kind == before.selection.kind
                and self.pos == before.selection.target and self.material == before.item.material
                and self.material_index == before.item.material_index and self.stage == 0
                and self.linked and self.construct_job and self.exact_item_link and not self.suspended)

    def view(self) -> dict:
        return {'building_id': self.building, 'job_id': self.job, 'item_id': self.item,
                'kind': KINDS[self.kind], 'pos': list(self.pos), 'material': self.material,
                'material_index': self.material_index, 'stage': self.stage, 'max_stage': self.max_stage,
                'linked': self.linked, 'construct_job': self.construct_job,
                'exact_item_link': self.exact_item_link, 'suspended': self.suspended}


@dataclass(frozen=True)
class Record:
    plan: Plan
    phase: str
    reason: str
    after: Capture | None = None
    insertion: Insertion | None = None

    def __post_init__(self):
        self.encode()

    def encode(self) -> bytes:
        require(type(self.plan) is Plan, 'invalid record plan')
        self.plan.__post_init__()
        require(type(self.phase) is str and self.phase in PHASES
                and type(self.reason) is str and self.reason in REASONS, 'unknown phase/reason')
        allowed = {'prepared': ('none',), 'indeterminate': ('native_failure',), 'placed': ('none',),
                   'refused': ('stale', 'expired', 'source_changed'), 'cancelled': ('cancelled',)}
        require(self.reason in allowed[self.phase], 'impossible phase/reason')
        require((self.after is not None) == (self.phase == 'placed')
                and (self.insertion is not None) == (self.after is not None), 'impossible after evidence')
        raw = (b'DFMBR019' + key_text(self.plan.key) + field(self.plan.before.raw)
               + self.plan.digest + self.plan.token
               + bytes([PHASES.index(self.phase), REASONS.index(self.reason), self.attempted, self.after is not None]))
        if self.after is not None:
            require(type(self.after) is Capture and type(self.insertion) is Insertion, 'invalid after evidence')
            require(self.after.raw == self.plan.before.expected_after().raw, 'wrong complete after capture')
            require(self.insertion.matches(self.plan.before), 'insertion proof does not match plan')
            raw += field(self.after.raw) + field(self.insertion.raw)
        raw += digest('dfmcp-build-receipt/1', raw)
        require(len(raw) <= MAX_RECORD_BYTES, 'oversized effect record')
        return raw

    @classmethod
    def decode(cls, raw: bytes, expected: Plan | None = None) -> Record:
        require(expected is None or type(expected) is Plan, 'invalid expected plan')
        r = Reader(raw, MAX_RECORD_BYTES)
        require(r.take(8) == b'DFMBR019', 'wrong effect profile')
        key = text(r.bounded(128), 128)
        plan = Plan(key, Capture.decode(r.bounded(MAX_CAPTURE_BYTES)))
        require(expected is None or plan == expected, 'effect belongs to another plan')
        require(r.take(32) == plan.digest and r.take(16) == plan.token, 'effect plan/token mismatch')
        phase, reason = integer(r.number(1), 0, 4), integer(r.number(1), 0, 5)
        attempted, has_after = r.flag(), r.flag()
        after = Capture.decode(r.bounded(MAX_CAPTURE_BYTES)) if has_after else None
        insertion = Insertion.decode(r.bounded(53)) if has_after else None
        checksum_start = r.offset
        require(r.take(32) == digest('dfmcp-build-receipt/1', raw[:checksum_start]), 'effect integrity mismatch')
        r.finish()
        require(attempted == (phase in (1, 2)), 'impossible attempt flag')
        out = cls(plan, PHASES[phase], REASONS[reason], after, insertion)
        require(out.raw == raw, 'noncanonical effect record')
        return out

    @property
    def raw(self) -> bytes:
        return self.encode()

    @property
    def attempted(self) -> bool:
        return self.phase in ('indeterminate', 'placed')

    @property
    def terminal(self) -> bool:
        return self.phase in ('placed', 'refused', 'cancelled')

    @property
    def resolved(self) -> bool:
        return self.terminal

    def view(self) -> dict:
        return {'key': self.plan.key, 'plan_digest': self.plan.digest.hex(),
                'phase': self.phase, 'reason': self.reason, 'placement_attempted': self.attempted,
                'selection': self.plan.before.selection.view(), 'terminal': self.terminal,
                'resolved': self.resolved, 'receipt_digest': self.raw[-32:].hex(),
                'historical_job_registration_verified': self.phase == 'placed',
                'after': None if self.after is None else self.after.view(),
                'insertion': None if self.insertion is None else self.insertion.view(),
                'operator_attention_required': self.phase == 'indeterminate',
                'building_completed_proved': False, 'building_usable_proved': False,
                'current_pause_proved': False, 'checkpoint_proved': False,
                'retry_permitted': False, 'production_admitted': False}


def verify_record(raw: bytes, expected: Plan | None = None) -> Record:
    return Record.decode(raw, expected)


def successor(previous: Record, current: Record) -> None:
    """This native engine never revisits any outcome after a placement attempt."""
    require(type(previous) is Record and type(current) is Record, 'invalid retained records')
    require(previous.plan == current.plan, 'retained plan changed')
    previous.encode()
    current.encode()
    if previous.phase != 'prepared':
        require(previous.raw == current.raw, 'retained outcome changed')
