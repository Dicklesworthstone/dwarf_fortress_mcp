"""Durable joint room evidence bound to every original furnishing-batch step.

One fixed journal retains the original room, complete placement custody, and
raw map/operations samples. Replay recomputes their ONE shared stability streak;
it grants neither a native read permit nor any game-effect or retry authority.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5; WP-05/07/10.
"""
from __future__ import annotations

from dataclasses import InitVar, dataclass, field as derived, replace
import hashlib
import struct
from typing import Callable

from build_placement_wire import Rejected, integer, require, text
from construction_monitor_rpc import Budget, endpoint
from construction_monitor_store import HEADER, KINDS, _Custody
from construction_receipt import Cursor, Guard
import construction_plan as construction
from furniture_batch import Batch
from furniture_completion import Origin, MAX_ORIGIN
import room_readiness as readiness

GOAL_MAGIC = b'DFMRDC01'
GOAL_DOMAIN = b'dfmcp.original-room-readiness-goal/1\0'
MAX_GOAL = MAX_ORIGIN + construction.MAX_GOAL + 16
MAGIC = b'DFMRDJ01'
DOMAIN = b'dfmcp.room-readiness-journal/1\0'
MAX_FILE = 128 * 1024 * 1024
MAX_FRAMES = 1030
# The initial goal includes its bounded endpoint; samples include BOTH complete
# map captures and the complete receipt-linked operations capture.
MAX_BODY = max(MAX_GOAL + 130, readiness.MAX_SAMPLE) + 1


def _idle() -> None:
    """A pure codec does not acquire local or native authority."""


def _blob(raw: bytes, maximum: int) -> bytes:
    require(type(raw) is bytes and 1 <= len(raw) <= maximum,
            'original room readiness evidence exceeds its bound')
    return struct.pack('>I', len(raw)) + raw


def _read_blob(reader: Cursor, maximum: int) -> bytes:
    return reader.take(integer(reader.u32(maximum), 1, maximum))


@dataclass(frozen=True)
class Goal:
    """An original room batch plus its immutable whole-selection timing policy."""

    origin: Origin
    condition: construction.Goal
    guard: InitVar[Guard | None] = None
    readiness_goal: readiness.Goal = derived(init=False, repr=False, compare=False)

    def __post_init__(self, guard: Guard | None) -> None:
        work = _idle if guard is None else guard
        work()
        require(type(self.origin) is Origin and type(self.condition) is construction.Goal,
                'room readiness requires complete original batch and construction condition')
        origin = Origin.decode(self.origin.encode(), work)
        condition = construction.Goal.decode(self.condition.encode())
        require(origin.room_handoff is not None,
                'room readiness requires the original complete room-backed batch')
        expected = construction.Goal(origin.receipts, condition.deadline, condition.interval,
            condition.stable_samples, condition.stable_span, condition.max_gap,
            condition.max_observations)
        require(condition.encode() == expected.encode(),
                'room readiness cannot omit or substitute original placement receipts')
        joint = readiness.Goal(origin.room_handoff, condition, checkpoint=work)
        object.__setattr__(self, 'origin', origin)
        object.__setattr__(self, 'condition', condition)
        object.__setattr__(self, 'readiness_goal', joint)
        work()

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
        return (GOAL_MAGIC + _blob(self.origin.encode(), MAX_ORIGIN)
                + _blob(self.condition.encode(), construction.MAX_GOAL))

    @classmethod
    def decode(cls, raw: bytes, guard: Guard | None = None) -> Goal:
        work = _idle if guard is None else guard
        work()
        reader = Cursor(raw, MAX_GOAL)
        require(reader.take(8) == GOAL_MAGIC, 'wrong original room readiness goal generation')
        origin = Origin.decode(_read_blob(reader, MAX_ORIGIN), work)
        condition = construction.Goal.decode(_read_blob(reader, construction.MAX_GOAL))
        reader.finish()
        result = cls(origin, condition, guard=work)
        require(result.encode() == raw, 'noncanonical original room readiness goal')
        work()
        return result

    @property
    def digest(self) -> str:
        return hashlib.sha256(GOAL_DOMAIN + self.encode()).hexdigest()


@dataclass(frozen=True)
class State:
    goal: Goal
    address: tuple[str, int]
    progress: readiness.Progress
    frames: int = 1
    tail: bytes = bytes(32)


def advance(state: readiness.Progress, goal: Goal, sample: readiness.Sample,
            guard: Guard) -> readiness.Progress:
    """Bind the existing joint reducer to the full original custody identity."""
    guard()
    require(type(goal) is Goal and type(state) is readiness.Progress
            and type(sample) is readiness.Sample and state.goal_digest == goal.digest,
            'room readiness state belongs to another original batch goal')
    expected = goal.origin.source
    require(all((manifest.generation, *manifest.software)
                == (expected.generation, expected.df_version, expected.dfhack_version)
                for manifest in (sample.furnishings.before, sample.furnishings.after)),
            'joint sample differs from original furnishing source')
    reduced = readiness.advance(replace(state, goal_digest=goal.readiness_goal.digest),
                                goal.readiness_goal, sample, guard)
    return replace(reduced, goal_digest=goal.digest)


