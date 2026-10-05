#!/usr/bin/env python3
"""Survey a complete room intention through one existing read-only map capture.

No mutation, clock control, effect reconciliation, retry, background work or
journal creation. Use the isolated map/1.5 development environment only.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass, field
import hashlib
import os
from pathlib import Path
import stat
import struct
import sys
import time

import excavation_observer as e
import room_terrain as terrain
from furniture_plan import MAX_BYTES, canonical, require
from room_provisioning import MAX_PLAN_BYTES, RoomPlan

ENVIRONMENT = frozenset(('DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5', 'DFMCP_MAP_TOKEN', 'DFMCP_MAP_ENDPOINT'))
MAX_WORK = 250000


@dataclass(frozen=True)
class Authority:
    address: str
    token: bytes = field(repr=False)

    @classmethod
    def load(cls) -> Authority:
        require(all(not key.startswith('DFMCP_') or key in ENVIRONMENT for key in os.environ),
                'room survey accepts only the isolated map-read environment')
        require(os.environ.get('DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5') == '1',
                'explicit map-read development opt-in required')
        address = os.environ.get('DFMCP_MAP_ENDPOINT', '127.0.0.1:5000')
        e.endpoint(address)
        token = os.environ.get('DFMCP_MAP_TOKEN', '').encode('utf-8')
        require(32 <= len(token) <= 256 and b'\0' not in token, 'invalid map query credential')
        return cls(address, token)

    def guard(self) -> None:
        require(Authority.load() == self, 'operator room-survey configuration changed')


class Budget:
    """One shrinking wall/work/file/network/call allowance for the whole command."""
    def __init__(self, timeout_ms: int):
        self.deadline = time.monotonic() + e.integer(timeout_ms, 1, 60000) / 1000
        self.work_left = MAX_WORK
        self.calls_left = 4  # Two binds, handshake, one observation; no reconnect.
        self.network_left = e.MAX_WIRE
        self.file_left = MAX_PLAN_BYTES + 1  # Reserve one growth/EOF probe too.

    def remaining(self) -> float:
        remaining = self.deadline - time.monotonic()
        require(remaining > 0, 'whole room-survey deadline exhausted')
        return remaining

    def work(self) -> None:
        self.remaining()
        require(self.work_left > 0, 'whole room-survey work allowance exhausted')
        self.work_left -= 1

    def charge(self, name: str, count: int) -> None:
        self.remaining()
        require(name in ('calls_left', 'network_left', 'file_left'), 'unknown survey budget')
        e.integer(count, 0, e.MAX_WIRE)
        remaining = getattr(self, name)
        require(count <= remaining, 'whole room-survey allowance exhausted')
        setattr(self, name, remaining - count)


class SurveyClient(e.MapClient):
    """Narrow the unchanged native client with the command's outer guard/budget."""
    def __init__(self, authority: Authority, region: e.Region, budget: Budget):
        self.authority, self.budget = authority, budget
        authority.guard()
        budget.remaining()
        # The parent's timeout is an additional ceiling, never a renewed outer
        # allowance. Every connect/send/recv uses the minimum remaining duration.
        super().__init__(authority.address, authority.token, region, 60000)

    def remaining(self) -> float:
        self.authority.guard()
        return min(self.budget.remaining(), super().remaining())

    def charge(self, size: int) -> None:
        self.authority.guard()
        self.budget.charge('network_left', size)
        super().charge(size)

    def frame(self, method: int, request: bytes) -> bytes:
        self.authority.guard()
        self.budget.charge('calls_left', 1)
        return super().frame(method, request)


