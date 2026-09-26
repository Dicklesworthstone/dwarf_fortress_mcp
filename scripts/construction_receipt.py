"""Receipt-linked construction conditions over unchanged operations/1.4 bytes.

Pure, bounded replay primitives. A valid byte stream is evidence, not authority
or proof of having contacted a game. The query-only transport brackets each fresh
capture with the original furniture/1.19 record on one native connection.
Beads: df-dfhack-bridge-plane-c-pic.4 and df-dfhack-bridge-plane-c-pic.5.
"""
from __future__ import annotations

from dataclasses import dataclass, replace
import hashlib
import struct
from types import MappingProxyType
from typing import Callable, Mapping

from build_placement_wire import (
    MAX_I32, MAX_TICK, MAX_U64, Reader, Record, canonical, field, integer,
    require, text, text_bytes,
)

MAX_CAPTURE = 16 * 1024 * 1024
MAX_SAMPLE = MAX_CAPTURE + 16384
MAX_OBSERVATIONS = 512
POLICY = 'dfmcp.receipt-construction-condition/1'
Guard = Callable[[], None]


class Cursor(Reader):
    def u32(self, maximum: int = MAX_I32) -> int:
        return integer(self.number(4), 0, maximum)

    def i32(self, minimum: int = -(2**31)) -> int:
        return integer(self.number(4, signed=True), minimum, MAX_I32)

    def string(self, maximum: int = 128, empty: bool = False) -> str:
        size = integer(self.number(2), 0 if empty else 1, maximum)
        return text(self.take(size), maximum) if size else ''

    def reference(self) -> int | None:
        return self.u32() if self.flag() else None


@dataclass(frozen=True)
class Job:
    id: int
    native_type: int
    kind: str
    reaction: str
    suspended: bool
    repeat: bool
    position: tuple[int, ...]
    worker: int | None
    holder: int | None
    timer: int
    attachments: int
    filters: int


@dataclass(frozen=True)
class Building:
    id: int
    native_type: int
    kind: str
    bounds: tuple[int, ...]
    stage: int
    max_stage: int


@dataclass(frozen=True)
class Item:
    id: int
    native_type: int
    kind: str
    subtype: int
    material: int
    material_index: int
    stack: int
    position: tuple[int, ...]
    flags: int
    container: int | None
    holder: int | None


@dataclass(frozen=True)
class Operations:
    """Decoded immutable records; consumers obtain these only by full decoding."""
    raw: bytes
    tick: int
    paused: bool
    site: int
    folder: str
    horizons: tuple[int, int, int]
    jobs: Mapping[int, Job]
    buildings: Mapping[int, Building]
    items: Mapping[int, Item]
    attachments: tuple[tuple[int, int, int, int], ...]

    @property
    def digest(self) -> str:
        return hashlib.sha256(self.raw).hexdigest()


