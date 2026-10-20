"""Read a bounded stop signal without making it authority or repairing it.

Only original-key inspection/recovery uses this owner. All manifest, registry,
intent and receipt checks stay with the existing dig batch. An opaque marker
still fences every new designation; its bytes and inode remain pinned.
Beads: df-dfhack-bridge-plane-c-pic.4/.5;
df-cancellation-progress-certificates-vvw.
"""
from __future__ import annotations

from contextlib import contextmanager
import hashlib
import os
import stat

import dig_designation_client as d

NAME = 'stopped.json'
MAX_BYTES = 1024


@contextmanager
def open_stop(directory, batch_id: str):
    """Yield the original pin and optional unverified-stop disclosure.

    An exact canonical marker needs no special disclosure, preserving legacy
    inventories. No JSON parser runs over opaque bytes. Invalid file custody,
    excessive size, replacement or active writer ownership still refuses.
    """
    import fcntl
    d.exact_hex(batch_id, 32)
    directory.verify()
    fd = os.open(NAME, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC,
                 dir_fd=directory.parent)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        before = os.fstat(fd)
        d.require(stat.S_ISREG(before.st_mode) and stat.S_IMODE(before.st_mode) == 0o600
                  and before.st_nlink == 1 and before.st_uid == os.fstat(directory.parent).st_uid
                  and 0 <= before.st_size <= MAX_BYTES, 'invalid original stop file custody or bound')
        raw = bytearray()
        while len(raw) <= before.st_size:
            directory.budget()
            part = os.read(fd, before.st_size + 1 - len(raw))
            if not part:
                break
            raw += part
        raw = bytes(raw)
        after = os.fstat(fd)
        d.require(len(raw) == before.st_size and
                  (before.st_mtime_ns, before.st_ctime_ns, before.st_size) ==
                  (after.st_mtime_ns, after.st_ctime_ns, after.st_size),
                  'original stop file changed during read')
        value = {'format': 'dfmcp.dig-blueprint-stop/1', 'batch_id': batch_id}
        expected = d.canonical({'value': value,
            'sha256': hashlib.sha256(d.canonical(value)).hexdigest()}) + b'\n'
        pin = d.Capsule(directory.root / NAME, fd, directory.parent, raw, {})
        pin.verify(); directory.verify()
        evidence = None if raw == expected else {
            'schema': 'dfmcp.dig-blueprint-stop-evidence/1', 'verification': 'unverified',
            'marker_sha256': hashlib.sha256(raw).hexdigest(), 'marker_bytes': len(raw),
            'file_identity': list(pin.identity), 'future_steps_blocked': True,
            'repair_performed': False, 'native_effects_cancelled': False,
            'native_quiescence_proven': False}
        yield pin, evidence
        pin.verify(); directory.verify()
    finally:
        os.close(fd)
