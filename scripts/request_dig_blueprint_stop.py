#!/usr/bin/env python3
"""Durably revoke an original dig batch even while its executor holds the lock.

This command only publishes the existing immutable stopped.json marker. It does
not take the executor's directory/manifest locks, contact DFHack, cancel a native
operation, repair custody, or grant authority. Already-dispatched work may finish;
its original keys and receipts remain the only recovery path.
Beads: df-dfhack-bridge-plane-c-pic.4/.5; df-cx-authority-budget-threading-lmu.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys
import time

MANIFEST = 'blueprint.json'
EFFECTS = 'effects'
STOP = 'stopped.json'
LEGACY_FORMAT = 'dfmcp.dig-blueprint-batch/1'
ROOM_FORMAT = 'dfmcp.dig-blueprint-batch/2'
STOP_FORMAT = 'dfmcp.dig-blueprint-stop/1'
SCHEMA = 'dfmcp.dig-blueprint-stop-request/1'
MAX_MANIFEST = 65536
ROOM_MAX_MANIFEST = MAX_MANIFEST + 262144
MAX_STOP = 1024
CHUNK = 16384


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True,
                      allow_nan=False).encode('ascii')


def envelope(value: object) -> bytes:
    return canonical({'value': value, 'sha256': hashlib.sha256(canonical(value)).hexdigest()}) + b'\n'


class Budget:
    def __init__(self, timeout_ms: int = 10000):
        require(type(timeout_ms) is int and 1 <= timeout_ms <= 60000, 'invalid stop deadline')
        self.deadline = time.monotonic() + timeout_ms / 1000
        self.work_left = 100000

    def check(self) -> None:
        require(self.work_left > 0 and time.monotonic() < self.deadline,
                'stop-request budget exhausted; inspect original custody')
        self.work_left -= 1


def identity(value: object) -> str:
    require(type(value) is str and re.fullmatch('[0-9a-f]{64}', value) is not None,
            'exact original batch identity required')
    return value


def decode(raw: bytes, budget: Budget) -> dict:
    """Bound JSON before decoding; replay bytes, never trust an outer checksum."""
    depth, quoted, escaped = 0, False, False
    for i, byte in enumerate(raw):
        if i % 256 == 0:
            budget.check()
        if quoted:
            if escaped:
                escaped = False
            elif byte == 92:
                escaped = True
            elif byte == 34:
                quoted = False
        elif byte == 34:
            quoted = True
        elif byte in (91, 123):
            depth += 1
            require(depth <= 14, 'stop input nesting bound exceeded')
        elif byte in (93, 125):
            depth -= 1

    def unique(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, 'duplicate stop input field')
            result[key] = value
        return result

    def nonfinite(_value):
        raise ValueError('nonfinite stop input number')

    value = json.loads(raw.decode('utf-8'), object_pairs_hook=unique, parse_constant=nonfinite)
    require(type(value) is dict and set(value) == {'value', 'sha256'}
            and envelope(value['value']) == raw, 'noncanonical or corrupt stop input')
    budget.check()
    return value['value']


def signature(info) -> tuple:
    return (info.st_dev, info.st_ino, info.st_mode, info.st_uid, info.st_nlink,
            info.st_size, info.st_mtime_ns, info.st_ctime_ns)


class Directory:
    def __init__(self, root: Path, fd: int, budget: Budget):
        self.root, self.fd, self.budget = root, fd, budget
        opened = os.fstat(fd)
        self.identity = opened.st_dev, opened.st_ino
        self.uid = opened.st_uid
        self.verify()

    def verify(self) -> None:
        self.budget.check()
        named, opened = os.stat(self.root, follow_symlinks=False), os.fstat(self.fd)
        require(self.root.resolve(strict=True) == self.root
                and (named.st_dev, named.st_ino) == (opened.st_dev, opened.st_ino) == self.identity
                and stat.S_ISDIR(named.st_mode) and stat.S_ISDIR(opened.st_mode)
                and stat.S_IMODE(named.st_mode) == stat.S_IMODE(opened.st_mode) == 0o700
                and named.st_uid == opened.st_uid == self.uid and self.uid in (0, os.geteuid()),
                'original stop directory custody changed')

    def names(self) -> set[str]:
        self.verify()
        names = set()
        with os.scandir(self.fd) as entries:
            for entry in entries:
                self.budget.check()
                require(len(names) < 3, 'unexpected batch directory entries')
                names.add(entry.name)
        require(names in ({MANIFEST, EFFECTS}, {MANIFEST, EFFECTS, STOP}),
                'incomplete or unexpected original batch directory')
        self.verify()
        return names


@contextmanager
def directory(root: Path, budget: Budget):
    require(os.name == 'posix' and hasattr(os, 'O_NOFOLLOW'), 'stop requests need POSIX custody')
    require(isinstance(root, Path) and root.is_absolute() and '..' not in root.parts
            and 1 <= len(str(root)) <= 4096, 'stop directory must be normalized and absolute')
    fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        for part in root.parts[1:]:
            budget.check()
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=fd)
            os.close(fd)
            fd = child
        # Deliberately no flock here. This one fixed-name write only revokes work.
        owner = Directory(root, fd, budget)
        yield owner
        owner.verify()
    finally:
        os.close(fd)


class Pin:
    def __init__(self, owner: Directory, name: str, fd: int, raw: bytes):
        self.owner, self.name, self.fd, self.raw = owner, name, fd, raw
        self.signature = signature(os.fstat(fd))
        self.verify()

    def verify(self) -> None:
        self.owner.verify()
        opened = os.fstat(self.fd)
        named = os.stat(self.name, dir_fd=self.owner.fd, follow_symlinks=False)
        require(signature(named) == signature(opened) == self.signature
                and stat.S_ISREG(opened.st_mode) and stat.S_IMODE(opened.st_mode) == 0o600
                and opened.st_uid == self.owner.uid and opened.st_nlink == 1
                and opened.st_size == len(self.raw), 'stop file custody changed')
        require(read_bytes(self.fd, len(self.raw), self.owner.budget) == self.raw
                and signature(os.fstat(self.fd)) == self.signature,
                'stop file changed during verification')
        self.owner.verify()


def read_bytes(fd: int, size: int, budget: Budget) -> bytes:
    os.lseek(fd, 0, os.SEEK_SET)
    result = bytearray()
    while len(result) <= size:
        budget.check()
        part = os.read(fd, min(CHUNK, size + 1 - len(result)))
        if not part:
            break
        result += part
    require(len(result) == size, 'stop input extent changed')
    return bytes(result)


@contextmanager
def pinned(owner: Directory, name: str, maximum: int):
    owner.verify()
    fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC, dir_fd=owner.fd)
    try:
        before = os.fstat(fd)
        require(stat.S_ISREG(before.st_mode) and stat.S_IMODE(before.st_mode) == 0o600
                and before.st_uid == owner.uid and before.st_nlink == 1
                and 1 <= before.st_size <= maximum, 'invalid stop input file')
        raw = read_bytes(fd, before.st_size, owner.budget)
        require(signature(os.fstat(fd)) == signature(before), 'stop input changed during read')
        pin = Pin(owner, name, fd, raw)
        yield pin
        pin.verify()
    finally:
        os.close(fd)


def manifest_identity(raw: bytes, budget: Budget) -> tuple[str, tuple[int, int]]:
    value = decode(raw, budget)
    require(type(value) is dict and value.get('format') in (LEGACY_FORMAT, ROOM_FORMAT),
            'unsupported original batch format')
    is_room = value['format'] == ROOM_FORMAT
    require(len(raw) <= (ROOM_MAX_MANIFEST if is_room else MAX_MANIFEST), 'batch manifest exceeds profile bound')
    effect_id = value.get('effects_identity')
    require(type(effect_id) is list and len(effect_id) == 2
            and all(type(n) is int and 0 <= n < 2**64 for n in effect_id),
            'invalid original effects directory identity')
    domain = b'dfmcp-dig-blueprint-batch/2' if is_room else b'dfmcp-dig-blueprint-batch/1'
    batch_id = hashlib.sha256(domain + b'\0' + canonical(value)).hexdigest()
    # This is identity-only revocation, not room/terrain/intent admission. The
    # ordinary owner still fully decodes the original manifest before any work.
    return batch_id, tuple(effect_id)


def verify_batch(owner: Directory, manifest: Pin, effects_id: tuple[int, int]) -> None:
    manifest.verify()
    owner.names()
    effect = os.stat(EFFECTS, dir_fd=owner.fd, follow_symlinks=False)
    require(stat.S_ISDIR(effect.st_mode) and stat.S_IMODE(effect.st_mode) == 0o700
            and effect.st_uid == owner.uid and (effect.st_dev, effect.st_ino) == effects_id,
            'original effects directory custody changed')
    owner.verify()


def request_stop(root: Path, batch_id: str, budget: Budget) -> dict:
    """Publish/resync the same permanent stop record understood by existing owners.

    A torn publication remains in place and fences the batch; it is never repaired.
    The signal can race an admitted or dispatched native call, so success is NOT
    a native cancellation, a pause receipt, a terminal effect, or a drain proof.
    """
    import fcntl
    identity(batch_id)
    budget.check()
    with directory(root, budget) as owner:
        with pinned(owner, MANIFEST, ROOM_MAX_MANIFEST) as manifest:
            actual_id, effects_id = manifest_identity(manifest.raw, budget)
            require(actual_id == batch_id, 'stop request names a different original batch')
            expected = envelope({'format': STOP_FORMAT, 'batch_id': batch_id})
            require(len(expected) <= MAX_STOP, 'stop marker bound exceeded')
            verify_batch(owner, manifest, effects_id)
            flags = os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
            created = False
            try:
                fd = os.open(STOP, os.O_RDWR | os.O_CREAT | os.O_EXCL | flags, 0o600, dir_fd=owner.fd)
                created = True
            except FileExistsError:
                fd = os.open(STOP, os.O_RDONLY | flags, dir_fd=owner.fd)
            try:
                # Serialize stop publishers only; never wait for a native owner.
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                info = os.fstat(fd)
                require(stat.S_ISREG(info.st_mode) and stat.S_IMODE(info.st_mode) == 0o600
                        and info.st_uid == owner.uid and info.st_nlink == 1,
                        'stop marker must be a private single-link regular file')
                verify_batch(owner, manifest, effects_id)
                if created:
                    view = memoryview(expected)
                    while view:
                        budget.check()
                        count = os.write(fd, view)
                        require(count > 0, 'short stop write; retain partial marker')
                        view = view[count:]
                else:
                    require(1 <= info.st_size <= MAX_STOP, 'invalid stop marker; no repair')
                    raw = read_bytes(fd, info.st_size, budget)
                    require(signature(os.fstat(fd)) == signature(info) and raw == expected,
                            'conflicting or incomplete stop marker; no overwrite')
                stop_pin = Pin(owner, STOP, fd, expected)
                verify_batch(owner, manifest, effects_id)
                budget.check()
                os.fsync(fd)
                budget.check()
                os.fsync(owner.fd)
                stop_pin.verify()
                verify_batch(owner, manifest, effects_id)
            finally:
                os.close(fd)
            result = {'schema': SCHEMA, 'ok': True, 'batch_id': batch_id,
                'original_manifest_sha256': hashlib.sha256(manifest.raw).hexdigest(),
                'stop_record_sha256': hashlib.sha256(expected).hexdigest(),
                'future_step_admission_revoked': True, 'cooperative_only': True,
                'in_flight_effect_may_complete': True, 'native_calls': 0,
                'native_effects_cancelled': False, 'game_paused_proven': False,
                'native_quiescence_proven': False, 'retry_commit_permitted': False,
                'excavation_completion_proven': False, 'production_admitted': False}
    budget.check()
    return result


def publish(raw: bytes) -> bool:
    try:
        if sys.stdout.buffer.write(raw) != len(raw):
            return False
        sys.stdout.buffer.flush()
        return True
    except (OSError, ValueError, KeyboardInterrupt):
        return False


class Parser(argparse.ArgumentParser):
    def error(self, _message):
        raise ValueError('invalid stop-request arguments')


def main(argv=None) -> int:
    try:
        parser = Parser(description=__doc__, allow_abbrev=False)
        parser.add_argument('--directory', type=Path, required=True)
        parser.add_argument('--batch-id', required=True)
        parser.add_argument('--timeout-ms', type=int, default=10000)
        args = parser.parse_args(argv)
        budget = Budget(args.timeout_ms)
        result = request_stop(args.directory, args.batch_id, budget)
        raw = canonical(result) + b'\n'
        budget.check()
        code = 0
    except (ValueError, OSError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        raw = canonical({'schema': SCHEMA, 'ok': False, 'stop_request_verified': False,
            'error': 'STOP_REQUEST_REFUSED', 'native_calls': 0,
            'detail': 'Preserve the original batch and any stop marker. A failed acknowledgment does not prove no stop was recorded.',
            'native_quiescence_proven': False, 'retry_commit_permitted': False}) + b'\n'
        code = 2
    return code if publish(raw) else 2


if __name__ == '__main__':
    raise SystemExit(main())