def decode_operations(raw: bytes, guard: Guard) -> Operations:
    """Validate the COMPLETE native roster before using absence or count facts.

    Bounds and field order match dfmcp_operations_v1_4.cpp, not a JSON projection.
    The injected guard is checked during every bounded traversal, including long
    container chains. Unknown enum keys remain data, never guessed known kinds.
    """
    guard()
    r = Cursor(raw, MAX_CAPTURE)
    require(r.take(8) == b'DFMO1400', 'wrong operations profile')
    jobs_raw = r.take(integer(r.u32(2 * 1024 * 1024), 1, 2 * 1024 * 1024))
    j = Cursor(jobs_raw, 2 * 1024 * 1024)
    require(j.take(8) == b'DFMJ1200', 'wrong jobs component')
    year, tick = j.u32(2**32 - 1), j.u32(403199)
    paused, site, next_job, folder = j.flag(), j.i32(0), j.u32(), j.string(512)

    def roster(cursor: Cursor, maximum: int, horizon: int, parse: Callable) -> dict:
        count = cursor.u32(maximum)
        values, types, keys = {}, {}, {}
        previous = -1
        for _ in range(count):
            guard()
            identity, native_type, kind = cursor.u32(), cursor.i32(0), cursor.string()
            require(previous < identity < horizon, 'unordered, duplicate or out-of-horizon identity')
            require(types.get(native_type, kind) == kind and keys.get(kind, native_type) == native_type,
                    'inconsistent native enum identity')
            types[native_type], keys[kind], previous = kind, native_type, identity
            values[identity] = parse(cursor, identity, native_type, kind)
        return values

    def job(c: Cursor, identity: int, native_type: int, kind: str) -> Job:
        return Job(identity, native_type, kind, c.string(empty=True), c.flag(), c.flag(),
                   tuple(c.i32() for _ in range(3)), c.reference(), c.reference(), c.i32(-1),
                   c.u32(65536), c.u32(4096))

    jobs = roster(j, 4096, next_job, job)
    j.finish()
    next_building, next_item = r.u32(), r.u32()

    def building(c: Cursor, identity: int, native_type: int, kind: str) -> Building:
        bounds = tuple(c.i32() for _ in range(5))
        stage, maximum = c.i32(0), c.i32(0)
        require(bounds[0] <= bounds[2] and bounds[1] <= bounds[3] and stage <= maximum,
                'invalid building bounds or stage')
        return Building(identity, native_type, kind, bounds, stage, maximum)

    buildings = roster(r, 4096, next_building, building)

    def item(c: Cursor, identity: int, native_type: int, kind: str) -> Item:
        return Item(identity, native_type, kind, c.i32(-1), c.i32(-1), c.i32(-1), c.u32(),
                    tuple(c.i32() for _ in range(3)), c.u32(511), c.reference(), c.reference())

    items = roster(r, 65536, next_item, item)
    for value in jobs.values():
        guard()
        require(value.holder is None or value.holder in buildings, 'missing job holder endpoint')
    for value in items.values():
        guard()
        require(value.container is None or value.container in items, 'missing item container endpoint')
        require(value.holder is None or value.holder in buildings, 'missing item holder endpoint')
    # Iterative linear-time colors, not recursive or quadratic parent walks.
    colors = {}
    for identity in items:
        chain = []
        while identity is not None and colors.get(identity, 0) != 2:
            guard()
            require(colors.get(identity, 0) != 1, 'cyclic item containment')
            colors[identity] = 1
            chain.append(identity)
            identity = items[identity].container
        for identity in chain:
            colors[identity] = 2
    attachments, counts, previous = [], {}, None
    for _ in range(r.u32(65536)):
        guard()
        row = (r.i32(0), r.i32(0), r.i32(0), r.i32(-1))
        jid, iid, _, index = row
        require(previous is None or previous < row, 'duplicate or unordered attachment')
        require(jid in jobs and iid in items, 'missing job/item attachment endpoint')
        require(index == -1 or index < jobs[jid].filters, 'attachment filter outside job')
        counts[jid] = counts.get(jid, 0) + 1
        attachments.append(row)
        previous = row
    r.finish()
    require(all(counts.get(identity, 0) == value.attachments for identity, value in jobs.items()),
            'declared job attachment count differs from complete relation')
    guard()
    return Operations(raw, year * 403200 + tick, paused, site, folder,
                      (next_job, next_building, next_item), MappingProxyType(jobs),
                      MappingProxyType(buildings), MappingProxyType(items), tuple(attachments))


@dataclass(frozen=True)
class Manifest:
    generation: int
    df_version: str
    dfhack_version: str

    def encode(self) -> bytes:
        return (struct.pack('>Q', integer(self.generation, 1, MAX_U64 - 1))
                + field(text_bytes(self.df_version, 128)) + field(text_bytes(self.dfhack_version, 128)))

    @classmethod
    def read(cls, r: Cursor) -> Manifest:
        out = cls(r.number(8), r.string(), r.string())
        out.encode()
        return out

    @property
    def software(self) -> tuple[str, str]:
        return self.df_version, self.dfhack_version


