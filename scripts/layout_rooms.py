#!/usr/bin/env python3
"""Lay out whole room projects and export existing provisioning artifacts.

This command is offline: inputs and exclusions are planning assumptions. Native
survey, allocation, review, execution and completion retain their own authority.
Beads: dfmcp-wp-pln01-spatial-blueprint-layout-pwd; df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

import argparse
import os
import stat
import sys
import time

from furniture_plan import canonical, integer, require
from room_layout import Layout, MAX_INPUT, MAX_OUTPUT, WorkLimit, solve

MAX_COMMAND_OUTPUT = 128 * 1024
EXPORTS = ('layout', 'report', 'room-plan', 'room-request', 'excavation', 'furniture-request')


class DeadlineExceeded(ValueError):
    pass


class Deadline:
    def __init__(self, milliseconds: int):
        integer(milliseconds, 1, 60_000)
        self.end = time.monotonic() + milliseconds / 1000

    def check(self) -> None:
        if time.monotonic() >= self.end:
            raise DeadlineExceeded('room layout foreground deadline reached')


def read_input(path: str, maximum: int, deadline: Deadline) -> bytes:
    """Stable bounded operator input, not private journal/all-parent custody."""
    deadline.check()
    require(os.name == 'posix' and hasattr(os, 'O_NOFOLLOW'), 'layout input requires POSIX no-follow access')
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    try:
        before = os.fstat(descriptor)
        require(stat.S_ISREG(before.st_mode) and 1 <= before.st_size <= maximum,
                'layout input must be a bounded regular file')
        raw = bytearray()
        while len(raw) <= before.st_size:
            deadline.check()
            chunk = os.read(descriptor, min(65_536, before.st_size + 1 - len(raw)))
            if not chunk:
                break
            raw.extend(chunk)
        after = os.fstat(descriptor)
        named = os.stat(path, follow_symlinks=False)
        def stamp(info):
            return (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)
        require(len(raw) == before.st_size and stamp(before) == stamp(after) == stamp(named),
                'layout input changed during acquisition')
        deadline.check()
        return bytes(raw)
    finally:
        os.close(descriptor)


def report(result: dict | None, code: str | None = None) -> dict:
    """An offline planning packet cannot claim a canonical fortress observation."""
    found = result is not None and result['status'] == 'layout_found'
    plan = result['room_plan'] if found else None
    status = None if result is None else result['status']
    reference = [] if result is None else [{'kind': 'room_layout', 'digest': result['layout_digest'],
                                           'epistemic_status': 'assumed_geometry', 'canonical_anchor': False}]
    next_steps = [] if not found else [{
        'kind': 'review_selected_room_plan', 'room_plan_digest': plan['plan_digest'],
        'native_survey_required': True, 'mutation_authority_granted': False,
        'automatic_execution': False}]
    return {'schema': 'dfmcp.room-layout-command/1', 'ok': code is None,
            'result': result, 'error': None if code is None else {
                'code': code, 'detail': 'Layout request, bounded search, stable input or output refused. No partial room plan was exported.'},
            'agent_turn': {'schema': 'dfmcp.agent_turn/1', 'operation': 'rooms.layout',
                'phase': 'propose' if found else 'inspect', 'session_id': None, 'request_id': None,
                'anchor': None, 'continuity': {'status': 'bootstrap', 'basis': None, 'gap': None, 'reset_reason': None},
                'briefing': {'layout_status': status, 'planned_areas': 0 if plan is None else len(plan['areas']),
                    'planned_furnishings': 0 if plan is None else plan['furniture_count'],
                    'runtime_admitted': False, 'mutation_admissible': False, 'room_completion_proven': False},
                'changes': [], 'attention': [], 'active_work': [], 'affordances': [],
                'recommendations': next_steps,
                'uncertainty': [
                    'The domain, exclusions and focus are caller-declared planning constraints, not observed terrain or an existing fortress entrance.',
                    'A full layout is not globally optimized. No native pathfinding, safety, item availability, construction or room assignment is established.',
                    'Exported geometry requires the existing native survey, allocation and explicit effect-review workflows. No stage is started automatically.'],
                'coverage': {'domain': 'complete_requested_offline_room_layout',
                    'original_request_retained': result is not None, 'native_observations': 0,
                    'game_effects': 0, 'global_infeasibility_proven': False},
                'budget': {'output_bytes_limit': MAX_COMMAND_OUTPUT, 'token_count_measured': False},
                'references': reference}}


def export(result: Layout, kind: str) -> bytes:
    """Called only on solve/decode results; never a raw unchecked imported object."""
    require(kind in EXPORTS, 'unknown layout export')
    value = result.json()
    if kind == 'layout':
        raw = result.encode()
    elif kind == 'report':
        raw = canonical(report(value)) + b'\n'
    else:
        require(value['status'] == 'layout_found', 'a complete layout is required for artifact export')
        plan = value['room_plan']
        # Emit the EXACT existing artifact, with no wrapper or trailing newline.
        # Its origin choices are frozen; downstream consumers must not re-layout.
        selected = plan if kind == 'room-plan' else plan[{
            'room-request': 'intent', 'excavation': 'excavation_blueprint',
            'furniture-request': 'furniture_request'}[kind]]
        raw = canonical(selected)
    require(len(raw) <= MAX_COMMAND_OUTPUT, 'complete layout command output exceeds bound')
    return raw


def publish(raw: bytes) -> int:
    try:
        if sys.stdout.buffer.write(raw) != len(raw):
            return 2
        sys.stdout.buffer.flush()
    except (OSError, ValueError, TypeError, KeyboardInterrupt):
        return 2
    return 0


class Parser(argparse.ArgumentParser):
    def error(self, message: str) -> None:
        raise ValueError('invalid room-layout arguments')


def main(argv: list[str] | None = None) -> int:
    code = None
    try:
        parser = Parser(description=__doc__, allow_abbrev=False)
        source = parser.add_mutually_exclusive_group(required=True)
        source.add_argument('--request-file')
        source.add_argument('--layout-file', help='recompute and verify a complete canonical layout artifact')
        parser.add_argument('--emit', choices=EXPORTS, default='layout')
        parser.add_argument('--timeout-ms', type=int, default=10000)
        args = parser.parse_args(argv)
        deadline = Deadline(args.timeout_ms)
        raw = read_input(args.layout_file if args.layout_file is not None else args.request_file,
                         MAX_OUTPUT if args.layout_file is not None else MAX_INPUT, deadline)
        result = (Layout.decode(raw, deadline.check) if args.layout_file is not None
                  else solve(raw, deadline.check))
        if args.emit not in ('layout', 'report') and result.json()['status'] != 'layout_found':
            code = ('ROOM_LAYOUT_SEARCH_LIMIT' if result.json()['status'] == 'search_limit_reached'
                    else 'ROOM_LAYOUT_NOT_FOUND')
        else:
            raw = export(result, args.emit)
            deadline.check()
    except DeadlineExceeded:
        code = 'ROOM_LAYOUT_TIMEOUT'
    except WorkLimit:
        code = 'ROOM_LAYOUT_WORK_LIMIT'
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        code = 'ROOM_LAYOUT_REFUSED'
    if code is not None:
        publish(canonical(report(None, code)) + b'\n')
        return 2
    # A short write is terminal. Never emit a second object or repeat the search.
    return publish(raw)


if __name__ == '__main__':
    raise SystemExit(main())
