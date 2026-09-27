"""Fixed append-only completion custody bound to the original furnishing batch.

The native read permit additionally requires a currently held and verified batch
owner. Cancellation can use an unbound owner when original custody is unavailable.
No existing batch or placement file is modified.
"""
from __future__ import annotations

from dataclasses import dataclass, replace
import hashlib
from typing import Callable

from build_placement_wire import Rejected, integer, require, text
from construction_monitor_rpc import Budget, endpoint
from construction_monitor_store import HEADER, KINDS, _Custody
from furniture_batch import Batch
from furniture_completion import Goal, LinkedSample, Progress, MAX_SAMPLE, advance, begin_read, cancel

MAGIC = b'DFMFCJ01'
DOMAIN = b'dfmcp.furniture-completion-journal/1\0'
MAX_FILE = 128 * 1024 * 1024
MAX_FRAMES = 1030
MAX_BODY = MAX_SAMPLE + 1


@dataclass(frozen=True)
class State:
    goal: Goal
    address: tuple[str, int]
    progress: Progress
    frames: int = 1
    tail: bytes = bytes(32)


def transition(state: State | None, kind: str, payload: bytes, budget: Budget) -> State:
    budget.work()
    require(type(payload) is bytes, 'completion journal payload must be bytes')
    if state is None:
        require(kind == 'goal' and len(payload) >= 2, 'completion journal must begin with the original goal')
        size = int.from_bytes(payload[:2], 'big')
        require(1 <= size <= 128 and len(payload) > size + 2, 'invalid completion endpoint framing')
        address = endpoint(text(payload[2:2 + size], 128))
        goal = Goal.decode(payload[2 + size:], budget.work)
        require(address == goal.origin.address, 'completion endpoint differs from original furnishing batch')
        return State(goal, address, Progress(goal.digest))
    require(not state.progress.terminal, 'record follows immutable terminal completion evidence')
    if kind == 'read_started':
        require(not payload, 'completion read-start record contains unexpected data')
        progress = begin_read(state.progress)
    elif kind == 'sample':
        progress = advance(state.progress, state.goal, LinkedSample.decode(payload), budget.work)
    elif kind == 'cancel':
        require(not payload, 'completion cancel record contains unexpected data')
        progress = cancel(state.progress)
    else:
        raise Rejected('unknown or repeated completion journal goal')
    return replace(state, progress=progress)


def frame(state: State | None, kind: str, payload: bytes) -> bytes:
    require(kind in KINDS and type(payload) is bytes and len(payload) < MAX_BODY, 'invalid bounded completion frame')
    sequence = 0 if state is None else state.frames
    require(sequence < MAX_FRAMES, 'completion monitor frame allowance exhausted')
    body = bytes([KINDS[kind]]) + payload
    header = HEADER.pack(len(body), sequence, bytes(32) if state is None else state.tail)
    return header + body + hashlib.sha256(DOMAIN + header + body).digest()


def replay_reader(read: Callable[[int, int], bytes], size: int, budget: Budget) -> State:
    integer(size, len(MAGIC) + HEADER.size + 33, MAX_FILE)
    require(read(0, len(MAGIC)) == MAGIC, 'wrong furnishing completion journal profile')
    offset, state = len(MAGIC), None
    names = {v: k for k, v in KINDS.items()}
    while offset < size:
        budget.work()
        require(size - offset >= HEADER.size + 33, 'incomplete furnishing completion frame')
        header = read(offset, HEADER.size)
        length, sequence, previous = HEADER.unpack(header)
        require(1 <= length <= MAX_BODY and length + HEADER.size + 32 <= size - offset,
                'truncated or oversized furnishing completion evidence')
        require(sequence == (0 if state is None else state.frames) and sequence < MAX_FRAMES
                and previous == (bytes(32) if state is None else state.tail), 'furnishing completion chain mismatch')
        body = read(offset + HEADER.size, length)
        checksum = read(offset + HEADER.size + length, 32)
        require(checksum == hashlib.sha256(DOMAIN + header + body).digest(), 'furnishing completion integrity failed')
        require(body[0] in names, 'unknown furnishing completion transition')
        state = transition(state, names[body[0]], body[1:], budget)
        state = replace(state, frames=sequence + 1, tail=checksum)
        offset += HEADER.size + length + 32
    require(state is not None and offset == size, 'completion journal has no complete original goal')
    budget.remaining()
    return state


def replay(raw: bytes, budget: Budget) -> State:
    require(type(raw) is bytes, 'completion journal replay requires bytes')
    return replay_reader(lambda start, size: raw[start:start + size], len(raw), budget)


class Journal(_Custody):
    """DFMFCJ01 owner with original-source custody checked around publication."""

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
                'completion and original furnishing custody require one shared budget')
        require(self._batch is None or self._batch is batch, 'completion original owner already bound')
        self._batch = batch
        self.check()

    def check(self) -> None:
        try:
            super().check()
            if self._batch is not None:
                # Shared _append invokes this after render and again after fsync
                # and full readback, before publishing the candidate in memory.
                self.state.goal.origin.verify_batch(self._batch)
        except BaseException:
            self.fenced, self.read_owned = True, False
            raise

    def start_read(self) -> None:
        require(self._batch is not None, 'completion read requires verified original furnishing custody')
        super().start_read()

    def accept(self, sample: LinkedSample, render: Callable[[State], None]) -> None:
        require(self._batch is not None, 'completion publication requires verified original furnishing custody')
        super().accept(sample, render)

    def close(self) -> None:
        self._batch = None
        super().close()