@dataclass(frozen=True)
class LinkedSample:
    """Native facts acquired by the fixed same-connection query bracket.

    Serialized evidence is not signed and cannot independently attest a network
    connection. Importing these bytes never grants a game-effect capability.
    """
    before: Manifest
    before_record: bytes
    operations: Manifest
    capture: bytes
    after: Manifest
    after_record: bytes

    def encode(self) -> bytes:
        require(type(self.capture) is bytes and 0 < len(self.capture) <= MAX_CAPTURE, 'invalid capture size')
        for record in (self.before_record, self.after_record):
            require(type(record) is bytes and 0 < len(record) <= 6144, 'invalid receipt size')
        out = (b'DFMCSP01' + self.before.encode() + field(self.before_record)
               + self.operations.encode() + struct.pack('>I', len(self.capture)) + self.capture
               + self.after.encode() + field(self.after_record))
        require(len(out) <= MAX_SAMPLE, 'oversized linked sample')
        return out

    @classmethod
    def decode(cls, raw: bytes) -> LinkedSample:
        r = Cursor(raw, MAX_SAMPLE)
        require(r.take(8) == b'DFMCSP01', 'wrong linked-sample generation')
        before, receipt = Manifest.read(r), r.bounded(6144)
        ops = Manifest.read(r)
        capture = r.take(integer(r.u32(MAX_CAPTURE), 1, MAX_CAPTURE))
        out = cls(before, receipt, ops, capture, Manifest.read(r), r.bounded(6144))
        r.finish()
        require(out.encode() == raw, 'noncanonical linked sample')
        return out

    def validate(self, goal: Goal, guard: Guard) -> Operations:
        self.encode()
        require(self.before_record == self.after_record == goal.receipt, 'original placement record not retained')
        require(self.before == self.after and self.before.generation == goal.record.plan.before.generation,
                'placement source changed across observation')
        require(self.before.software == self.operations.software, 'native software families disagree')
        return decode_operations(self.capture, guard)


@dataclass(frozen=True)
class Goal:
    receipt: bytes
    deadline: int
    interval: int = 1
    stable_samples: int = 2
    stable_span: int = 1
    max_gap: int = 1200
    max_observations: int = 512

    def __post_init__(self):
        record = self.record
        require(record.phase == 'placed', 'only an exact placed receipt can select construction work')
        integer(self.deadline, record.plan.before.tick + 1, MAX_TICK)
        integer(self.interval, 1, 403200)
        integer(self.stable_samples, 2, 64)
        integer(self.stable_span, 1, 4032000)
        integer(self.max_gap, self.interval, 4032000)
        integer(self.max_observations, self.stable_samples, MAX_OBSERVATIONS)
        require(max(self.stable_span, (self.stable_samples - 1) * self.interval)
                < self.deadline - record.plan.before.tick, 'goal cannot fit before fixed deadline')

    @property
    def record(self) -> Record:
        return Record.decode(self.receipt)

    def encode(self) -> bytes:
        self.__post_init__()
        return (b'DFMCGO01' + field(self.receipt)
                + struct.pack('>QIIQII', self.deadline, self.interval, self.stable_samples,
                              self.stable_span, self.max_gap, self.max_observations))

    @classmethod
    def decode(cls, raw: bytes) -> Goal:
        r = Cursor(raw, 8192)
        require(r.take(8) == b'DFMCGO01', 'wrong construction goal')
        out = cls(r.bounded(6144), r.number(8), r.u32(), r.u32(), r.number(8), r.u32(), r.u32())
        r.finish()
        require(out.encode() == raw, 'noncanonical construction goal')
        return out

    @property
    def digest(self) -> str:
        return hashlib.sha256(b'dfmcp.construction-goal/1\0' + self.encode()).hexdigest()


@dataclass(frozen=True)
class Assessment:
    status: str
    stage: int | None
    max_stage: int
    building_type: int | None
    construction_jobs: int = 0
    removal_jobs: int = 0
    suspended_jobs: int = 0
    item_job_links: int = 0


