#!/usr/bin/env python3
"""Create a retained route-furniture review or recheck all its original selections.

Read-only map/inventory workflow. Emit one bounded JSON object; never write a
plan, replace a review, start a placement or silently choose replacement items.
"""
from __future__ import annotations

import argparse
import os
import stat
import sys

from furniture_allocation import Request
from furniture_plan import MAX_BYTES as MAX_REQUEST_BYTES, integer, require
import route_furniture_inventory as inventory
import route_furniture_review as model
import route_furniture_review_runtime as runtime


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError('invalid route-review arguments')


def read_file(path, maximum, budget):
    """Read immutable input bytes; this is not private-journal custody."""
    budget.remaining()
    require(type(path) is str and '\0' not in path and 1 <= len(path.encode('utf-8')) <= 4096,
            'invalid input path')
    require(os.name == 'posix' and hasattr(os, 'O_NOFOLLOW'), 'POSIX no-follow input required')
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    try:
        before = os.fstat(fd)
        require(stat.S_ISREG(before.st_mode) and 1 <= before.st_size <= maximum,
                'review input must be a bounded regular file')
        data = bytearray()
        while len(data) <= before.st_size:
            budget.remaining()
            part = os.read(fd, min(4096, before.st_size + 1 - len(data)))
            budget.reserve('disk_bytes', len(part))
            if not part:
                break
            data.extend(part)
        stamp = lambda value: (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns)
        require(len(data) == before.st_size and stamp(before) == stamp(os.fstat(fd))
                == stamp(os.stat(path, follow_symlinks=False)), 'input file changed or was replaced')
        budget.remaining()
        return bytes(data)
    finally:
        os.close(fd)


def publish(raw):
    try:
        if sys.stdout.buffer.write(raw) != len(raw):
            return 2
        sys.stdout.buffer.flush()
        return 0
    except (OSError, ValueError, KeyboardInterrupt):
        return 2


def main(argv=None):
    operation = 'unknown'
    try:
        parser = Parser(description=__doc__)
        parser.add_argument('operation', choices=('create', 'check'))
        parser.add_argument('--request-file')
        parser.add_argument('--origin', type=int, nargs=3)
        parser.add_argument('--size', type=int, nargs=3)
        parser.add_argument('--max-route-steps', type=int)
        parser.add_argument('--sequence-deliveries', action='store_true',
                            help='create only: preserve selected items and add dependencies to avoid modeled route blockage')
        parser.add_argument('--search-delivery-items', action='store_true',
                            help='create with sequencing only: search alternative items before sealing a new review')
        parser.add_argument('--delivery-search-nodes', type=int,
                            help='alternative-item search bound, 1..4096; never a proof when exhausted')
        parser.add_argument('--review-file')
        parser.add_argument('--expected-review')
        parser.add_argument('--timeout-ms', type=int, default=10000)
        args = parser.parse_args(argv)
        operation = args.operation
        budget = inventory.wire.Budget(args.timeout_ms)
        creating = operation == 'create'
        require(all((value is not None) == creating for value in (args.request_file, args.origin, args.size)),
                'only create selects request and region')
        require(all((value is not None) != creating for value in (args.review_file, args.expected_review)),
                'check requires the retained artifact and independent digest')
        require(creating or args.max_route_steps is None, 'check cannot change the original route limit')
        require(creating or not args.sequence_deliveries, 'check cannot reorder an original review')
        require(not args.search_delivery_items or (creating and args.sequence_deliveries),
                'alternative items require new sequenced review creation')
        require(args.delivery_search_nodes is None or args.search_delivery_items,
                'node bound requires alternative-item search')
        if args.delivery_search_nodes is not None:
            integer(args.delivery_search_nodes, 1, runtime.joint.MAX_SEARCH_NODES)
        if creating:
            request = Request.decode(read_file(args.request_file, MAX_REQUEST_BYTES, budget))
            region = inventory.Region(tuple(args.origin), tuple(args.size))
            maximum = inventory.routes.MAX_ROUTE_STEPS if args.max_route_steps is None else args.max_route_steps
            options = {'max_route_steps': maximum}
            if args.sequence_deliveries:
                options['sequence_deliveries'] = True
            if args.search_delivery_items:
                options['search_delivery_items'] = True
                if args.delivery_search_nodes is not None:
                    options['max_search_nodes'] = args.delivery_search_nodes
            raw = runtime.create(request, region, inventory.Authority.load(), budget, **options)
        else:
            model.digest_text(args.expected_review)
            review = model.Review.decode(read_file(args.review_file, model.MAX_BYTES, budget),
                                         args.expected_review, budget.work)
            raw = runtime.check(review, inventory.Authority.load(), budget)
        status = 0
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        raw = inventory.bounded_output(runtime.packet(operation=operation))
        status = 2
    return status if publish(raw) == 0 else 2


if __name__ == '__main__':
    raise SystemExit(main())
