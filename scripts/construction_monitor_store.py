"""Private append-only custody for one receipt-linked construction monitor.

This file contains monitoring evidence, never game-effect authority. Opening it
cannot renew the original deadline or authorize construction, removal or retry.
Beads: df-dfhack-bridge-plane-c-pic.4 / df-dfhack-bridge-plane-c-pic.5.
"""
from __future__ import annotations

from dataclasses import dataclass, replace
import fcntl
import hashlib
import os
from pathlib import Path
import stat
import struct
from typing import Callable

from build_placement_wire import Rejected, integer, require, field, text
from construction_receipt import Goal, LinkedSample, Progress, MAX_SAMPLE, advance, begin_read, cancel
from construction_monitor_rpc import Budget, endpoint

MAGIC = b'DFMCJR01'
DOMAIN = b'dfmcp.construction-journal/1\0'
MAX_FILE = 128 * 1024 * 1024
MAX_FRAMES = 1030
MAX_BODY = MAX_SAMPLE + 1
HEADER = struct.Struct('>II32s')
KINDS = {'goal': 1, 'read_started': 2, 'sample': 3, 'cancel': 4}


@dataclass(frozen=True)
class State:
    goal: Goal
    address: tuple[str, int]
    progress: Progress
    frames: int = 1
    tail: bytes = bytes(32)


def transition(state: State | None, kind: str, payload: bytes, budget: Budget) -> State:
    budget.work()
    require(type(payload) is bytes, 'journal payload must be bytes')
    if state is None:
        require(kind == 'goal' and len(payload) >= 2, 'journal must begin with a complete goal')
        size = int.from_bytes(payload[:2], 'big')
        require(1 <= size <= 128 and len(payload) > size + 2, 'invalid endpoint framing')
        address = endpoint(text(payload[2:2 + size], 128))
        goal = Goal.decode(payload[2 + size:])
        return State(goal, address, Progress(goal.digest))
    require(not state.progress.terminal, 'record follows immutable terminal monitor evidence')
    if kind == 'read_started':
        require(not payload, 'read-start record contains unexpected data')
        progress = begin_read(state.progress)
    elif kind == 'sample':
        progress = advance(state.progress, state.goal, LinkedSample.decode(payload), budget.work)
    elif kind == 'cancel':
        require(not payload, 'cancel record contains unexpected data')
        progress = cancel(state.progress)
    else:
        raise Rejected('unknown or repeated journal goal')
    return replace(state, progress=progress)


def frame(state: State | None, kind: str, payload: bytes) -> bytes:
    require(kind in KINDS and type(payload) is bytes and len(payload) < MAX_BODY, 'invalid bounded frame')
    sequence = 0 if state is None else state.frames
    require(sequence < MAX_FRAMES, 'monitor frame allowance exhausted')
    body = bytes([KINDS[kind]]) + payload
    header = HEADER.pack(len(body), sequence, bytes(32) if state is None else state.tail)
    return header + body + hashlib.sha256(DOMAIN + header + body).digest()


def replay_reader(read: Callable[[int, int], bytes], size: int, budget: Budget) -> State:
    """Replay bounded frames without retaining the entire journal in memory."""
    integer(size, len(MAGIC) + HEADER.size + 33, MAX_FILE)
    require(read(0, len(MAGIC)) == MAGIC, 'wrong construction-monitor journal profile')
    offset, state = len(MAGIC), None
    names = {v: k for k, v in KINDS.items()}
    while offset < size:
        budget.work()
        require(size - offset >= HEADER.size + 33, 'incomplete construction journal frame')
        header = read(offset, HEADER.size)
        length, sequence, previous = HEADER.unpack(header)
        require(1 <= length <= MAX_BODY and length + HEADER.size + 32 <= size - offset,
                'truncated or oversized construction evidence')
        require(sequence == (0 if state is None else state.frames) and sequence < MAX_FRAMES
                and previous == (bytes(32) if state is None else state.tail), 'construction journal chain mismatch')
        body = read(offset + HEADER.size, length)
        checksum = read(offset + HEADER.size + length, 32)
        require(checksum == hashlib.sha256(DOMAIN + header + body).digest(), 'construction evidence integrity failed')
        require(body[0] in names, 'unknown construction journal transition')
        state = transition(state, names[body[0]], body[1:], budget)
        state = replace(state, frames=sequence + 1, tail=checksum)
        offset += HEADER.size + length + 32
    require(state is not None and offset == size, 'construction journal has no complete goal')
    budget.remaining()
    return state