def read_input(path: Path, maximum: int, guard, budget: Budget) -> bytes:
    """Bounded operator input, not journal custody or all-parent no-follow access."""
    guard()
    require(os.name == 'posix' and hasattr(os, 'O_NOFOLLOW') and 1 <= len(str(path)) <= 4096,
            'bounded POSIX room input path required')
    fd = os.open(path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        before = os.fstat(fd)
        require(stat.S_ISREG(before.st_mode) and 1 <= before.st_size <= maximum,
                'room input must be a bounded nonempty regular file')
        budget.charge('file_left', before.st_size + 1)
        raw = bytearray()
        while len(raw) <= before.st_size:
            guard()
            chunk = os.read(fd, before.st_size + 1 - len(raw))
            if not chunk:
                break
            raw += chunk
        after = os.fstat(fd)
        named = os.stat(path, follow_symlinks=False)
        identity = lambda info: (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)
        require(len(raw) == before.st_size and identity(before) == identity(after) == identity(named),
                'room input changed during bounded read')
        guard()
        return bytes(raw)
    finally:
        os.close(fd)


def packet(result: dict | None, address: str | None = None) -> dict:
    established = result is not None
    if established:
        e.endpoint(address)
    status = result['status'] if established else 'unestablished'
    acquisition = ({'endpoint': address, 'profile': 'map/1.5',
                    'native_capture_established': True, 'native_observations': 1,
                    'survey_digest': result['survey_digest']} if established else None)
    return {
        'ok': established, 'profile': 'room-terrain-survey/1', 'runtime_admitted': False,
        'result': result, 'acquisition': acquisition, 'game_mutation_dispatched': False,
        **({} if established else {'error':
            'Room input, terrain read, source, authority or budget refused; no proposal established.'}),
        'agent_turn': {
            'schema': 'dfmcp.agent_turn/1', 'operation': 'room.survey', 'phase': 'observe',
            'session_id': None, 'turn_id': None, 'request_id': None, 'anchor': None,
            'continuity': {'status': 'bootstrap' if established else 'indeterminate',
                           'basis': None, 'gap': 'one_sample_only', 'reset_reason': None},
            'profile': 'tactical',
            'briefing': {'status': status, 'runtime_admitted': False, 'mutation_admissible': False,
                         'room_completion_proven': False, 'native_effect_inventory_verified': False},
            'changes': [], 'attention': [], 'active_work': [], 'affordances': [], 'recommendations': [],
            'uncertainty': [
                'A sampled remaining mask is not mining eligibility, a reservation or mutation authority.',
                'Native effects are not queried; this result cannot clear unknown work or authorize replacement keys.',
                'A residual is not the complete room goal; retain the original room plan for all later stages.',
                'Aquifers, pressure, wall material, support, native paths and current or continuous state are unknown.',
                'Map and dig identities are independent; exported artifacts do not attest acquisition by themselves.'],
            'coverage': {'terrain': 'complete_requested_capture' if established else 'unestablished',
                         'effect_inventory': 'not_queried', 'furniture_inventory': 'not_queried',
                         'room_assignments': 'not_queried'},
            'budget': {'maximum_output_bytes': terrain.MAX_OUTPUT, 'token_count_measured': False,
                       'maximum_work_checks': MAX_WORK, 'maximum_native_calls': 4,
                       'maximum_native_bytes': e.MAX_WIRE},
            'references': [{'survey_digest': result['survey_digest'],
                            'room_plan_digest': result['room_plan']['plan_digest'],
                            'observation_witness': result['source']['observation_witness']}] if established else [],
        },
    }


def serialize(value: dict) -> bytes:
    raw = canonical(value)
    require(len(raw) <= terrain.MAX_OUTPUT, 'complete room-survey envelope exceeds byte bound')
    return raw


def run(plan: RoomPlan, authority: Authority, budget: Budget, *, emit: str = 'survey') -> bytes:
    require(emit in ('survey', 'remaining-blueprint', 'excavation-handoff'), 'unsupported room survey export')
    def guard():
        authority.guard()
        budget.work()
    guard()
    selected = terrain.selection(plan, guard)
    with SurveyClient(authority, selected.region, budget) as client:
        observed = client.observe()
        guard()
        result = terrain.survey(selected.plan, observed.raw, observed.manifest, guard)
        value = packet(result, authority.address)
        value['report_digest'] = hashlib.sha256(b'dfmcp-room-terrain-report/1\0' + canonical(value)).hexdigest()
        output = serialize(value)  # Reserve the complete report even for a narrow export.
        if emit == 'remaining-blueprint':
            require(result['remaining_blueprint'] is not None,
                    'no complete nonempty excavation proposal to export')
            output = canonical(result['remaining_blueprint'])
        if emit == 'excavation-handoff':
            from room_excavation_handoff import RoomExcavationHandoff
            output = RoomExcavationHandoff.create(selected.plan, observed.raw, observed.manifest,
                                                  authority.address, guard).encode()
        guard()  # Cached results and final serialization cannot bypass revocation.
        return output


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument('--request-file', type=Path)
    source.add_argument('--plan-file', type=Path)
    parser.add_argument('--emit', choices=('survey', 'remaining-blueprint', 'excavation-handoff'), default='survey')
    parser.add_argument('--timeout-ms', type=int, default=10000)
    args = parser.parse_args(argv)
    try:
        budget = Budget(args.timeout_ms)
        authority = Authority.load()
        def guard():
            authority.guard()
            budget.work()
        path = args.plan_file or args.request_file
        raw = read_input(path, MAX_PLAN_BYTES if args.plan_file else MAX_BYTES, guard, budget)
        plan = (RoomPlan.decode(raw, guard) if args.plan_file else RoomPlan.from_request(raw, guard))
        output = run(plan, authority, budget, emit=args.emit)
        guard()
        code = 0
    except (ValueError, OSError, TypeError, KeyError, RecursionError, struct.error):
        output, code = serialize(packet(None)), 2
    try:
        if sys.stdout.buffer.write(output) != len(output):
            return 2
        sys.stdout.buffer.flush()
    except (OSError, ValueError):
        # Output failure never reconnects, rereads, or emits a second JSON object.
        return 2
    return code


if __name__ == '__main__':
    sys.exit(main())
