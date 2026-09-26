"""Whole-plan receipt-linked furniture conditions over one complete capture.

Every selected original placement is bracketed before and after the same
operations/1.4 capture. All targets must satisfy their unchanged single-receipt
conditions together; earlier per-building success is never latched. This is
monitoring evidence only, not placement authority or causal/continuous proof.
Beads: df-dfhack-bridge-plane-c-pic.4 and df-dfhack-bridge-plane-c-pic.5.
"""
from __future__ import annotations

from dataclasses import dataclass, field as dataclass_field, replace
import hashlib
import struct

from build_placement_wire import MAX_RECORD_BYTES, field, integer, require
from construction_receipt import (
    Assessment as SingleAssessment, Cursor, Goal as SingleGoal, Guard,
    MAX_CAPTURE, MAX_OBSERVATIONS, Manifest, Operations, assess, decode_operations,
)

MAX_TARGETS = 32
MAX_GOAL = MAX_TARGETS * (MAX_RECORD_BYTES + 2) + 128
MAX_SAMPLE = MAX_CAPTURE + 2 * MAX_TARGETS * (MAX_RECORD_BYTES + 2) + 4096
POLICY = 'dfmcp.receipt-construction-plan-condition/1'
TIMING = struct.Struct('>QIIQII')


def _receipts(values: tuple[bytes, ...]) -> bytes:
    require(type(values) is tuple and 1 <= len(values) <= MAX_TARGETS,
            'construction plan requires 1..32 receipt bytes')
    out = bytearray([len(values)])
    for raw in values:
        require(type(raw) is bytes and 0 < len(raw) <= MAX_RECORD_BYTES,
                'invalid bounded construction plan receipt')
        out += field(raw)
    return bytes(out)


def _read_receipts(reader: Cursor) -> tuple[bytes, ...]:
    return tuple(reader.bounded(MAX_RECORD_BYTES)
                 for _ in range(integer(reader.number(1), 1, MAX_TARGETS)))


@dataclass(frozen=True)
class Goal:
    receipts: tuple[bytes, ...]
    deadline: int
    interval: int = 1
    stable_samples: int = 2
    stable_span: int = 1
    max_gap: int = 1200
    max_observations: int = MAX_OBSERVATIONS
    goals: tuple[SingleGoal, ...] = dataclass_field(init=False, repr=False, compare=False)

    def __post_init__(self):
        _receipts(self.receipts)
        children = tuple(SingleGoal(raw, self.deadline, self.interval, self.stable_samples,
                                    self.stable_span, self.max_gap, self.max_observations)
                         for raw in self.receipts)
        records = tuple(child.record for child in children)
        source = records[0].plan.before.identity
        require(all(record.plan.before.identity == source for record in records),
                'construction plan receipts belong to different native fortress incarnations')
        for values, name in (
                ((record.plan.key for record in records), 'idempotency key'),
                ((record.insertion.building for record in records), 'building'),
                ((record.insertion.item for record in records), 'item'),
                ((record.insertion.job for record in records), 'construction job'),
                ((record.insertion.pos for record in records), 'target position')):
            require(len(set(values)) == len(records), 'duplicate construction plan ' + name)
        ordered = tuple(child for _, child in sorted(
            zip((record.insertion.building for record in records), children), key=lambda pair: pair[0]))
        object.__setattr__(self, 'goals', ordered)
        object.__setattr__(self, 'receipts', tuple(child.receipt for child in ordered))

    def encode(self) -> bytes:
        # Revalidate even a dataclass replaced by another in-process caller.
        self.__post_init__()
        raw = (b'DFMCPG01' + _receipts(self.receipts)
               + TIMING.pack(self.deadline, self.interval, self.stable_samples,
                             self.stable_span, self.max_gap, self.max_observations))
        require(len(raw) <= MAX_GOAL, 'oversized construction plan goal')
        return raw

    @classmethod
    def decode(cls, raw: bytes) -> Goal:
        r = Cursor(raw, MAX_GOAL)
        require(r.take(8) == b'DFMCPG01', 'wrong construction plan goal generation')
        out = cls(_read_receipts(r), r.number(8), r.u32(), r.u32(), r.number(8), r.u32(), r.u32())
        r.finish()
        require(out.encode() == raw, 'noncanonical construction plan goal')
        return out

    @property
    def digest(self) -> str:
        return hashlib.sha256(b'dfmcp.construction-plan-goal/1\0' + self.encode()).hexdigest()


