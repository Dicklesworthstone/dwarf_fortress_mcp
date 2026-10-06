#!/usr/bin/env python3
"""Compile room recipes or allocate their complete furniture set in one read.

Compilation is offline. Allocation uses ONLY the existing isolated inventory
profile. No game effects, automatic excavation, placement, or room assignments.
Beads: df-dfhack-bridge-plane-c-pic.3 / .4 / .5.
"""
from __future__ import annotations

import argparse
import os
import stat
import sys

import furniture_inventory as inventory
from furniture_handoff import Handoff
from furniture_plan import MAX_BYTES, canonical, require
from room_provisioning import MAX_PLAN_BYTES, RoomPlan
from room_furniture_handoff import RoomFurnitureHandoff

MAX_OUTPUT = 65536
PROFILE = 'room-provisioning/1'


def read_input(path: str, maximum: int, budget: inventory.Budget) -> bytes:
    """Bounded operator data, not a private journal or all-parent custody claim."""
    budget.remaining()
    require(os.name == 'posix' and hasattr(os, 'O_NOFOLLOW'), 'no-follow input requires POSIX')
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    try:
        before = os.fstat(fd)
        require(stat.S_ISREG(before.st_mode) and 1 <= before.st_size <= maximum,
                'input must be a bounded regular file')
        raw = bytearray()
        while len(raw) <= before.st_size:
            budget.remaining()
            chunk = os.read(fd, before.st_size + 1 - len(raw))
            budget.reserve('disk_bytes', len(chunk))
            if not chunk:
                break
            raw += chunk
        after = os.fstat(fd)
        named = os.stat(path, follow_symlinks=False)
        stamp = lambda info: (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)
        require(len(raw) == before.st_size and stamp(before) == stamp(after) == stamp(named),
                'input changed or was substituted during read')
        budget.remaining()
        return bytes(raw)
    finally:
        os.close(fd)


def encode_output(value: dict) -> bytes:
    raw = canonical(value) + b'\n'
    require(len(raw) <= MAX_OUTPUT, 'complete room allocation exceeds output allowance')
    return raw


def allocate_plan(plan: RoomPlan, authority: inventory.Authority, budget: inventory.Budget,
                  *, emit: str = 'report') -> bytes:
    """One native acquisition, retaining every room and every requested constraint."""
    require(type(plan) is RoomPlan, 'complete room recipe required')
    require(type(emit) is str and emit in ('report', 'room-handoff'), 'unsupported room allocation export')
    authority.guard()
    def guard() -> None:
        budget.work()
        authority.guard()
    # A directly constructed dataclass is not a validated artifact. Regenerate
    # every derived field before it can select even a read-only native request.
    plan = RoomPlan.decode(plan.encode(), guard)
    request = plan.request()
    # Use the existing capture owner and projection rather than another protocol
    # client. The complete inventory is decoded once and shared by allocation and
    # handoff derivation; no native read or candidate selection is repeated.
    with inventory.InventoryClient(authority, budget) as client:
        manifest, capture = client.capture_once()
    # The verified native capture is released and the connection closed before
    # CPU-bound allocation and composite encoding; authority/deadline stay live.
    address = f'{authority.address[0]}:{authority.address[1]}'
    result = inventory.project(request, capture, guard, handoff_binding=(address, manifest))
    require(result['request_digest'] == request.digest, 'allocation changed original room request')
    handoff = None
    if result['status'] == 'allocated':
        handoff = Handoff.from_json(result['handoff'])
        require(handoff.request == request and handoff.plan().json() == result['plan']
                and handoff.digest == result['handoff_digest'], 'allocation handoff lost room intent')
    else:
        require(result['status'] == 'shortage' and result['handoff'] is None
                and result['plan'] is None and result['assignments'] == [],
                'incomplete allocation must not return an executable subset')
    result['source'].update(native_generation=manifest.generation,
                            df_version=manifest.df_version, dfhack_version=manifest.dfhack_version)
    result['native_capture_established'] = True
    result['capture_release_verified'] = True
    result['room_provisioning'] = plan.summary()
    value = inventory.packet(result)
    value['profile'] = PROFILE
    turn = value['agent_turn']
    turn['operation'] = 'rooms.allocate'
    turn['briefing']['room_completion_proven'] = False
    turn['coverage'].update(room_geometry='complete_intended_recipe_only',
                            room_terrain='not_observed', native_room_assignments='not_created')
    turn['references'].append({'kind': 'room_provisioning_recipe', 'plan_digest': plan.digest,
                               'furniture_request_digest': request.digest})
    turn['uncertainty'].append(
        'Room geometry is an unobserved proposal. Excavation, placement and completion remain '
        'separate reviewed workflows; furniture completion alone does not establish completed rooms.')
    raw = encode_output(value)  # Reserve the complete diagnostic report even for a narrow export.
    if emit == 'room-handoff':
        require(handoff is not None, 'shortage cannot export a partial room handoff')
        raw = RoomFurnitureHandoff(plan, handoff, checkpoint=guard).encode()
    guard()
    authority.guard()
    budget.remaining()
    return raw