def replay(raw: bytes, budget: Budget) -> State:
    require(type(raw) is bytes, 'journal replay requires bytes')
    return replay_reader(lambda start, size: raw[start:start + size], len(raw), budget)


def private(info: os.stat_result, *, directory: bool = False) -> None:
    expected = 0o700 if directory else 0o600
    require((stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode))
            and stat.S_IMODE(info.st_mode) == expected and info.st_uid in (0, os.geteuid())
            and (directory or info.st_nlink == 1), 'construction evidence custody is not private and single-link')


def stamp(info: os.stat_result) -> tuple:
    return (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)


def parent(path: str) -> tuple[int, str]:
    require(os.name == 'posix' and hasattr(os, 'O_NOFOLLOW'), 'construction custody requires POSIX no-follow access')
    require(type(path) is str and 1 <= len(path) <= 4096 and '\0' not in path, 'invalid journal path')
    value = Path(path)
    require(value.is_absolute() and str(value) == path and '..' not in value.parts and len(value.parts) >= 2,
            'absolute canonical journal path required')
    fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        for part in value.parts[1:-1]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = child
        private(os.fstat(fd), directory=True)
        return fd, value.name
    except BaseException:
        os.close(fd)
        raise


def read_exact(fd: int, offset: int, count: int, budget: Budget) -> bytes:
    budget.reserve('disk_bytes', count)
    out = bytearray()
    while len(out) < count:
        budget.remaining()
        part = os.pread(fd, count - len(out), offset + len(out))
        require(bool(part), 'construction evidence ended during read')
        out += part
    return bytes(out)


def write_all(fd: int, raw: bytes, budget: Budget) -> None:
    budget.reserve('disk_bytes', len(raw))
    remaining = memoryview(raw)
    while remaining:
        budget.remaining()
        written = os.write(fd, remaining)
        require(written > 0, 'construction journal write made no progress')
        remaining = remaining[written:]


def read_private(path: str, maximum: int, budget: Budget) -> bytes:
    """Import a bounded private receipt without writes or following any symlink."""
    directory, name = parent(path)
    fd = None
    try:
        fd = os.open(name, os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC | os.O_NOFOLLOW, dir_fd=directory)
        before = os.fstat(fd)
        private(before)
        integer(before.st_size, 1, maximum)
        raw = read_exact(fd, 0, before.st_size, budget)
        after, named = os.fstat(fd), os.stat(name, dir_fd=directory, follow_symlinks=False)
        private(after)
        private(named)
        require(stamp(before) == stamp(after) == stamp(named), 'receipt changed during import')
        budget.remaining()
        return raw
    finally:
        if fd is not None:
            os.close(fd)
        os.close(directory)