@dataclass(frozen=True)
class LinkedSample:
    before: Manifest
    before_records: tuple[bytes, ...]
    operations: Manifest
    capture: bytes
    after: Manifest
    after_records: tuple[bytes, ...]

    def encode(self) -> bytes:
        require(type(self.capture) is bytes and 0 < len(self.capture) <= MAX_CAPTURE,
                'invalid complete plan capture size')
        require(all(type(manifest) is Manifest for manifest in (self.before, self.operations, self.after)),
                'invalid plan sample manifest')
        raw = (b'DFMCPS01' + self.before.encode() + _receipts(self.before_records)
               + self.operations.encode() + struct.pack('>I', len(self.capture)) + self.capture
               + self.after.encode() + _receipts(self.after_records))
        require(len(raw) <= MAX_SAMPLE, 'oversized construction plan sample')
        return raw

    @classmethod
    def decode(cls, raw: bytes) -> LinkedSample:
        r = Cursor(raw, MAX_SAMPLE)
        require(r.take(8) == b'DFMCPS01', 'wrong construction plan sample generation')
        before, receipts = Manifest.read(r), _read_receipts(r)
        operations = Manifest.read(r)
        capture = r.take(integer(r.u32(MAX_CAPTURE), 1, MAX_CAPTURE))
        out = cls(before, receipts, operations, capture, Manifest.read(r), _read_receipts(r))
        r.finish()
        require(out.encode() == raw, 'noncanonical construction plan sample')
        return out

    def validate(self, goal: Goal, guard: Guard) -> Operations:
        guard()
        self.encode()
        require(self.before_records == self.after_records == goal.receipts,
                'every original plan placement must be retained in canonical order')
        require(self.before == self.after
                and self.before.generation == goal.goals[0].record.plan.before.generation,
                'plan placement source changed across observation')
        require(self.before.software == self.operations.software,
                'plan native software families disagree')
        # Exactly one complete roster decode; child conditions share its immutable
        # jobs/buildings/items and cannot mix captures from different instants.
        return decode_operations(self.capture, guard)


@dataclass(frozen=True)
class Assessment:
    building_id: int
    item_id: int
    job_id: int
    key: str
    receipt_digest: str
    condition: SingleAssessment

    def view(self) -> dict:
        return {'building_id': self.building_id, 'item_id': self.item_id, 'job_id': self.job_id,
                'key': self.key, 'receipt_digest': self.receipt_digest, **self.condition.__dict__}


@dataclass(frozen=True)
class Progress:
    goal_digest: str
    phase: str = 'active'
    reason: str = 'not_sampled'
    reason_building: int | None = None
    observations: int = 0
    streak: int = 0
    first_tick: int | None = None
    counted_tick: int | None = None
    last_tick: int | None = None
    last_capture: str | None = None
    source: tuple | None = None
    horizons: tuple[int, int, int] | None = None
    reading: bool = False
    interruptions: int = 0
    assessments: tuple[Assessment, ...] = ()

    @property
    def terminal(self) -> bool:
        return self.phase in ('satisfied', 'failed', 'invalidated', 'expired', 'cancelled')

    def view(self) -> dict:
        return {'policy': POLICY, 'goal_digest': self.goal_digest, 'phase': self.phase, 'reason': self.reason,
                'reason_building': self.reason_building, 'observations': self.observations,
                'streak': self.streak, 'first_tick': self.first_tick,
                'last_counted_tick': self.counted_tick, 'last_tick': self.last_tick,
                'capture_sha256': self.last_capture, 'interrupted_reads': self.interruptions,
                'read_outcome_unknown': self.reading, 'terminal': self.terminal,
                'sampled_condition_satisfied': self.phase == 'satisfied',
                'selection_count': len(self.assessments),
                'condition_met_count': sum(row.condition.status == 'condition_met' for row in self.assessments),
                'assessments_complete': bool(self.assessments),
                'assessments': [row.view() for row in self.assessments],
                'placement_effect_discharged': False, 'current_usability_proven': False,
                'continuous_stability_proven': False, 'retry_placement_permitted': False}


