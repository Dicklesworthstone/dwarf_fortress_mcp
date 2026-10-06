"""Bind sampled construction completion to every original furnishing-batch step.

This captures original local custody and replays every child placement journal.
It neither changes that custody nor creates placement, retry, or effect authority.
Beads: df-dfhack-bridge-plane-c-pic.4 / df-dfhack-bridge-plane-c-pic.5.
"""
from __future__ import annotations

from dataclasses import InitVar, dataclass, field as derived, replace
import hashlib
from pathlib import Path
import struct

import build_placement_store as placements
from build_placement_rpc import Manifest
from build_placement_wire import MAX_TICK, field, integer, require, text_bytes, exact_hex
import construction_plan as condition
from construction_receipt import Cursor, Guard
import furniture_batch as furnishing
from furniture_plan import FurniturePlan
from furniture_handoff import Handoff
from room_furniture_handoff import RoomFurnitureHandoff

MAX_ORIGIN = 2 * 1024 * 1024
MAX_GOAL = MAX_ORIGIN + condition.MAX_GOAL + 16
MAX_SAMPLE = condition.MAX_SAMPLE
ORIGIN_MAGIC = b'DFMFCO01'
GOAL_MAGIC = b'DFMFCG01'
HANDOFF_ORIGIN_MAGIC = b'DFMFCO02'
HANDOFF_GOAL_MAGIC = b'DFMFCG02'
ROOM_ORIGIN_MAGIC = b'DFMFCO03'
ROOM_GOAL_MAGIC = b'DFMFCG03'
POLICY = 'dfmcp.original-furnishing-completion/1'
LinkedSample = condition.LinkedSample
Progress = condition.Progress
begin_read = condition.begin_read
cancel = condition.cancel


def _identity(value: tuple[int, int]) -> bytes:
    require(type(value) is tuple and len(value) == 2, 'invalid original custody identity')
    return struct.pack('>QQ', *(integer(number, 0, 2**64 - 1) for number in value))


def _blob(raw: bytes, maximum: int) -> bytes:
    require(type(raw) is bytes and 1 <= len(raw) <= maximum, 'invalid original evidence byte bound')
    return struct.pack('>I', len(raw)) + raw


def _read_blob(reader: Cursor, maximum: int) -> bytes:
    return reader.take(integer(reader.u32(maximum), 1, maximum))


def _path(value: str) -> bytes:
    raw = text_bytes(value, 4096)
    path = Path(value)
    require(path.is_absolute() and str(path) == value and '..' not in path.parts
            and len(path.parts) >= 2, 'original batch path must be absolute and canonical')
    return raw


def _pure_guard() -> None:
    """Pure constructors remain bounded without claiming filesystem authority."""


@dataclass(frozen=True)
class Child:
    name: str
    file_identity: tuple[int, int]
    raw: bytes

    def __post_init__(self) -> None:
        text_bytes(self.name, 256)
        _identity(self.file_identity)
        _blob(self.raw, placements.MAX_FILE)


