#!/usr/bin/env python3
"""Propose distinct furniture using one read-only map/inventory bracket.

Does not create files, prepare/commit placements, reserve items, or advance time.
Use the existing furniture-request/1 input and an explicit bounded terrain region.
"""
from __future__ import annotations

import argparse
import os
import stat
import sys

from construction_monitor_rpc import Budget
from furniture_allocation import Request
from furniture_plan import MAX_BYTES, require
from route_furniture_allocation import MAX_ROUTE_STEPS
from route_furniture_inventory import Authority, Region, bounded_output, packet, run


class Parser(argparse.ArgumentParser):
    def error(self, message: str) -> None:
        raise ValueError('invalid route furniture arguments')


def read_request(path: str, budget: Budget) -> Request:
    budget.remaining()
    require(os.name == 'posix' and hasattr(os, 'O_NOFOLLOW'), 'no-follow input required')
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK)
    try:
        before = os.fstat(fd)
        require(stat.S_ISREG(before.st_mode) and 1 <= before.st_size <= MAX_BYTES,
                'request must be a bounded regular file')
        raw = bytearray()
        while len(raw) <= before.st_size:
            budget.remaining()
            value = os.read(fd, before.st_size + 1 - len(raw))
            budget.reserve('disk_bytes', len(value))
            if not value:
                break
            raw += value
        after = os.fstat(fd)
        named = os.stat(path, follow_symlinks=False)
        stamp = lambda s: (s.st_dev, s.st_ino, s.st_mode, s.st_size, s.st_mtime_ns, s.st_ctime_ns)
        require(len(raw) == before.st_size and stamp(before) == stamp(after) == stamp(named),
                'request changed during read')
        result = Request.decode(bytes(raw))
        budget.remaining()
        return result
    finally:
        os.close(fd)


def publish(raw: bytes) -> int:
    """Partial stdout never triggers a second object or another native read."""
    try:
        if sys.stdout.buffer.write(raw) != len(raw):
            return 2
        sys.stdout.buffer.flush()
    except (OSError, ValueError, KeyboardInterrupt):
        return 2
    return 0


def main(argv: list[str] | None = None) -> int:
    try:
        parser = Parser(description=__doc__)
        parser.add_argument('--request-file', required=True)
        parser.add_argument('--origin', nargs=3, type=int, required=True)
        parser.add_argument('--size', nargs=3, type=int, required=True)
        parser.add_argument('--max-route-steps', type=int, default=MAX_ROUTE_STEPS)
        parser.add_argument('--sequence-deliveries', action='store_true',
                            help='strengthen dependencies to keep all selected deliveries feasible in the captured route model')
        parser.add_argument('--timeout-ms', type=int, default=10000)
        args = parser.parse_args(argv)
        budget = Budget(args.timeout_ms)
        region = Region(tuple(args.origin), tuple(args.size))
        request = read_request(args.request_file, budget)
        authority = Authority.load()
        options = {'max_route_steps': args.max_route_steps}
        if args.sequence_deliveries:
            options['sequence_deliveries'] = True
        output = run(request, region, authority, budget, **options)
        budget.remaining()
        authority.guard()
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        # Do not expose native strings, request paths, tokens or partial selections.
        publish(bounded_output(packet(None)))
        return 2
    return publish(output)


if __name__ == '__main__':
    raise SystemExit(main())
