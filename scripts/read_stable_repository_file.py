#!/usr/bin/env python3
"""Read one bounded regular file through stable no-follow descriptor custody."""

from __future__ import annotations

import hashlib
import os
import stat
from dataclasses import dataclass
from pathlib import Path
from typing import NoReturn


class StableReadError(ValueError):
    pass


@dataclass(frozen=True)
class StableFile:
    path: Path
    content: bytes
    sha256: str
    size: int
    device: int
    inode: int
    mode: int
    owner_uid: int


def fail(message: str) -> NoReturn:
    raise StableReadError(message)


def same_identity(left: os.stat_result, right: os.stat_result) -> bool:
    return (
        left.st_dev == right.st_dev
        and left.st_ino == right.st_ino
        and left.st_size == right.st_size
        and left.st_mode == right.st_mode
        and left.st_uid == right.st_uid
        and left.st_gid == right.st_gid
        and left.st_mtime_ns == right.st_mtime_ns
        and left.st_ctime_ns == right.st_ctime_ns
    )


def read_stable_regular_file(
    path: Path,
    maximum_bytes: int,
    label: str,
    *,
    allow_empty: bool = False,
) -> StableFile:
    if isinstance(maximum_bytes, bool) or maximum_bytes <= 0:
        fail("maximum_bytes must be a positive integer")
    raw = os.fspath(path)
    if not raw or len(os.fsencode(raw)) > 4096:
        fail(f"{label} path is empty or exceeds its byte bound")
    if any(ord(character) < 0x20 for character in raw):
        fail(f"{label} path contains a control character")
    if not hasattr(os, "O_NOFOLLOW"):
        fail("this platform cannot enforce no-follow file opening")

    try:
        before = os.lstat(path)
    except OSError as exc:
        fail(f"cannot inspect {label}: {exc}")
    if stat.S_ISLNK(before.st_mode) or not stat.S_ISREG(before.st_mode):
        fail(f"{label} must be a regular non-symbolic-link file")
    minimum = 0 if allow_empty else 1
    if before.st_size < minimum or before.st_size > maximum_bytes:
        fail(f"{label} must contain {minimum}..={maximum_bytes} bytes")

    flags = os.O_RDONLY | os.O_NOFOLLOW
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    try:
        descriptor = os.open(path, flags)
    except OSError as exc:
        fail(f"cannot open {label} without following symbolic links: {exc}")
    try:
        opened = os.fstat(descriptor)
        if not same_identity(before, opened):
            fail(f"{label} changed between path inspection and open")
        digest = hashlib.sha256()
        chunks: list[bytes] = []
        total = 0
        while True:
            remaining = maximum_bytes + 1 - total
            if remaining <= 0:
                fail(f"{label} exceeded its byte bound while being read")
            chunk = os.read(descriptor, min(1024 * 1024, remaining))
            if not chunk:
                break
            total += len(chunk)
            if total > maximum_bytes:
                fail(f"{label} exceeded its byte bound while being read")
            digest.update(chunk)
            chunks.append(chunk)
        after = os.fstat(descriptor)
        if not same_identity(opened, after) or total != after.st_size:
            fail(f"{label} changed while being read")
        if total < minimum:
            fail(f"{label} became empty while being read")
        return StableFile(
            path=path,
            content=b"".join(chunks),
            sha256=digest.hexdigest(),
            size=total,
            device=after.st_dev,
            inode=after.st_ino,
            mode=stat.S_IMODE(after.st_mode),
            owner_uid=after.st_uid,
        )
    finally:
        os.close(descriptor)