@dataclass(frozen=True)
class Origin:
    batch_path: str
    manifest_raw: bytes
    manifest_identity: tuple[int, int]
    index_raw: bytes
    index_identity: tuple[int, int]
    children: tuple[Child, ...]
    guard: InitVar[Guard | None] = None
    batch_id: str = derived(init=False)
    plan: FurniturePlan = derived(init=False, repr=False, compare=False)
    source: Manifest = derived(init=False)
    address: tuple[str, int] = derived(init=False)
    root_identity: tuple[int, int] = derived(init=False)
    effects_identity: tuple[int, int] = derived(init=False)
    receipts: tuple[bytes, ...] = derived(init=False, repr=False)
    handoff: Handoff | None = derived(init=False, repr=False)
    room_handoff: RoomFurnitureHandoff | None = derived(init=False, repr=False)

    def __post_init__(self, guard: Guard | None) -> None:
        work = _pure_guard if guard is None else guard
        work()
        _path(self.batch_path)
        _identity(self.manifest_identity)
        _identity(self.index_identity)
        _blob(self.manifest_raw, furnishing.MAX_ROOM_DEFINITION)
        _blob(self.index_raw, furnishing.MAX_FILE)
        value = furnishing.unseal(self.manifest_raw)
        require(value.get('schema') in (furnishing.SCHEMA, furnishing.HANDOFF_SCHEMA, furnishing.ROOM_SCHEMA),
                'wrong original batch generation')
        with_handoff = value['schema'] == furnishing.HANDOFF_SCHEMA
        with_room = value['schema'] == furnishing.ROOM_SCHEMA
        placements.exact_object(value, {'schema', 'nonce', 'plan', 'source', 'endpoint', 'folder', 'site',
                                       'dimensions', 'first_tick', 'root_identity', 'effects_identity'}
                                | ({'handoff'} if with_handoff else {'room_handoff'} if with_room else set()))
        maximum = (furnishing.MAX_ROOM_DEFINITION if with_room else
                   furnishing.MAX_DEFINITION if with_handoff else furnishing.MAX_FILE)
        require(len(self.manifest_raw) <= maximum, 'original definition exceeds its fixed profile bound')
        room_handoff = (RoomFurnitureHandoff.from_json(value['room_handoff'], work) if with_room else None)
        handoff = (room_handoff.allocation if with_room else
                   Handoff.from_json(value['handoff']) if with_handoff else None)
        exact_hex(value['nonce'], 24)
        plan = FurniturePlan.from_json(value['plan'])
        require(plan.json() == value['plan'], 'original furniture plan is not normalized')
        source = placements.manifest_from(value['source'])
        address = furnishing.endpoint(value['endpoint'])
        text_bytes(value['folder'], 512)
        integer(value['site'], 0, 2147483647)
        integer(value['first_tick'], 0, MAX_TICK)
        require(type(value['dimensions']) is list, 'invalid original map dimensions')
        plan.check_dimensions(value['dimensions'])
        if room_handoff is not None:
            room_handoff.check_dimensions(value['dimensions'], work)
        if handoff is not None:
            handoff.validate_binding(value['endpoint'], value['folder'], value['site'],
                                     source.df_version, source.dfhack_version)
            require(handoff.plan() == plan and value['first_tick'] >= handoff.source.tick,
                    'original batch differs from retained request or inventory time')
        for name in ('root_identity', 'effects_identity'):
            require(type(value[name]) is list, 'invalid persisted original directory identity')
            _identity(tuple(value[name]))
        batch_id = furnishing.sha(self.manifest_raw)
        require(type(self.children) is tuple and len(self.children) == len(plan.ordered)
                and all(type(child) is Child for child in self.children),
                'every original furnishing step requires exactly one retained child')
        require(self.index_raw.startswith(furnishing.HEADER) and self.index_raw.endswith(b'\n'),
                'incomplete original batch index')
        lines = self.index_raw[len(furnishing.HEADER):].splitlines(keepends=True)
        require(len(lines) == len(plan.ordered), 'original steps are not completely registered')
        previous, receipts, last_after = furnishing.sha(furnishing.HEADER), [], None
        for step, child, line in zip(plan.ordered, self.children, lines):
            work()
            entry = furnishing.unseal(line)
            placements.exact_object(entry, {'batch_id', 'step', 'intent_sha256', 'file_identity', 'previous'})
            key = ('fr-' + batch_id if with_room else 'fb-' + value['nonce']) + '-' + step.name
            require(entry['batch_id'] == batch_id and entry['step'] == step.name
                    and entry['previous'] == previous and child.name == placements.filename(key),
                    'original index or child is not the complete canonical step order')
            exact_hex(entry['intent_sha256'], 32)
            require(type(entry['file_identity']) is list, 'invalid original indexed child identity')
            _identity(tuple(entry['file_identity']))
            require(entry['file_identity'] == list(child.file_identity)
                    and entry['intent_sha256'] == furnishing.sha(child.raw.splitlines(keepends=True)[0]),
                    'original indexed child identity or intent differs')
            state = placements.replay(child.raw)
            record, before = state.terminal, state.plan.before
            require(record is not None and record.phase == 'placed',
                    'completion requires every original registered Placed receipt')
            require(state.plan.key == key and before.selection == furnishing.selection(step)
                    and state.address == address, 'original child differs from the requested furnishing')
            require(state.manifest == state.terminal_manifest == source
                    and before.generation == source.generation
                    and before.folder == value['folder'] and before.site == value['site']
                    and list(before.dimensions) == value['dimensions']
                    and before.tick >= value['first_tick'], 'original child source or fortress differs')
            if handoff is not None:
                furnishing.validate_handoff_capture(handoff, before, source, value['endpoint'])
            if last_after is not None:
                require(before.tick >= last_after.tick and before.sequence >= last_after.sequence
                        and before.next_building >= last_after.next_building
                        and before.next_job >= last_after.next_job, 'original child history regressed')
            last_after = record.after
            receipts.append(record.raw)
            previous = furnishing.sha(line)
        for name, item in (('batch_id', batch_id), ('plan', plan), ('source', source), ('address', address),
                           ('root_identity', tuple(value['root_identity'])),
                           ('effects_identity', tuple(value['effects_identity'])), ('receipts', tuple(receipts)),
                           ('handoff', handoff), ('room_handoff', room_handoff)):
            object.__setattr__(self, name, item)
        require(len(self.encode()) <= MAX_ORIGIN, 'oversized original furnishing evidence')
        work()

    @classmethod
    def from_batch(cls, batch: furnishing.Batch, expected_id: str) -> Origin:
        require(type(batch) is furnishing.Batch, 'original furnishing owner required')
        exact_hex(expected_id, 32)
        require(batch.id == expected_id, 'original furnishing batch identity differs')
        audit = batch.audit()
        require(audit['status'] == 'all_placed' and len(batch.entries) == len(batch.plan.ordered)
                and all(row.get('registered') is True for row in audit['steps']),
                'all original furnishings must be placed and registered')
        children = []
        for step in batch.plan.ordered:
            batch.budget.remaining()
            name = placements.filename(batch.key(step))
            journal = batch.effects.journals[name]
            children.append(Child(name, tuple(furnishing.identity(journal.fd)), journal.raw))
        out = cls(batch.path, batch.files['batch.json'].raw, tuple(batch.files['batch.json'].identity),
                  batch.files['steps.jsonl'].raw, tuple(batch.files['steps.jsonl'].identity), tuple(children),
                  guard=batch.budget.remaining)
        require(out.batch_id == expected_id and out.root_identity == tuple(batch.root_identity)
                and out.effects_identity == batch.effects.identity, 'original batch directory binding differs')
        batch.check()
        return out

    def verify_batch(self, batch: furnishing.Batch) -> None:
        # The optional local stop marker is deliberately outside this origin.
        # Full immutable manifest, index, child bytes and all inode bindings stay.
        observed = Origin.from_batch(batch, self.batch_id)
        require(observed.encode() == self.encode(), 'original furnishing custody changed')

    def encode(self) -> bytes:
        magic = (ROOM_ORIGIN_MAGIC if self.room_handoff is not None else
                 ORIGIN_MAGIC if self.handoff is None else HANDOFF_ORIGIN_MAGIC)
        maximum = (furnishing.MAX_ROOM_DEFINITION if self.room_handoff is not None else
                   furnishing.MAX_FILE if self.handoff is None else furnishing.MAX_DEFINITION)
        raw = (magic + field(_path(self.batch_path)) + _identity(self.manifest_identity)
               + _blob(self.manifest_raw, maximum) + _identity(self.index_identity)
               + _blob(self.index_raw, furnishing.MAX_FILE) + bytes([len(self.children)]))
        for child in self.children:
            raw += field(text_bytes(child.name, 256)) + _identity(child.file_identity) + _blob(child.raw, placements.MAX_FILE)
        require(len(raw) <= MAX_ORIGIN, 'oversized original furnishing evidence')
        return raw

    @classmethod
    def decode(cls, raw: bytes, guard: Guard | None = None) -> Origin:
        r = Cursor(raw, MAX_ORIGIN)
        magic = r.take(8)
        require(magic in (ORIGIN_MAGIC, HANDOFF_ORIGIN_MAGIC, ROOM_ORIGIN_MAGIC),
                'wrong original furnishing evidence generation')
        path = r.string(4096)
        manifest_identity = (r.number(8), r.number(8))
        maximum = (furnishing.MAX_ROOM_DEFINITION if magic == ROOM_ORIGIN_MAGIC else
                   furnishing.MAX_FILE if magic == ORIGIN_MAGIC else furnishing.MAX_DEFINITION)
        manifest = _read_blob(r, maximum)
        index_identity = (r.number(8), r.number(8))
        index = _read_blob(r, furnishing.MAX_FILE)
        children = []
        for _ in range(integer(r.number(1), 1, condition.MAX_TARGETS)):
            if guard is not None:
                guard()
            children.append(Child(r.string(256), (r.number(8), r.number(8)), _read_blob(r, placements.MAX_FILE)))
        r.finish()
        out = cls(path, manifest, manifest_identity, index, index_identity, tuple(children), guard=guard)
        require(out.encode() == raw, 'noncanonical original furnishing evidence')
        return out

    @property
    def digest(self) -> str:
        domain = (b'dfmcp.furniture-completion-origin/3\0' if self.room_handoff is not None else
                  b'dfmcp.furniture-completion-origin/1\0' if self.handoff is None else b'dfmcp.furniture-completion-origin/2\0')
        return hashlib.sha256(domain + self.encode()).hexdigest()