class Journal:
    def __init__(self, path: str, budget: Budget, *, writable: bool = False,
                 create: tuple[Goal, tuple[str, int]] | None = None):
        self.path, self.budget, self.writable = path, budget, writable
        self.fd = self.directory = None
        self.state = None
        self.fenced, self.read_owned = False, False
        self.length = 0
        self._digest = hashlib.sha256()
        try:
            budget.remaining()
            self.directory, self.name = parent(path)
            info = os.fstat(self.directory)
            self.directory_identity = info.st_dev, info.st_ino
            flags = os.O_RDWR | os.O_APPEND if writable else os.O_RDONLY
            if create is not None:
                require(writable, 'read-only owner cannot create a monitor')
                goal, address = create
                address_text = f'{address[0]}:{address[1]}'
                require(endpoint(address_text) == address, 'noncanonical construction endpoint')
                payload = field(address_text.encode('ascii')) + goal.encode()
                initial = transition(None, 'goal', payload, budget)
                first = MAGIC + frame(None, 'goal', payload)
                flags |= os.O_CREAT | os.O_EXCL
            self.fd = os.open(self.name, flags | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC,
                              0o600, dir_fd=self.directory)
            private(os.fstat(self.fd))
            fcntl.flock(self.fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            if create is not None:
                write_all(self.fd, first, budget)
                os.fsync(self.fd)
                os.fsync(self.directory)
                self.length, self._digest = len(first), hashlib.sha256(first)
                self.state = replace(initial, frames=1, tail=first[-32:])
                self.check()
            else:
                before = self._identity()
                size = integer(before.st_size, len(MAGIC) + HEADER.size + 33, MAX_FILE)
                hasher = hashlib.sha256()
                def read(offset: int, count: int) -> bytes:
                    raw = read_exact(self.fd, offset, count, budget)
                    hasher.update(raw)
                    return raw
                state = replay_reader(read, size, budget)
                require(stamp(before) == stamp(self._identity()), 'journal changed during full replay')
                self.length, self._digest, self.state = size, hasher, state
                self.check()
        except BaseException:
            self.close()
            raise

    def _identity(self) -> os.stat_result:
        require(not self.fenced and self.fd is not None, 'construction journal fenced or closed')
        self.budget.remaining()
        private(os.fstat(self.directory), directory=True)
        other, name = parent(self.path)
        try:
            info = os.fstat(other)
            require((info.st_dev, info.st_ino) == self.directory_identity and name == self.name,
                    'construction parent directory replaced')
        finally:
            os.close(other)
        actual, named = os.fstat(self.fd), os.stat(self.name, dir_fd=self.directory, follow_symlinks=False)
        private(actual)
        private(named)
        require(stamp(actual) == stamp(named), 'construction journal pathname replaced')
        return actual

    def check(self) -> None:
        try:
            before = self._identity()
            require(before.st_size == self.length, 'construction journal extent changed')
            digest = hashlib.sha256()
            for offset in range(0, self.length, 1024 * 1024):
                digest.update(read_exact(self.fd, offset, min(1024 * 1024, self.length - offset), self.budget))
            require(stamp(before) == stamp(self._identity()) and digest.digest() == self._digest.digest(),
                    'construction journal bytes changed outside owner')
            self.budget.remaining()
        except BaseException:
            self.fenced = True
            self.read_owned = False
            raise

    def _append(self, kind: str, payload: bytes, candidate: State) -> None:
        require(self.writable, 'read-only monitor cannot append')
        self.check()
        raw = frame(self.state, kind, payload)
        require(self.length + len(raw) <= MAX_FILE, 'construction journal capacity exhausted')
        try:
            self.read_owned = False
            os.lseek(self.fd, 0, os.SEEK_END)
            write_all(self.fd, raw, self.budget)
            os.fsync(self.fd)
            os.fsync(self.directory)
            # Verify all old and new bytes, not just a possibly replaced tail.
            self.length += len(raw)
            self._digest.update(raw)
            self.check()
            self.state = replace(candidate, frames=self.state.frames + 1, tail=raw[-32:])
        except BaseException:
            self.fenced = True
            raise

    def start_read(self) -> None:
        require(self.writable and not self.read_owned, 'read intent already owned or journal is read-only')
        self.check()
        # Reserve a complete maximum-sized sample and future cancellation before
        # creating read intent or contacting DFHack. Refusal never truncates history.
        require(self.length + MAX_BODY + 3 * (HEADER.size + 33) <= MAX_FILE
                and self.state.frames + 3 <= MAX_FRAMES, 'cannot reserve complete read evidence')
        candidate = transition(self.state, 'read_started', b'', self.budget)
        self._append('read_started', b'', candidate)
        self.read_owned = True

    def accept(self, sample: LinkedSample, render: Callable[[State], None]) -> None:
        require(self.read_owned, 'reopened or unowned read cannot publish a sample')
        try:
            self.read_owned = False
            self.check()
            payload = sample.encode()
            candidate = transition(self.state, 'sample', payload, self.budget)
            # Complete result reservation precedes publication; output failure leaves
            # the already durable read-start as unknown, never a successful sample.
            render(candidate)
            self.budget.remaining()
            self._append('sample', payload, candidate)
        except BaseException:
            self.fenced = True
            raise

    def cancel(self, render: Callable[[State], None]) -> bool:
        self.check()
        if self.state.progress.terminal:
            return False
        candidate = transition(self.state, 'cancel', b'', self.budget)
        render(candidate)
        self._append('cancel', b'', candidate)
        return True

    def view(self) -> dict:
        self.check()
        return {'state': self.state.progress.view(), 'journal_head': self.state.tail.hex(),
                'journal_frames': self.state.frames, 'journal_bytes': self.length}

    def close(self) -> None:
        self.read_owned = False
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None
        if self.directory is not None:
            os.close(self.directory)
            self.directory = None

    def __enter__(self) -> Journal:
        return self

    def __exit__(self, *_args):
        self.close()
