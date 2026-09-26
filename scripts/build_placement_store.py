"""Private append-only furniture placement journals and a complete-directory pending fence.

Host-local cooperating custody, not cross-controller exclusion or anti-rollback
against a malicious owner. No repair, overwrite, eviction or commit replay.
Indeterminate native records are immutable retained evidence, not permission to
replace an uncertain receipt with a subsequent observation or another dispatch.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import stat

from build_placement_rpc import Budget, Manifest, Reply, endpoint
from build_placement_wire import Capture, Plan, Record, canonical, digest, exact_hex, integer, key_text, require

MAX_FILE = 32768
MAX_FRAME = 16384
MAX_PLACEMENTS = 256
SUFFIX = '.placement'
FORMAT = 'dfmcp.build-placement-journal/1'


def filename(key: str) -> str:
    key_text(key)
    return key + SUFFIX


def exact_object(value: object, keys: set[str]) -> dict:
    require(type(value) is dict and set(value) == keys, 'invalid journal fields')
    return value


def software_equal(a: Manifest, b: Manifest) -> bool:
    return (a.df_version, a.dfhack_version) == (b.df_version, b.dfhack_version)


def manifest_from(value: object) -> Manifest:
    fields = exact_object(value, {'generation', 'df_version', 'dfhack_version'})
    return Manifest(**fields)


def intent(plan: Plan, manifest: Manifest, address: tuple[str, int]) -> dict:
    require(manifest.generation == plan.before.generation, 'intent source mismatch')
    address_text = f'{address[0]}:{address[1]}'
    require(endpoint(address_text) == address, 'invalid intent endpoint')
    return {'key': plan.key, 'capture_hex': plan.before.encode().hex(),
            'plan_digest': plan.digest.hex(), 'token': plan.token.hex(),
            'manifest': manifest.view(), 'endpoint': address_text}


@dataclass(frozen=True)
class State:
    plan: Plan
    manifest: Manifest
    address: tuple[str, int]
    prepared: Record | None = None
    dispatched: bool = False
    terminal: Record | None = None
    terminal_manifest: Manifest | None = None
    frames: int = 1
    tail: str = ''

    @property
    def pending(self) -> bool:
        return self.terminal is None or not self.terminal.resolved

    def view(self) -> dict:
        return {'key': self.plan.key, 'plan_digest': self.plan.digest.hex(),
                'stage': (('terminal' if self.terminal.resolved else 'receipt_recorded') if self.terminal
                          else 'dispatch_recorded' if self.dispatched
                          else 'prepared_recorded' if self.prepared else 'intent_recorded'),
                'pending': self.pending, 'dispatch_intent_recorded': self.dispatched,
                'effect': None if self.terminal is None else self.terminal.view(),
                'effect_status': 'unknown' if self.terminal is None else self.terminal.phase,
                'original_manifest': self.manifest.view(),
                'receipt_manifest': None if self.terminal_manifest is None else self.terminal_manifest.view(),
                'retry_permitted': False, 'construction_completion_proved': False,
                'storage_acknowledged_this_call': False}


def apply(state: State | None, kind: str, payload: object) -> State:
    if state is None:
        require(kind == 'intent', 'journal must start with intent')
        p = exact_object(payload, {'key', 'capture_hex', 'plan_digest', 'token', 'manifest', 'endpoint'})
        plan = Plan(p['key'], Capture.decode(exact_hex(p['capture_hex'], maximum=2048)))
        require(exact_hex(p['plan_digest'], 32) == plan.digest and exact_hex(p['token'], 16) == plan.token,
                'journal plan commitment mismatch')
        manifest = manifest_from(p['manifest'])
        require(manifest.generation == plan.before.generation, 'intent generation mismatch')
        return State(plan, manifest, endpoint(p['endpoint']))
    require(state.terminal is None, 'record after immutable native receipt')
    from dataclasses import replace
    if kind == 'dispatch':
        p = exact_object(payload, {'plan_digest'})
        require(state.prepared is not None and not state.dispatched
                and p['plan_digest'] == state.plan.digest.hex(), 'invalid or repeated dispatch intent')
        return replace(state, dispatched=True)
    require(kind in ('prepared', 'terminal'), 'unknown journal transition')
    p = exact_object(payload, {'record_hex', 'manifest'})
    value = Record.decode(exact_hex(p['record_hex'], maximum=6144), state.plan)
    manifest = manifest_from(p['manifest'])
    require(software_equal(manifest, state.manifest) and manifest.generation >= state.manifest.generation,
            'receipt software/source mismatch')
    if kind == 'prepared':
        require(state.prepared is None and not state.dispatched and value.phase == 'prepared'
                and manifest == state.manifest, 'invalid preparation evidence')
        return replace(state, prepared=value)
    require(value.phase != 'prepared', 'prepared record cannot close journal')
    return replace(state, terminal=value, terminal_manifest=manifest)


def replay(raw: bytes) -> State:
    require(type(raw) is bytes and 0 < len(raw) <= MAX_FILE and raw.endswith(b'\n'), 'incomplete or oversized journal')
    lines = raw.splitlines(keepends=True)
    require(1 <= len(lines) <= 4, 'journal transition count exceeded')
    state, previous = None, '0' * 64
    from dataclasses import replace
    for index, line in enumerate(lines):
        require(len(line) <= MAX_FRAME, 'journal frame too large')
        frame = exact_object(json.loads(line), {'format', 'sequence', 'previous', 'kind', 'payload', 'checksum'})
        require(frame['format'] == FORMAT and type(frame['sequence']) is int and frame['sequence'] == index
                and frame['previous'] == previous, 'journal chain mismatch')
        payload = {key: value for key, value in frame.items() if key != 'checksum'}
        checksum = digest('dfmcp-build-placement-frame/1', canonical(payload)).hex()
        require(frame['checksum'] == checksum and canonical(frame) + b'\n' == line,
                'noncanonical or corrupt journal frame')
        state = apply(state, frame['kind'], frame['payload'])
        state = replace(state, frames=index + 1, tail=checksum)
        previous = checksum
    return state


def frame_bytes(state: State | None, kind: str, payload: dict) -> bytes:
    apply(state, kind, payload)  # Semantic validation precedes any write.
    frame = {'format': FORMAT, 'sequence': 0 if state is None else state.frames,
             'previous': '0' * 64 if state is None else state.tail, 'kind': kind, 'payload': payload}
    frame['checksum'] = digest('dfmcp-build-placement-frame/1', canonical(frame)).hex()
    out = canonical(frame) + b'\n'
    require(len(out) <= MAX_FRAME, 'journal frame too large')
    return out


def private(info, mode: int, directory: bool = False) -> None:
    require((stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode))
            and stat.S_IMODE(info.st_mode) == mode and info.st_uid in (0, os.geteuid())
            and (directory or info.st_nlink == 1), 'journal custody is not private and single-link')


def stamp(info) -> tuple:
    return info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns


def open_directory(path: str) -> int:
    require(os.name == 'posix' and hasattr(os, 'O_NOFOLLOW'), 'private journals require POSIX no-follow custody')
    p = Path(path)
    require(p.is_absolute() and str(p) == path and '..' not in p.parts and len(path) <= 4096,
            'absolute canonical directory required')
    fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        for part in p.parts[1:]:
            next_fd = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=fd)
            os.close(fd)
            fd = next_fd
        private(os.fstat(fd), 0o700, True)
        return fd
    except BaseException:
        os.close(fd)
        raise


class Journal:
    def __init__(self, owner: PlacementDirectory, name: str, fd: int):
        self.owner, self.name, self.fd = owner, name, fd
        self.raw = self._read()
        self.state = replay(self.raw)
        require(filename(self.state.plan.key) == name, 'journal filename does not identify its key')
        # Durable preparation proves what happened; it does not mint authority
        # for a process that reopened a journal after transport loss or restart.
        self.fresh_intent = False
        self.prepared_this_owner = False

    def _read(self) -> bytes:
        self.owner.budget.remaining()
        before = os.fstat(self.fd)
        private(before, 0o600)
        require(0 < before.st_size <= MAX_FILE, 'empty or oversized journal')
        raw = bytearray()
        while len(raw) <= before.st_size:
            part = os.pread(self.fd, before.st_size + 1 - len(raw), len(raw))
            if not part:
                break
            raw += part
        after = os.fstat(self.fd)
        named = os.stat(self.name, dir_fd=self.owner.fd, follow_symlinks=False)
        private(after, 0o600)
        private(named, 0o600)
        require(stamp(before) == stamp(after) == stamp(named) and len(raw) == before.st_size,
                'journal changed during read or pathname was replaced')
        self.owner.budget.remaining()
        return bytes(raw)

    def check(self) -> None:
        require(self._read() == self.raw, 'journal bytes changed outside owner')

    def append(self, kind: str, payload: dict) -> None:
        require(self.owner.writable, 'read-only journal owner')
        if kind in ('prepared', 'dispatch'):
            require(self.fresh_intent, 'reopened journal cannot acquire placement authority')
        if kind == 'dispatch':
            require(self.prepared_this_owner, 'dispatch needs preparation acknowledged by this owner')
        self.owner.check()
        new = self.raw + frame_bytes(self.state, kind, payload)
        # Complete replay before append bounds every transition and the total.
        state = replay(new)
        self.owner.budget.remaining()
        if kind == 'dispatch':
            # Consume in-memory dispatch authority before the first write.
            self.prepared_this_owner = False
        try:
            os.lseek(self.fd, 0, os.SEEK_END)
            write_all(self.fd, new[len(self.raw):])
            os.fsync(self.fd)
            os.fsync(self.owner.fd)
            require(self._read() == new, 'journal append readback mismatch')
            self.raw, self.state = new, state
            self.owner.check()
            if kind == 'prepared':
                self.prepared_this_owner = True
        except BaseException:
            self.owner.fenced = True
            raise  # Retain even a partial append; never repair or dispatch past it.

    def retain(self, reply: Reply, prepared: bool = False) -> bool:
        value = reply.record
        require(value is not None and Record.decode(value.raw, self.state.plan) == value,
                'reply has no exact matching record')
        require(software_equal(reply.manifest, self.state.manifest)
                and reply.manifest.generation >= self.state.manifest.generation,
                'native software changed or source regressed')
        if self.state.terminal is not None:
            require(value.raw == self.state.terminal.raw, 'conflicting immutable native receipt')
            self.owner.check()
            return False
        if not prepared and value.phase == 'prepared':
            return False
        self.append('prepared' if prepared else 'terminal',
                    {'record_hex': value.raw.hex(), 'manifest': reply.manifest.view()})
        return True


def write_all(fd: int, raw: bytes) -> None:
    remaining = memoryview(raw)
    while remaining:
        written = os.write(fd, remaining)
        require(written > 0, 'journal short write')
        remaining = remaining[written:]


class PlacementDirectory:
    def __init__(self, path: str, budget: Budget, writable: bool = False):
        self.path, self.budget, self.writable = path, budget, writable
        self.fd = None
        self.journals = {}
        self.fenced = False
        try:
            self.fd = open_directory(path)
            import fcntl
            fcntl.flock(self.fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            info = os.fstat(self.fd)
            self.identity = info.st_dev, info.st_ino
            for name in self.names():
                flags = os.O_RDWR | os.O_APPEND if writable else os.O_RDONLY
                fd = os.open(name, flags | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC, dir_fd=self.fd)
                try:
                    self.journals[name] = Journal(self, name, fd)
                except BaseException:
                    os.close(fd)
                    raise
            self.check()
        except BaseException:
            self.close()
            raise

    def names(self) -> list[str]:
        self.budget.remaining()
        names = []
        with os.scandir(self.fd) as entries:
            for entry in entries:
                require(len(names) < MAX_PLACEMENTS, 'placement directory capacity exceeded')
                require(entry.name.endswith(SUFFIX), 'unrecognized placement directory entry')
                key_text(entry.name[:-len(SUFFIX)])
                names.append(entry.name)
        return sorted(names)

    def check(self) -> None:
        require(not self.fenced and self.fd is not None, 'placement directory fenced or closed')
        self.budget.remaining()
        private(os.fstat(self.fd), 0o700, True)
        named = open_directory(self.path)
        try:
            info = os.fstat(named)
            require((info.st_dev, info.st_ino) == self.identity, 'placement directory pathname replaced')
        finally:
            os.close(named)
        require(self.names() == sorted(self.journals), 'placement directory membership changed')
        for journal in self.journals.values():
            journal.check()
        self.budget.remaining()

    def ready(self, key: str) -> None:
        self.check()
        require(filename(key) not in self.journals, 'placement key already retained; use query/cancel, never place')
        require(len(self.journals) < MAX_PLACEMENTS, 'placement directory full')
        require(not any(j.state.pending for j in self.journals.values()),
                'unresolved placement blocks new keys in this directory')

    def create(self, plan: Plan, manifest: Manifest, address: tuple[str, int]) -> Journal:
        require(self.writable, 'read-only placement directory')
        self.ready(plan.key)
        raw = frame_bytes(None, 'intent', intent(plan, manifest, address))
        replay(raw)
        name = filename(plan.key)
        fd = None
        try:
            self.budget.remaining()
            fd = os.open(name, os.O_RDWR | os.O_APPEND | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK,
                         0o600, dir_fd=self.fd)
            private(os.fstat(fd), 0o600)
            write_all(fd, raw)
            os.fsync(fd)
            os.fsync(self.fd)
            journal = Journal(self, name, fd)
            self.journals[name] = journal
            fd = None  # Owner now closes this descriptor.
            self.check()
            journal.fresh_intent = True
            return journal
        except BaseException:
            self.fenced = True
            if fd is not None:
                os.close(fd)
            raise

    def get(self, key: str) -> Journal:
        self.check()
        name = filename(key)
        require(name in self.journals, 'placement key not present in this directory')
        return self.journals[name]

    def inventory(self, limit: int = 32, continuation: str | None = None) -> dict:
        integer(limit, 1, 64)
        self.check()
        witness = {'path': self.path, 'identity': list(self.identity),
                   'files': [[name, hashlib.sha256(j.raw).hexdigest()] for name, j in sorted(self.journals.items())]}
        identity = digest('dfmcp-build-placement-inventory/1', canonical(witness)).hex()
        offset = 0
        if continuation is not None:
            token = exact_object(json.loads(exact_hex(continuation, maximum=1024)), {'inventory', 'offset', 'limit'})
            require(token['inventory'] == identity and type(token['limit']) is int and token['limit'] == limit,
                    'stale or substituted inventory page')
            offset = integer(token['offset'], 1, MAX_PLACEMENTS)
            require(offset < len(self.journals) and offset % limit == 0, 'invalid inventory page offset')
        # Pending first makes unresolved work visible even after many resolved placements.
        ordered = sorted(self.journals.values(), key=lambda j: (not j.state.pending, j.name))
        rows = [{'key': j.state.plan.key, 'plan_digest': j.state.plan.digest.hex(), 'pending': j.state.pending,
                 'phase': j.state.terminal.phase if j.state.terminal else 'unknown',
                 'dispatch_intent_recorded': j.state.dispatched} for j in ordered[offset:offset + limit]]
        consumed = offset + len(rows)
        next_page = canonical({'inventory': identity, 'offset': consumed, 'limit': limit}).hex() if consumed < len(ordered) else None
        return {'inventory_digest': identity, 'total': len(ordered), 'pending': sum(j.state.pending for j in ordered),
                'operator_attention': sum(j.state.terminal is not None and not j.state.terminal.resolved for j in ordered),
                'rows': rows, 'continuation': next_page, 'coverage': 'complete_local_directory',
                'retry_permitted': False, 'construction_completion_proved': False}

    def close(self) -> None:
        for journal in self.journals.values():
            os.close(journal.fd)
        self.journals.clear()
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None

    def __enter__(self) -> PlacementDirectory:
        return self

    def __exit__(self, *_args):
        self.close()