def failure(operation: str) -> bytes:
    """Do not disclose cached source facts, caller paths, native text or secrets."""
    value = inventory.packet(None, 'RoomProvisioningRefused')
    value['profile'] = PROFILE
    value['detail'] = 'Room request, source, authority, input stability or budget refused. No room allocation established.'
    value['agent_turn']['operation'] = 'rooms.' + operation
    value['agent_turn']['briefing']['room_completion_proven'] = False
    return encode_output(value)


class Parser(argparse.ArgumentParser):
    def error(self, message: str) -> None:
        raise ValueError('invalid room-provisioning arguments')


def main(argv: list[str] | None = None) -> int:
    operation = 'unknown'
    authority = None
    status = 0
    try:
        parser = Parser(description=__doc__)
        commands = parser.add_subparsers(dest='operation', required=True)
        compile_command = commands.add_parser('compile')
        compile_command.add_argument('--request-file', required=True)
        compile_command.add_argument('--emit', choices=('plan', 'excavation', 'furniture-request'), default='plan')
        compile_command.add_argument('--timeout-ms', type=int, default=10000)
        allocate_command = commands.add_parser('allocate')
        source = allocate_command.add_mutually_exclusive_group(required=True)
        source.add_argument('--request-file')
        source.add_argument('--plan-file')
        allocate_command.add_argument('--timeout-ms', type=int, default=10000)
        allocate_command.add_argument('--emit', choices=('report', 'room-handoff'), default='report')
        args = parser.parse_args(argv)
        operation = args.operation
        budget = inventory.Budget(args.timeout_ms)
        imported = operation == 'allocate' and args.plan_file is not None
        raw = read_input(args.plan_file if imported else args.request_file,
                         MAX_PLAN_BYTES if imported else MAX_BYTES, budget)
        plan = RoomPlan.decode(raw, budget.work) if imported else RoomPlan.from_request(raw, budget.work)
        if operation == 'allocate':
            authority = inventory.Authority.load()
            raw = allocate_plan(plan, authority, budget, emit=args.emit)
        elif args.emit == 'plan':
            raw = plan.encode()  # Canonical artifact imports intentionally require no trailing newline.
        else:
            key = 'excavation_blueprint' if args.emit == 'excavation' else 'furniture_request'
            raw = canonical(plan.json()[key])
        if authority is not None:
            authority.guard()
        budget.remaining()
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        raw, status = failure(operation), 2
    try:
        if sys.stdout.buffer.write(raw) != len(raw):
            return 2
        sys.stdout.buffer.flush()
    except (OSError, ValueError):
        return 2
    return status


if __name__ == '__main__':
    raise SystemExit(main())