@dataclass(frozen=True)
class Goal:
    origin: Origin
    condition: condition.Goal

    def __post_init__(self) -> None:
        require(type(self.origin) is Origin and type(self.condition) is condition.Goal,
                'completion requires one original batch and one construction condition')
        expected = condition.Goal(self.origin.receipts, self.deadline, self.interval, self.stable_samples,
                                  self.stable_span, self.max_gap, self.max_observations)
        require(expected.encode() == self.condition.encode(),
                'construction selection omits or substitutes original furnishing steps')

    @property
    def receipts(self):
        return self.condition.receipts

    @property
    def goals(self):
        return self.condition.goals

    @property
    def deadline(self):
        return self.condition.deadline

    @property
    def interval(self):
        return self.condition.interval

    @property
    def stable_samples(self):
        return self.condition.stable_samples

    @property
    def stable_span(self):
        return self.condition.stable_span

    @property
    def max_gap(self):
        return self.condition.max_gap

    @property
    def max_observations(self):
        return self.condition.max_observations

    def encode(self) -> bytes:
        magic = (ROOM_GOAL_MAGIC if self.origin.room_handoff is not None else
                 GOAL_MAGIC if self.origin.handoff is None else HANDOFF_GOAL_MAGIC)
        return magic + _blob(self.origin.encode(), MAX_ORIGIN) + _blob(self.condition.encode(), condition.MAX_GOAL)

    @classmethod
    def decode(cls, raw: bytes, guard: Guard | None = None) -> Goal:
        r = Cursor(raw, MAX_GOAL)
        require(r.take(8) in (GOAL_MAGIC, HANDOFF_GOAL_MAGIC, ROOM_GOAL_MAGIC),
                'wrong furnishing completion goal generation')
        origin = Origin.decode(_read_blob(r, MAX_ORIGIN), guard)
        goal = condition.Goal.decode(_read_blob(r, condition.MAX_GOAL))
        r.finish()
        out = cls(origin, goal)
        require(out.encode() == raw, 'noncanonical furnishing completion goal')
        return out

    @property
    def digest(self) -> str:
        domain = (b'dfmcp.furniture-completion-goal/3\0' if self.origin.room_handoff is not None else
                  b'dfmcp.furniture-completion-goal/1\0' if self.origin.handoff is None else b'dfmcp.furniture-completion-goal/2\0')
        return hashlib.sha256(domain + self.encode()).hexdigest()


def advance(state: Progress, goal: Goal, sample: LinkedSample, guard: Guard) -> Progress:
    guard()
    require(state.goal_digest == goal.digest, 'completion state belongs to another original furnishing goal')
    expected = goal.origin.source
    require(all((manifest.generation, manifest.df_version, manifest.dfhack_version)
                == (expected.generation, expected.df_version, expected.dfhack_version)
                for manifest in (sample.before, sample.after)),
            'sample differs from the original furnishing native source')
    handoff = goal.origin.handoff
    if handoff is not None:
        original = handoff.source
        require((sample.operations.generation, *sample.operations.software)
                == (original.generation, original.df_version, original.dfhack_version),
                'completion operations source differs from original allocation')
    reduced = condition.advance(replace(state, goal_digest=goal.condition.digest), goal.condition, sample, guard)
    if handoff is not None and (reduced.last_tick < handoff.source.tick
            or any(a < b for a, b in zip(reduced.horizons, handoff.source.horizons))):
        reduced = replace(reduced, phase='invalidated', reason='allocation_source_regressed',
                          reason_building=None, streak=0, first_tick=None, counted_tick=None)
    return replace(reduced, goal_digest=goal.digest)
