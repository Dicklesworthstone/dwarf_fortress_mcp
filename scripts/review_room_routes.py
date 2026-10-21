#!/usr/bin/env python3
"""Create/check original room-route plans; bind/inspect complete artifacts offline.

No placement, excavation, file writes, clock changes or automatic batch start.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5; dfmcp-wp-pln01-spatial-blueprint-layout-pwd.
"""
from __future__ import annotations

import argparse
import os
import stat
import sys
import time

from furniture_plan import canonical, integer, require
from room_provisioning import RoomPlan, MAX_PLAN_BYTES
import room_route_handoff as model
import room_route_workflow as flow
import route_furniture_review as reviews


class OfflineBudget:
    """Bound only offline input/codec work; live operations use the existing budget."""
    def __init__(self, timeout_ms):
        integer(timeout_ms, 1, 60000)
        self.deadline = time.monotonic() + timeout_ms / 1000
        self.work_steps, self.disk_bytes = 20_000_000, 2 * 1024 * 1024

    def remaining(self):
        value = self.deadline - time.monotonic()
        require(value > 0, 'room-route foreground time exhausted')
        return value

    def reserve(self, name, amount):
        self.remaining()
        require(name in ('work_steps', 'disk_bytes') and type(amount) is int
                and 0 <= amount <= getattr(self, name), 'room-route budget exhausted')
        setattr(self, name, getattr(self, name) - amount)

    def work(self):
        self.reserve('work_steps', 1)


def open_live(timeout_ms):
    from route_furniture_inventory import Authority, wire
    return wire.Budget(timeout_ms), Authority.load()


def read_file(path, maximum, budget):
    require(type(path) is str and '\0' not in path and 1 <= len(path.encode()) <= 4096,
            'invalid room-route input path')
    budget.remaining()
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    try:
        before = os.fstat(fd)
        require(stat.S_ISREG(before.st_mode) and 1 <= before.st_size <= maximum,
                'room-route input must be a bounded regular file')
        raw = bytearray()
        while len(raw) <= before.st_size:
            budget.remaining()
            data = os.read(fd, min(4096, before.st_size + 1 - len(raw)))
            budget.reserve('disk_bytes', len(data))
            if not data:
                break
            raw += data
        stamp = lambda v: (v.st_dev, v.st_ino, v.st_mode, v.st_size, v.st_mtime_ns, v.st_ctime_ns)
        require(len(raw) == before.st_size and stamp(before) == stamp(os.fstat(fd)) ==
                stamp(os.stat(path, follow_symlinks=False)), 'room-route input changed during read')
        budget.remaining()
        return bytes(raw)
    finally:
        os.close(fd)


class Parser(argparse.ArgumentParser):
    def error(self, _message):
        raise ValueError('invalid room-route arguments')


def publish(raw):
    try:
        if sys.stdout.buffer.write(raw) != len(raw):
            return 2
        sys.stdout.buffer.flush()
        return 0
    except (OSError, ValueError, KeyboardInterrupt):
        return 2


def main(argv=None, *, backend=open_live, acquire=flow.acquire_native):
    operation, authority = 'unknown', None
    try:
        parser = Parser(description=__doc__, allow_abbrev=False)
        commands = parser.add_subparsers(dest='operation', required=True)
        for name in ('bind', 'inspect', 'create', 'check'):
            p = commands.add_parser(name, allow_abbrev=False)
            p.add_argument('--timeout-ms', type=int, default=10000)
            if name in ('bind', 'create'):
                p.add_argument('--room-plan-file', required=True)
                p.add_argument('--expected-room-plan', required=True)
            else:
                p.add_argument('--handoff-file', required=True)
                p.add_argument('--expected-handoff', required=True)
            if name == 'bind':
                p.add_argument('--review-file', required=True)
                p.add_argument('--expected-review', required=True)
            if name == 'create':
                p.add_argument('--origin', nargs=3, type=int, required=True)
                p.add_argument('--size', nargs=3, type=int, required=True)
                p.add_argument('--max-route-steps', type=int, default=flow.routes.MAX_ROUTE_STEPS)
            if name in ('bind', 'create', 'inspect'):
                p.add_argument('--emit', choices=('report', 'handoff'), default='report')
        args = parser.parse_args(argv)
        operation = args.operation
        integer(args.timeout_ms, 1, 60000)
        if operation in ('create', 'check'):
            budget, authority = backend(args.timeout_ms)
        else:
            budget = OfflineBudget(args.timeout_ms)
        if operation in ('bind', 'create'):
            reviews.digest_text(args.expected_room_plan)
            room = RoomPlan.decode(read_file(args.room_plan_file, MAX_PLAN_BYTES, budget), budget.work)
            require(room.digest == args.expected_room_plan, 'selected original room identity differs')
        else:
            handoff = model.RoomRouteHandoff.decode(read_file(args.handoff_file, model.MAX_BYTES, budget),
                                                   args.expected_handoff, budget.work)
        if operation == 'bind':
            review = reviews.Review.decode(read_file(args.review_file, reviews.MAX_BYTES, budget),
                                            args.expected_review, budget.work)
            handoff = model.RoomRouteHandoff.create(room, review, budget.work)
        if operation in ('bind', 'inspect'):
            room, review = handoff.unpack(budget.work)
            raw = (handoff.encode(budget.work) if args.emit == 'handoff' else flow.encode(flow.packet(
                operation, {'status': 'historical_intent_only', 'handoff_digest': handoff.digest,
                    'room_plan_digest': room.digest, 'route_review_digest': review.digest,
                    'plan': None, 'native_calls': 0, 'room_completion_proven': False}, handoff.json(budget.work))))
        elif operation == 'create':
            raw = flow.create(room, tuple(args.origin), tuple(args.size), authority, budget,
                              max_route_steps=args.max_route_steps, acquire=acquire)
            if args.emit == 'handoff':
                import json
                packet = json.loads(raw)
                require(packet['result']['status'] == 'ready_in_captured_model'
                        and packet['handoff'] is not None, 'narrow export requires the whole room-route condition')
                raw = canonical(packet['handoff'])
        else:
            raw = flow.check(handoff, authority, budget, acquire=acquire)
        budget.remaining()
        if authority is not None:
            authority.guard()
            budget.remaining()
        status = 0
    except (OSError, ValueError, TypeError, KeyError, RecursionError, ImportError, KeyboardInterrupt):
        raw, status = flow.encode(flow.packet(operation)), 2
    # Once output starts there is no fallback object and no repeated acquisition.
    return status if publish(raw) == 0 else 2


if __name__ == '__main__':
    raise SystemExit(main())