def begin_read(state: Progress) -> Progress:
    require(not state.terminal, 'terminal construction plan cannot acquire observations')
    if state.reading:
        state = replace(state, streak=0, first_tick=None, counted_tick=None,
                        interruptions=state.interruptions + 1, reason='interrupted_read', reason_building=None)
    return replace(state, reading=True)


def cancel(state: Progress) -> Progress:
    if state.terminal:
        return state
    return replace(state, phase='cancelled', reason='monitor_cancelled_only', reason_building=None,
                   reading=False, streak=0, first_tick=None, counted_tick=None)


def advance(state: Progress, goal: Goal, sample: LinkedSample, guard: Guard) -> Progress:
    guard()
    require(state.goal_digest == goal.digest and state.reading and not state.terminal,
            'invalid construction plan transition')
    observed = sample.validate(goal, guard)
    findings = []
    for child in goal.goals:
        guard()
        record = child.record
        p = record.insertion
        findings.append(Assessment(p.building, p.item, p.job, record.plan.key,
                                   child.receipt[-32:].hex(), assess(child, observed, guard)))
    guard()
    source = (sample.before.generation, sample.operations.generation, *sample.operations.software)
    current = replace(state, reading=False, observations=state.observations + 1,
                      last_tick=observed.tick, last_capture=observed.digest, source=source,
                      horizons=observed.horizons, assessments=tuple(findings), reason_building=None)

    def stop(phase: str, reason: str, building: int | None = None) -> Progress:
        return replace(current, phase=phase, reason=reason, reason_building=building,
                       streak=0, first_tick=None, counted_tick=None)

    invalid = {'world_identity_mismatch', 'source_regressed', 'building_missing', 'item_missing',
               'building_identity_mismatch', 'item_identity_mismatch', 'original_job_identity_mismatch'}
    # Inspect the COMPLETE selection before deciding. The earliest canonical
    # building supplies a bounded primary reason; every row retains its evidence.
    for row in findings:
        if row.condition.status in invalid:
            return stop('invalidated', row.condition.status, row.building_id)
    if state.source is not None and source != state.source:
        return stop('invalidated', 'native_source_changed')
    if state.horizons is not None and any(a < b for a, b in zip(observed.horizons, state.horizons)):
        return stop('invalidated', 'native_horizon_regressed')
    if state.last_tick is not None and observed.tick < state.last_tick:
        return stop('invalidated', 'game_clock_regressed')
    prior = {row.building_id: row.condition for row in state.assessments}
    for row in findings:
        old, condition = prior.get(row.building_id), row.condition
        if old is not None and old.building_type is not None and condition.building_type != old.building_type:
            return stop('invalidated', 'native_building_type_changed', row.building_id)
        if old is not None and old.stage is not None and condition.stage < old.stage:
            return stop('invalidated', 'construction_stage_regressed', row.building_id)
    for row in findings:
        if row.condition.status == 'removal_pending':
            return stop('failed', row.condition.status, row.building_id)
    if observed.tick >= goal.deadline:
        return stop('expired', 'game_deadline_reached')
    pending = next((row for row in findings if row.condition.status != 'condition_met'), None)
    if pending is not None:
        current = stop('active', pending.condition.status, pending.building_id)
    else:
        if state.last_tick is not None and (observed.tick - state.last_tick > goal.max_gap
                or (observed.tick == state.last_tick and observed.digest != state.last_capture)):
            current = stop('active', 'observation_gap_or_same_tick_change')
        if state.last_tick != observed.tick and (current.counted_tick is None
                or observed.tick - current.counted_tick >= goal.interval):
            current = replace(current, phase='candidate', reason='awaiting_stability',
                              first_tick=observed.tick if current.first_tick is None else current.first_tick,
                              counted_tick=observed.tick, streak=current.streak + 1)
        if (current.streak >= goal.stable_samples
                and observed.tick - current.first_tick >= goal.stable_span
                and current.counted_tick == observed.tick):
            return replace(current, phase='satisfied', reason='receipt_linked_plan_sampled_condition')
    if current.observations >= goal.max_observations:
        return replace(current, phase='expired', reason='sample_budget_exhausted', reason_building=None,
                       streak=0, first_tick=None, counted_tick=None)
    return current
