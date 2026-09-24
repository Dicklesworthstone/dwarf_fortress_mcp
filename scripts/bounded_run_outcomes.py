"""Durable historical terminal evidence for the existing run/1.13 client.

Beads: df-dfhack-bridge-plane-c-pic.5, df-action-coordinator-exec-ero.4.
The native record is replayed through the existing strict codec. Neither an
intent nor retained source-loss evidence grants permission to retry an unpause.
"""
from __future__ import annotations

from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import stat
import struct
from typing import Iterator

import bounded_run_client as c

SUFFIX = '.outcome.json'
FORMAT = 'dfmcp.bounded-run-outcome/1'
TERMINAL = frozenset(('stopped', 'refused', 'source_lost'))
MAX_FILE = 8192
MAX_OUTPUT = 32768
OPEN_READ = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC


def _fingerprint(info: os.stat_result) -> tuple:
    return (info.st_dev, info.st_ino, info.st_mode, info.st_uid, info.st_nlink,
            info.st_size, info.st_mtime_ns, info.st_ctime_ns)


def _private(info: os.stat_result) -> None:
    c.require(stat.S_ISREG(info.st_mode) and stat.S_IMODE(info.st_mode) == 0o600
              and info.st_uid in (0, os.geteuid()) and info.st_nlink == 1,
              'run evidence must be a private regular single-link file')


def _open_parent(path: Path) -> int:
    c.require(path.is_absolute() and '..' not in path.parts and len(str(path)) <= 4096
              and len(os.fsencode(path.name + SUFFIX)) <= 255,
              'run evidence path must be absolute and bounded without parent traversal')
    fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        for part in path.parts[1:-1]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=fd)
            os.close(fd)
            fd = child
        info = os.fstat(fd)
        c.require(stat.S_IMODE(info.st_mode) == 0o700 and info.st_uid in (0, os.geteuid()),
                  'run evidence parent must have owned exact mode 0700')
        return fd
    except BaseException:
        os.close(fd)
        raise


def _read(fd: int, parent: int, name: str) -> bytes:
    before = os.fstat(fd)
    _private(before)
    c.require(1 <= before.st_size <= MAX_FILE, 'empty or oversized run evidence')
    data = bytearray()
    while len(data) <= before.st_size:
        part = os.pread(fd, before.st_size + 1 - len(data), len(data))
        if not part:
            break
        data.extend(part)
    after = os.fstat(fd)
    named = os.stat(name, dir_fd=parent, follow_symlinks=False)
    c.require(_fingerprint(before) == _fingerprint(after) == _fingerprint(named)
              and len(data) == before.st_size, 'run evidence changed during read')
    return bytes(data)


def _json(data: bytes) -> dict:
    value = json.loads(data)
    c.require(isinstance(value, dict) and data == c.canonical(value) + b'\n',
              'run evidence must have intact canonical JSON bytes')
    return value


def record_bytes(record: dict) -> bytes:
    """Reconstruct EXACT native bytes, including its original receipt, then verify.

    The decoder retains every wire field. Re-encoding avoids adding raw bytes to
    the public RPC result; equality also checks every derived presentation field.
    """
    tick = record['observed_tick']
    for field in ('unpause_attempted', 'pause_verified', 'current_pause_unproved'):
        c.require(type(record[field]) is bool, 'invalid decoded run record flag')
    raw = (b'DFMRE013' + c.key_bytes(record['key'])
           + struct.pack('>II', c.integer(record['game_ticks'], 1, 1200), c.integer(record['wall_ms'], 1, 60000))
           + c.exact_hex(record['observation_hex'], 35)
           + c.exact_hex(record['plan_digest_hex'], 32) + c.exact_hex(record['prepare_token_hex'], 16)
           + struct.pack('>BBBBBQ', c.PHASES.index(record['phase']), c.REASONS.index(record['reason']),
                         record['unpause_attempted'], record['pause_verified'], tick is not None,
                         0 if tick is None else c.integer(tick, 0, c.MAX_TICK))
           + c.exact_hex(record['receipt_digest_hex'], 32))
    c.require(c.canonical(c.decode_record(raw)) == c.canonical(record), 'decoded native record was modified')
    return raw