def transition(state: State | None, kind: str, payload: bytes, budget: Budget) -> State:
    budget.work()
    require(type(payload) is bytes, 'room readiness journal payload must be bytes')
    if state is None:
        require(kind == 'goal' and len(payload) >= 2,
                'room readiness journal must begin with the complete original goal')
        size = int.from_bytes(payload[:2], 'big')
        require(1 <= size <= 128 and len(payload) > size + 2,
                'invalid room readiness endpoint framing')
        address = endpoint(text(payload[2:2 + size], 128))
        goal = Goal.decode(payload[2 + size:], budget.work)
        require(address == goal.origin.address,
                'room readiness endpoint differs from the original batch')
        return State(goal, address, readiness.Progress(goal.digest))
    require(not state.progress.terminal, 'record follows immutable terminal room evidence')
    if kind == 'read_started':
        require(not payload, 'room readiness read-start contains unexpected data')
        progress = readiness.begin_read(state.progress)
    elif kind == 'sample':
        progress = advance(state.progress, state.goal, readiness.Sample.decode(payload), budget.work)
    elif kind == 'cancel':
        require(not payload, 'room readiness cancellation contains unexpected data')
        progress = readiness.cancel(state.progress)
    else:
        raise Rejected('unknown or repeated room readiness goal')
    return replace(state, progress=progress)


def frame(state: State | None, kind: str, payload: bytes) -> bytes:
    require(kind in KINDS and type(payload) is bytes and len(payload) < MAX_BODY,
            'invalid bounded room readiness frame')
    sequence = 0 if state is None else state.frames
    require(sequence < MAX_FRAMES, 'room readiness frame allowance exhausted')
    body = bytes([KINDS[kind]]) + payload
    header = HEADER.pack(len(body), sequence, bytes(32) if state is None else state.tail)
    return header + body + hashlib.sha256(DOMAIN + header + body).digest()


def replay_reader(read: Callable[[int, int], bytes], size: int, budget: Budget) -> State:
    """Replay one bounded frame at a time, without retaining complete history."""
    integer(size, len(MAGIC) + HEADER.size + 33, MAX_FILE)
    require(read(0, len(MAGIC)) == MAGIC, 'wrong room readiness journal profile')
    offset, state = len(MAGIC), None
    names = {value: name for name, value in KINDS.items()}
    while offset < size:
        budget.work()
        require(size - offset >= HEADER.size + 33, 'incomplete room readiness frame')
        header = read(offset, HEADER.size)
        require(type(header) is bytes and len(header) == HEADER.size,
                'room readiness header ended during read')
        length, sequence, previous = HEADER.unpack(header)
        require(1 <= length <= MAX_BODY and length + HEADER.size + 32 <= size - offset,
                'truncated or oversized room readiness evidence')
        require(sequence == (0 if state is None else state.frames) and sequence < MAX_FRAMES
                and previous == (bytes(32) if state is None else state.tail),
                'room readiness journal chain mismatch')
        body = read(offset + HEADER.size, length)
        checksum = read(offset + HEADER.size + length, 32)
        require(type(body) is bytes and len(body) == length
                and type(checksum) is bytes and len(checksum) == 32,
                'room readiness evidence ended during read')
        require(checksum == hashlib.sha256(DOMAIN + header + body).digest(),
                'room readiness evidence integrity failed')
        require(body[0] in names, 'unknown room readiness transition')
        state = transition(state, names[body[0]], body[1:], budget)
        state = replace(state, frames=sequence + 1, tail=checksum)
        offset += HEADER.size + length + 32
    require(state is not None and offset == size, 'room readiness journal has no complete original goal')
    budget.remaining()
    return state


def replay(raw: bytes, budget: Budget) -> State:
    require(type(raw) is bytes, 'room readiness replay requires bytes')
    return replay_reader(lambda start, size: raw[start:start + size], len(raw), budget)


class Journal(_Custody):
    """Fixed joint-room codec with original custody around each publication."""

    def __init__(self, path: str, budget: Budget, *, writable: bool = False,
                 create: tuple[Goal, tuple[str, int]] | None = None):
        self._batch = None
        super().__init__(path, budget, writable=writable, create=create)

    @staticmethod
    def _magic() -> bytes:
        return MAGIC

    @staticmethod
    def _limits() -> tuple[int, int, int]:
        return MAX_FILE, MAX_FRAMES, MAX_BODY

    @staticmethod
    def _transition(state: State | None, kind: str, payload: bytes, budget: Budget) -> State:
        return transition(state, kind, payload, budget)

    @staticmethod
    def _frame(state: State | None, kind: str, payload: bytes) -> bytes:
        return frame(state, kind, payload)

    @staticmethod
    def _replay_reader(read: Callable[[int, int], bytes], size: int, budget: Budget) -> State:
        return replay_reader(read, size, budget)

    def bind_batch(self, batch: Batch) -> None:
        require(type(batch) is Batch and batch.budget is self.budget,
                'room monitor and original custody require one shared budget')
        require(self._batch is None or self._batch is batch, 'original room batch owner already bound')
        self._batch = batch
        self.check()

    def check(self) -> None:
        try:
            super().check()
            if self._batch is not None:
                # _Custody also checks after render and after fsync/readback,
                # before publishing the candidate or acknowledging completion.
                self.state.goal.origin.verify_batch(self._batch)
        except BaseException:
            self.fenced, self.read_owned = True, False
            raise

    def start_read(self) -> None:
        require(self._batch is not None, 'room sampling requires verified original batch custody')
        super().start_read()

    def accept(self, sample: readiness.Sample, render: Callable[[State], None]) -> None:
        require(self._batch is not None, 'room publication requires verified original batch custody')
        super().accept(sample, render)

    def close(self) -> None:
        self._batch = None
        super().close()
