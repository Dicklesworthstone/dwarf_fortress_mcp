"""Fixed append-only original route-batch construction monitoring journal.

Reuses private no-follow custody, complete sync/readback and owned read intents.
Neither replay nor a sampled completion supplies placement authority. The caller
also holds and rechecks the original Batch; the journal alone proves no custody.
Beads: df-dfhack-bridge-plane-c-pic.4/.5.
"""
from __future__ import annotations

from dataclasses import dataclass, replace
import hashlib

from build_placement_wire import integer, require, text
from construction_monitor_rpc import endpoint
from construction_monitor_store import HEADER, KINDS, _Custody
from construction_plan import LinkedSample, Progress, MAX_SAMPLE, begin_read, cancel
from route_furniture_completion import Goal, MAX_GOAL, advance

MAGIC = b'DFMRGJ01'
DOMAIN = b'dfmcp-route-batch-construction-journal/1\0'
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


def transition(state, kind: str, payload: bytes, budget) -> State:
    budget.work()
    require(type(payload) is bytes, 'original completion payload must be bytes')
    if state is None:
        require(kind == 'goal' and 3 <= len(payload) <= MAX_GOAL + 130,
                'original completion journal requires a bounded complete goal')
        width = int.from_bytes(payload[:2], 'big')
        require(1 <= width <= 128 and len(payload) > width + 2, 'invalid completion endpoint')
        address = endpoint(text(payload[2:2 + width], 128))
        goal = Goal.decode(payload[2 + width:], budget.work)
        return State(goal, address, Progress(goal.digest))
    require(not state.progress.terminal, 'record after terminal original construction history')
    if kind == 'read_started':
        require(not payload, 'read intent must not contain a replacement goal')
        progress = begin_read(state.progress)
    elif kind == 'sample':
        progress = advance(state.progress, state.goal, LinkedSample.decode(payload), budget.work)
    elif kind == 'cancel':
        require(not payload, 'monitor cancellation must not contain effect instructions')
        progress = cancel(state.progress)
    else:
        raise ValueError('unknown original completion transition')
    return replace(state, progress=progress)


def frame(state, kind: str, payload: bytes) -> bytes:
    require(kind in KINDS and type(payload) is bytes and len(payload) < MAX_BODY,
            'invalid original completion frame')
    sequence = 0 if state is None else state.frames
    require(sequence < MAX_FRAMES, 'original completion event capacity exhausted')
    body = bytes([KINDS[kind]]) + payload
    header = HEADER.pack(len(body), sequence, bytes(32) if state is None else state.tail)
    return header + body + hashlib.sha256(DOMAIN + header + body).digest()


def replay_reader(read, size: int, budget) -> State:
    integer(size, len(MAGIC) + HEADER.size + 33, MAX_FILE)
    require(read(0, len(MAGIC)) == MAGIC, 'wrong original completion journal format')
    offset, state = len(MAGIC), None
    names = {value: key for key, value in KINDS.items()}
    while offset < size:
        budget.work()
        require(size - offset >= HEADER.size + 33, 'torn original completion frame')
        header = read(offset, HEADER.size)
        width, sequence, previous = HEADER.unpack(header)
        require(1 <= width <= MAX_BODY and width + HEADER.size + 32 <= size - offset
                and sequence == (0 if state is None else state.frames) and sequence < MAX_FRAMES
                and previous == (bytes(32) if state is None else state.tail),
                'original completion frame extent or chain differs')
        body = read(offset + HEADER.size, width)
        digest = read(offset + HEADER.size + width, 32)
        require(hashlib.sha256(DOMAIN + header + body).digest() == digest and body[0] in names,
                'corrupt original completion evidence')
        state = transition(state, names[body[0]], body[1:], budget)
        state = replace(state, frames=sequence + 1, tail=digest)
        offset += HEADER.size + width + 32
    require(state is not None and offset == size, 'incomplete original completion journal')
    budget.remaining()
    return state


class Journal(_Custody):
    _magic = staticmethod(lambda: MAGIC)
    _limits = staticmethod(lambda: (MAX_FILE, MAX_FRAMES, MAX_BODY))
    _transition = staticmethod(transition)
    _frame = staticmethod(frame)
    _replay_reader = staticmethod(replay_reader)