def assess(goal: Goal, observed: Operations, guard: Guard) -> Assessment:
    """No job disappearance, count change or matching ID alone proves success."""
    guard()
    record = goal.record
    p, before = record.insertion, record.plan.before
    building, item = observed.buildings.get(p.building), observed.items.get(p.item)
    outcome = Assessment('pending', None if building is None else building.stage,
                         p.max_stage, None if building is None else building.native_type)
    if (observed.site, observed.folder) != (before.site, before.folder):
        return replace(outcome, status='world_identity_mismatch')
    if observed.tick < before.tick or observed.horizons[0] < record.after.next_job or observed.horizons[1] < record.after.next_building:
        return replace(outcome, status='source_regressed')
    if building is None:
        return replace(outcome, status='building_missing')
    x, y, z = p.pos
    if (building.kind != ('', 'Bed', 'Chair', 'Table')[p.kind]
            or building.bounds != (x, y, x, y, z) or building.max_stage != p.max_stage):
        return replace(outcome, status='building_identity_mismatch')
    if item is None:
        return replace(outcome, status='item_missing')
    expected = before.item
    if ((item.native_type, item.kind, item.subtype, item.material, item.material_index)
            != (expected.native_type, ('', 'BED', 'CHAIR', 'TABLE')[p.kind], expected.subtype,
                expected.material, expected.material_index)):
        return replace(outcome, status='item_identity_mismatch')
    held = []
    for job in observed.jobs.values():
        guard()
        if job.holder == p.building:
            held.append(job)
    construction = [j for j in held if j.kind == 'ConstructBuilding']
    removal = [j for j in held if j.kind == 'DestroyBuilding']
    linked_jobs = set()
    for jid, iid, _, _ in observed.attachments:
        guard()
        if iid == p.item:
            linked_jobs.add(jid)
    outcome = replace(outcome, construction_jobs=len(construction), removal_jobs=len(removal),
                      suspended_jobs=sum(j.suspended for j in construction), item_job_links=len(linked_jobs))
    if removal:
        return replace(outcome, status='removal_pending')
    original_job = observed.jobs.get(p.job)
    if original_job is not None and (original_job.kind != 'ConstructBuilding' or original_job.holder != p.building):
        return replace(outcome, status='original_job_identity_mismatch')
    if construction:
        return replace(outcome, status='suspended' if all(j.suspended for j in construction) else 'pending')
    if building.stage != p.max_stage:
        return replace(outcome, status='no_construction_job')
    # Same installed-item policy as the existing construction query, plus the
    # original receipt's type/material identity and a singleton furniture stack.
    forbidden = (1 << 1) | (1 << 3) | (1 << 6) | (1 << 7)
    if (item.holder != p.building or item.container is not None or not item.flags & (1 << 8)
            or item.flags & forbidden or item.stack != 1 or linked_jobs):
        return replace(outcome, status='item_unverified')
    return replace(outcome, status='condition_met')


@dataclass(frozen=True)
class Progress:
    goal_digest: str
    phase: str = 'active'
    reason: str = 'not_sampled'
    observations: int = 0
    streak: int = 0
    first_tick: int | None = None
    counted_tick: int | None = None
    last_tick: int | None = None
    last_capture: str | None = None
    source: tuple | None = None
    horizons: tuple[int, int, int] | None = None
    building_type: int | None = None
    last_stage: int | None = None
    reading: bool = False
    interruptions: int = 0
    assessment: Assessment | None = None

    @property
    def terminal(self) -> bool:
        return self.phase in ('satisfied', 'failed', 'invalidated', 'expired', 'cancelled')

    def view(self) -> dict:
        return {'policy': POLICY, 'goal_digest': self.goal_digest, 'phase': self.phase, 'reason': self.reason,
                'observations': self.observations, 'streak': self.streak, 'first_tick': self.first_tick,
                'last_counted_tick': self.counted_tick, 'last_tick': self.last_tick,
                'capture_sha256': self.last_capture, 'interrupted_reads': self.interruptions,
                'read_outcome_unknown': self.reading, 'terminal': self.terminal,
                'sampled_condition_satisfied': self.phase == 'satisfied',
                'placement_effect_discharged': False, 'current_usability_proven': False,
                'continuous_stability_proven': False, 'retry_placement_permitted': False,
                'assessment': None if self.assessment is None else dict(self.assessment.__dict__)}