def _payload(intent: dict, result: dict) -> dict:
    c.match_record(result, intent)
    manifest = result['manifest']
    c.require(set(manifest) == {'generation', 'df_version', 'dfhack_version'}, 'invalid outcome source manifest')
    generation = c.integer(manifest['generation'], 1, 2**64 - 2)
    raw = record_bytes(result['record'])
    record = c.decode_record(raw)
    c.require(record['phase'] in TERMINAL and record['before']['generation'] <= generation,
              'outcome is not a source-bound terminal native record')
    return {'format': FORMAT, 'intent_sha256': hashlib.sha256(c.canonical(intent)).hexdigest(),
            'manifest': dict(manifest), 'record_hex': raw.hex()}


def _envelope(payload: dict) -> bytes:
    value = {'outcome': payload, 'sha256': c.digest(b'dfmcp-bounded-run-outcome/1', c.canonical(payload)).hex()}
    data = c.canonical(value) + b'\n'
    c.require(len(data) <= MAX_FILE, 'run outcome exceeds retention bound')
    return data


class OutcomeStore:
    """Pinned intent + exclusive directory ownership; no repair, rewrite or prune.

    POSIX advisory locks serialize cooperating recovery processes. Host-local
    custody is not a global controller lease or protection against owner rollback.
    Even offline inspection opens only read-only descriptors and never fsyncs.
    """
    def __init__(self, path: Path, *, parent_fd: int | None = None):
        self.path = path
        self.borrowed_parent = parent_fd
        self.parent = self.source = self.outcome = None
        self.intent = None
        self.source_data = self.outcome_data = None
        self.source_stat = self.outcome_stat = None

    def __enter__(self) -> OutcomeStore:
        import fcntl
        try:
            # dup shares the batch owner's lock; check() still proves exact named-parent identity.
            self.parent = (_open_parent(self.path) if self.borrowed_parent is None
                           else os.dup(self.borrowed_parent))
            fcntl.flock(self.parent, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.source = os.open(self.path.name, OPEN_READ, dir_fd=self.parent)
            fcntl.flock(self.source, fcntl.LOCK_SH | fcntl.LOCK_NB)
            self.source_data = _read(self.source, self.parent, self.path.name)
            self.source_stat = _fingerprint(os.fstat(self.source))
            value = _json(self.source_data)
            c.require(set(value) == {'intent', 'sha256'}, 'invalid intent envelope')
            self.intent = c.verify_intent(value['intent'])
            c.require(value['sha256'] == hashlib.sha256(c.canonical(self.intent)).hexdigest(), 'intent checksum mismatch')
            self.load()  # Corrupt retained evidence refuses BEFORE any native call.
            self.check()
            return self
        except BaseException:
            self.close()
            raise

    def check(self) -> None:
        current = _open_parent(self.path)
        try:
            left, right = os.fstat(current), os.fstat(self.parent)
            c.require((left.st_dev, left.st_ino) == (right.st_dev, right.st_ino), 'run evidence parent was replaced')
        finally:
            os.close(current)
        c.require(_fingerprint(os.fstat(self.source)) == self.source_stat
                  and _read(self.source, self.parent, self.path.name) == self.source_data,
                  'retained run intent changed')
        if self.outcome is not None:
            c.require(_fingerprint(os.fstat(self.outcome)) == self.outcome_stat
                      and _read(self.outcome, self.parent, self.path.name + SUFFIX) == self.outcome_data,
                      'retained run outcome changed')
        else:
            try:
                os.stat(self.path.name + SUFFIX, dir_fd=self.parent, follow_symlinks=False)
            except FileNotFoundError:
                return
            raise c.Rejected('unexpected outcome publication during custody')

    def load(self) -> dict | None:
        if self.outcome is None:
            try:
                self.outcome = os.open(self.path.name + SUFFIX, OPEN_READ, dir_fd=self.parent)
            except FileNotFoundError:
                return None
            self.outcome_data = _read(self.outcome, self.parent, self.path.name + SUFFIX)
            self.outcome_stat = _fingerprint(os.fstat(self.outcome))
        value = _json(self.outcome_data)
        c.require(set(value) == {'outcome', 'sha256'}, 'invalid outcome envelope')
        payload = value['outcome']
        c.require(isinstance(payload, dict) and set(payload) == {'format', 'intent_sha256', 'manifest', 'record_hex'},
                  'invalid retained outcome fields')
        encoded = payload['record_hex']
        c.require(isinstance(encoded, str) and 294 <= len(encoded) <= 548 and len(encoded) % 2 == 0,
                  'invalid retained native record extent')
        result = {'manifest': payload['manifest'], 'record': c.decode_record(c.exact_hex(encoded, len(encoded) // 2))}
        c.require(c.canonical(_payload(self.intent, result)) == c.canonical(payload)
                  and _envelope(payload) == self.outcome_data, 'run outcome does not bind this intent')
        return result

    def view(self, result: dict | None = None, *, synced: bool = False, contacted: bool = False) -> dict:
        result = self.load() if result is None else result
        phase = result.get('record', {}).get('phase') if result is not None else None
        terminal = phase in TERMINAL
        status = {'stopped': 'historical_pause_verified', 'refused': 'historical_unpause_refused',
                  'source_lost': 'indeterminate_source_lost'}.get(phase, 'unknown')
        view = {**(result or {}), 'intent': self.intent, 'effect_status': status,
                'evidence_scope': 'verified_historical_native_record' if terminal else 'recorded_intent_only',
                'terminal_record_retained': terminal, 'native_contacted': contacted,
                'storage_acknowledged_this_call': synced, 'retry_permitted': False,
                'current_pause_unproved': True, 'goal_completion_proved': False,
                'requires_operator_attention': phase == 'source_lost',
                'native_record_status': phase or ('absent' if contacted else 'unqueried')}
        c.require(len(c.canonical(view)) <= MAX_OUTPUT, 'complete run recovery output exceeds bound')
        return view

    def retain(self, result: dict) -> dict:
        """Sync terminal native bytes before acknowledging them; leave ambiguous work unresolved."""
        self.check()
        previous = self.load()
        phase = result.get('record', {}).get('phase')
        if previous is not None:
            c.require('record' in result and record_bytes(previous['record']) == record_bytes(result['record']),
                      'native result contradicts immutable retained terminal evidence')
            _payload(self.intent, result)
            self.check()
            return self.view(contacted=True)
        if phase not in TERMINAL:
            c.match_record(result, self.intent)
            self.check()
            return self.view(result, contacted=True)
        payload = _payload(self.intent, result)
        data = _envelope(payload)
        # Reserve the complete final result before the first durable side effect.
        view = self.view({'manifest': payload['manifest'], 'record': result['record']}, synced=True, contacted=True)
        fd = os.open(self.path.name + SUFFIX, os.O_RDWR | os.O_CREAT | os.O_EXCL
                     | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC, 0o600, dir_fd=self.parent)
        try:
            _private(os.fstat(fd))
            remaining = memoryview(data)
            while remaining:
                count = os.write(fd, remaining)
                c.require(count > 0, 'short run outcome write')
                remaining = remaining[count:]
            os.fsync(fd)
            os.fsync(self.parent)
            c.require(_read(fd, self.parent, self.path.name + SUFFIX) == data, 'outcome bytes changed after sync')
            self.outcome_data = data
            self.outcome_stat = _fingerprint(os.fstat(fd))
            self.outcome, fd = fd, None
            self.check()
            return view
        finally:
            if fd is not None:
                os.close(fd)
            # Never unlink incomplete or uncertain evidence and never retry the effect.

    def close(self) -> None:
        for field in ('outcome', 'source', 'parent'):
            fd = getattr(self, field)
            if fd is not None:
                os.close(fd)
                setattr(self, field, None)

    def __exit__(self, error_type, *_args) -> None:
        try:
            if error_type is None:
                self.check()
        finally:
            self.close()


def inspect(path: Path) -> dict:
    with OutcomeStore(path) as store:
        return store.view()


def retain(path: Path, result: dict) -> dict:
    with OutcomeStore(path) as store:
        return store.retain(result)


@contextmanager
def new_run(path: Path) -> Iterator[None]:
    """Hold recovery ownership and refuse orphan evidence before a new dispatch."""
    import fcntl
    parent = _open_parent(path)
    try:
        fcntl.flock(parent, fcntl.LOCK_EX | fcntl.LOCK_NB)
        for name in (path.name, path.name + SUFFIX):
            try:
                os.stat(name, dir_fd=parent, follow_symlinks=False)
            except FileNotFoundError:
                continue
            raise c.Rejected('new run requires unused intent and outcome paths')
        yield
    finally:
        os.close(parent)
