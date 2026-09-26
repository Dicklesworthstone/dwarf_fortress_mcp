"""Private append-only custody for a complete receipt-linked furnishing plan.

The fixed plan codec shares the single-monitor filesystem owner. Plan receipts
and their simultaneous captured condition are one transition and one publication;
no individual item's earlier completion can discharge the whole plan.
Beads: df-dfhack-bridge-plane-c-pic.4 / df-dfhack-bridge-plane-c-pic.5.
"""
from __future__ import annotations

from dataclasses import dataclass, replace
import hashlib
from typing import Callable

from build_placement_wire import Rejected, integer, require, text
from construction_monitor_rpc import Budget, endpoint
from construction_monitor_store import HEADER, KINDS, _Custody
from construction_plan import Goal, LinkedSample, Progress, MAX_SAMPLE, advance, begin_read, cancel

MAGIC = b'DFMPJR01'
DOMAIN = b'dfmcp.construction-plan-journal/1\0'
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
    require(type(payload) is bytes, 'plan journal payload must be bytes')
    if state is None:
        require(kind == 'goal' and len(payload) >= 2, 'plan journal must begin with a complete goal')
        size = int.from_bytes(payload[:2], 'big')
        require(1 <= size <= 128 and len(payload) > size + 2, 'invalid plan endpoint framing')
        address = endpoint(text(payload[2:2 + size], 128))
        goal = Goal.decode(payload[2 + size:])
        return State(goal, address, Progress(goal.digest))
    require(not state.progress.terminal, 'record follows immutable terminal plan evidence')
    if kind == 'read_started':
        require(not payload, 'plan read-start record contains unexpected data')
        progress = begin_read(state.progress)
    elif kind == 'sample':
        progress = advance(state.progress, state.goal, LinkedSample.decode(payload), budget.work)
    elif kind == 'cancel':
        require(not payload, 'plan cancel record contains unexpected data')
        progress = cancel(state.progress)
    else:
        raise Rejected('unknown or repeated plan journal goal')
    return replace(state, progress=progress)


def frame(state: State | None, kind: str, payload: bytes) -> bytes:
    require(kind in KINDS and type(payload) is bytes and len(payload) < MAX_BODY, 'invalid bounded plan frame')
    sequence = 0 if state is None else state.frames
    require(sequence < MAX_FRAMES, 'plan monitor frame allowance exhausted')
    body = bytes([KINDS[kind]]) + payload
    header = HEADER.pack(len(body), sequence, bytes(32) if state is None else state.tail)
    return header + body + hashlib.sha256(DOMAIN + header + body).digest()


def replay_reader(read: Callable[[int, int], bytes], size: int, budget: Budget) -> State:
    """Replay every complete plan transition with one shared shrinking budget."""
    integer(size, len(MAGIC) + HEADER.size + 33, MAX_FILE)
    require(read(0, len(MAGIC)) == MAGIC, 'wrong construction-plan journal profile')
    offset, state = len(MAGIC), None
    names = {v: k for k, v in KINDS.items()}
    while offset < size:
        budget.work()
        require(size - offset >= HEADER.size + 33, 'incomplete construction plan frame')
        header = read(offset, HEADER.size)
        length, sequence, previous = HEADER.unpack(header)
        require(1 <= length <= MAX_BODY and length + HEADER.size + 32 <= size - offset,
                'truncated or oversized construction plan evidence')
        require(sequence == (0 if state is None else state.frames) and sequence < MAX_FRAMES
                and previous == (bytes(32) if state is None else state.tail), 'construction plan chain mismatch')
        body = read(offset + HEADER.size, length)
        checksum = read(offset + HEADER.size + length, 32)
        require(checksum == hashlib.sha256(DOMAIN + header + body).digest(), 'construction plan evidence integrity failed')
        require(body[0] in names, 'unknown construction plan transition')
        state = transition(state, names[body[0]], body[1:], budget)
        state = replace(state, frames=sequence + 1, tail=checksum)
        offset += HEADER.size + length + 32
    require(state is not None and offset == size, 'construction plan journal has no complete goal')
    budget.remaining()
    return state


def replay(raw: bytes, budget: Budget) -> State:
    require(type(raw) is bytes, 'plan journal replay requires bytes')
    return replay_reader(lambda start, size: raw[start:start + size], len(raw), budget)


class Journal(_Custody):
    """The fixed DFMPJR01 owner; files cannot switch this source-selected codec."""

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