def begin_read(state: Progress) -> Progress:
    require(not state.terminal, 'terminal monitor cannot acquire observations')
    # A complete read-start left by a crashed owner is an unknown interval.
    if state.reading:
        state = replace(state, streak=0, first_tick=None, counted_tick=None,
                        interruptions=state.interruptions + 1, reason='interrupted_read')
    return replace(state, reading=True)


def cancel(state: Progress) -> Progress:
    if state.terminal:
        return state
    return replace(state, phase='cancelled', reason='monitor_cancelled_only', reading=False,
                   streak=0, first_tick=None, counted_tick=None)


def advance(state: Progress, goal: Goal, sample: LinkedSample, guard: Guard) -> Progress:
    require(state.goal_digest == goal.digest and state.reading and not state.terminal, 'invalid monitor transition')
    observed = sample.validate(goal, guard)
    finding = assess(goal, observed, guard)
    source = (sample.before.generation, sample.operations.generation, *sample.operations.software)
    current = replace(state, reading=False, observations=state.observations + 1,
                      last_tick=observed.tick, last_capture=observed.digest, source=source,
                      horizons=observed.horizons, last_stage=finding.stage, assessment=finding,
                      building_type=finding.building_type)

    def stop(phase: str, reason: str) -> Progress:
        return replace(current, phase=phase, reason=reason, streak=0, first_tick=None, counted_tick=None)

    invalid = {'world_identity_mismatch', 'source_regressed', 'building_missing', 'item_missing',
               'building_identity_mismatch', 'item_identity_mismatch', 'original_job_identity_mismatch'}
    if finding.status in invalid:
        return stop('invalidated', finding.status)
    if state.source is not None and source != state.source:
        return stop('invalidated', 'native_source_changed')
    if state.horizons is not None and any(a < b for a, b in zip(observed.horizons, state.horizons)):
        return stop('invalidated', 'native_horizon_regressed')
    if state.building_type is not None and finding.building_type != state.building_type:
        return stop('invalidated', 'native_building_type_changed')
    if state.last_tick is not None and observed.tick < state.last_tick:
        return stop('invalidated', 'game_clock_regressed')
    if finding.status == 'removal_pending':
        return stop('failed', finding.status)
    if state.last_stage is not None and finding.stage < state.last_stage:
        return stop('invalidated', 'construction_stage_regressed')
    if observed.tick >= goal.deadline:
        return stop('expired', 'game_deadline_reached')
    if finding.status != 'condition_met':
        current = stop('active', finding.status)
    else:
        if state.last_tick is not None and (observed.tick - state.last_tick > goal.max_gap
                or (observed.tick == state.last_tick and observed.digest != state.last_capture)):
            current = stop('active', 'observation_gap_or_same_tick_change')
        if state.last_tick != observed.tick and (current.counted_tick is None
                or observed.tick - current.counted_tick >= goal.interval):
            current = replace(current, phase='candidate', reason='awaiting_stability',
                              first_tick=observed.tick if current.first_tick is None else current.first_tick,
                              counted_tick=observed.tick, streak=current.streak + 1)
        if current.streak >= goal.stable_samples and observed.tick - current.first_tick >= goal.stable_span:
            # The final observation must itself count; an early poll cannot close
            # the goal just because the span elapsed between eligible samples.
            if current.counted_tick == observed.tick:
                return replace(current, phase='satisfied', reason='receipt_linked_sampled_condition')
    if current.observations >= goal.max_observations:
        return replace(current, phase='expired', reason='sample_budget_exhausted',
                       streak=0, first_tick=None, counted_tick=None)
    return current
