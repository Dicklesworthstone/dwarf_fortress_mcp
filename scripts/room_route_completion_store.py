"""Append-only original-room monitor using the existing private-file owner.

A new fixed format prevents older furniture-only readers from dropping room
requirements. Replay cannot renew timing, replace the parent or own a read.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from dataclasses import dataclass, replace
import hashlib

from build_placement_wire import integer, require, text
from construction_monitor_rpc import endpoint
from construction_monitor_store import HEADER, KINDS, _Custody
from room_route_completion import (
    Goal, Sample, Progress, MAX_GOAL, MAX_SAMPLE, advance, begin_read, cancel,
)

MAGIC = b'DFMRQJ01'
DOMAIN = b'dfmcp-room-route-completion-journal/1\0'
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


def transition(state, kind, payload, budget):
    budget.work()
    require(type(payload) is bytes, 'room completion payload must be bytes')
    if state is None:
        require(kind == 'goal' and 3 <= len(payload) <= MAX_GOAL + 130,
                'room journal requires a bounded complete original goal')
        width = int.from_bytes(payload[:2], 'big')
        require(1 <= width <= 128 and len(payload) > width + 2, 'invalid room endpoint framing')
        address = endpoint(text(payload[2:2 + width], 128))
        goal = Goal.decode(payload[2 + width:], budget.work)
        _, review = goal.handoff.unpack(budget.work)
        source = review.unpack(budget.work)[2]
        require(address == endpoint(source.endpoint), 'journal endpoint differs from original room source')
        return State(goal, address, Progress(goal.digest))
    require(not state.progress.terminal, 'record after terminal room completion history')
    if kind == 'read_started':
        require(not payload, 'read intent cannot replace the original room')
        progress = begin_read(state.progress)
    elif kind == 'sample':
        progress = advance(state.progress, state.goal, Sample.decode(payload), budget.work)
    elif kind == 'cancel':
        require(not payload, 'monitor cancellation cannot carry effect instructions')
        progress = cancel(state.progress)
    else:
        raise ValueError('unknown room completion transition')
    return replace(state, progress=progress)


def frame(state, kind, payload):
    require(kind in KINDS and type(payload) is bytes and len(payload) < MAX_BODY,
            'invalid bounded room completion frame')
    sequence = 0 if state is None else state.frames
    require(sequence < MAX_FRAMES, 'room monitor event capacity exhausted')
    body = bytes([KINDS[kind]]) + payload
    header = HEADER.pack(len(body), sequence, bytes(32) if state is None else state.tail)
    return header + body + hashlib.sha256(DOMAIN + header + body).digest()


def replay_reader(read, size, budget):
    integer(size, len(MAGIC) + HEADER.size + 33, MAX_FILE)
    require(read(0, len(MAGIC)) == MAGIC, 'wrong room completion journal format')
    offset, state = len(MAGIC), None
    names = {value: key for key, value in KINDS.items()}
    while offset < size:
        budget.work()
        require(size - offset >= HEADER.size + 33, 'torn room completion frame')
        header = read(offset, HEADER.size)
        width, sequence, previous = HEADER.unpack(header)
        require(1 <= width <= MAX_BODY and width + HEADER.size + 32 <= size - offset
                and sequence == (0 if state is None else state.frames) and sequence < MAX_FRAMES
                and previous == (bytes(32) if state is None else state.tail),
                'room completion frame extent or chain differs')
        body = read(offset + HEADER.size, width)
        digest = read(offset + HEADER.size + width, 32)
        require(hashlib.sha256(DOMAIN + header + body).digest() == digest and body[0] in names,
                'corrupt original room evidence')
        state = transition(state, names[body[0]], body[1:], budget)
        state = replace(state, frames=sequence + 1, tail=digest)
        offset += HEADER.size + width + 32
    require(state is not None and offset == size, 'incomplete original room journal')
    budget.remaining()
    return state


class Journal(_Custody):
    _magic = staticmethod(lambda: MAGIC)
    _limits = staticmethod(lambda: (MAX_FILE, MAX_FRAMES, MAX_BODY))
    _transition = staticmethod(transition)
    _frame = staticmethod(frame)
    _replay_reader = staticmethod(replay_reader)
