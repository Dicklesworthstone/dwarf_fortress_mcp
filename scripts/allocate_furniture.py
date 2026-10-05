#!/usr/bin/env python3
"""Propose an exact furniture batch from one complete native inventory read.

Explicitly unadmitted, read-only operations/1.4 workflow. Emits bounded JSON to
stdout; never writes a plan file, reserves an item, prepares or places furniture.
"""
from __future__ import annotations

import argparse
import os
import stat
import sys

from furniture_allocation import Request
from furniture_inventory import Authority, Budget, bounded_output, packet, run
from furniture_plan import MAX_BYTES, require


def read_request(path: str, budget: Budget) -> Request:
    """Bounded regular file read; do not follow the final pathname symlink."""
    budget.remaining()
    require(os.name == 'posix' and hasattr(os, 'O_NOFOLLOW'), 'request input requires POSIX no-follow open')
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK)
    try:
        before = os.fstat(fd)
        require(stat.S_ISREG(before.st_mode) and 1 <= before.st_size <= MAX_BYTES,
                'request must be a bounded regular file')
        raw = bytearray()
        while len(raw) <= before.st_size:
            budget.remaining()
            part = os.read(fd, before.st_size + 1 - len(raw))
            budget.reserve('disk_bytes', len(part))
            if not part:
                break
            raw += part
        after = os.fstat(fd)
        stamp = lambda info: (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)
        require(len(raw) == before.st_size and stamp(before) == stamp(after), 'request changed during read')
        result = Request.decode(bytes(raw))
        budget.remaining()
        return result
    finally:
        os.close(fd)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--request-file', required=True)
    parser.add_argument('--with-handoff', action='store_true',
                        help='retain the complete original request and selected-item evidence for batch initialization')
    parser.add_argument('--timeout-ms', type=int, default=10000)
    args = parser.parse_args(argv)
    try:
        budget = Budget(args.timeout_ms)
        request = read_request(args.request_file, budget)
        if args.with_handoff:
            output = run(request, Authority.load(), budget, retain_constraints=True)
        else:
            output = run(request, Authority.load(), budget)
        status = 0
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt) as error:
        # No native strings, paths, credentials or partial assignments in errors.
        output = bounded_output(packet(None, type(error).__name__))
        status = 2
    try:
        sys.stdout.buffer.write(output)
        sys.stdout.buffer.flush()
    except OSError:
        return 2
    return status


if __name__ == '__main__':
    raise SystemExit(main())
