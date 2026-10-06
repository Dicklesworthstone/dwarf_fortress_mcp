#!/usr/bin/env python3
"""Bind an existing exact room allocation to its completed terrain journal.

One fresh read-only map capture, no inventory reallocation or game effects.
Use the existing isolated map/1.5 environment. Preserve the original journal
for later placement and completion custody. Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import os
import stat
import struct
import sys

from furniture_plan import canonical, require
from room_furniture_handoff import RoomFurnitureHandoff, MAX_BYTES as MAX_INPUT
from room_terrain_origin import TerrainOrigin
from terrain_furniture_handoff import TerrainFurnitureHandoff
import track_excavation as t

MAX_OUTPUT = 384 * 1024


def read_input(path: Path, budget: t.Budget) -> RoomFurnitureHandoff:
    require(1 <= len(str(path)) <= 4096, 'bounded room handoff input path required')
    budget.checkpoint()
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    try:
        before = os.fstat(fd)
        require(stat.S_ISREG(before.st_mode) and 1 <= before.st_size <= MAX_INPUT,
                'bounded regular room handoff required')
        raw = bytearray()
        while len(raw) <= before.st_size:
            budget.checkpoint()
            chunk = os.read(fd, before.st_size + 1 - len(raw))
            if not chunk:
                break
            raw += chunk
        stamp = lambda s: (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns)
        require(len(raw) == before.st_size and stamp(before) == stamp(os.fstat(fd))
                == stamp(os.stat(path, follow_symlinks=False)), 'room handoff input changed')
        return RoomFurnitureHandoff.decode(bytes(raw), budget.checkpoint)
    finally:
        os.close(fd)


def packet(handoff: TerrainFurnitureHandoff | None) -> dict:
    known = handoff is not None
    return {'ok': known, 'schema': 'dfmcp.terrain-furniture-binding-result/1',
        'handoff': handoff.json() if known else None,
        'handoff_digest': handoff.digest if known else None,
        'terrain_history_verified_this_call': known, 'native_map_observations': 1 if known else None,
        'game_mutations_dispatched': False, 'journal_written': False, 'production_admitted': False,
        **({} if known else {'error': 'Original room, allocation, terrain history, fresh map, authority or custody refused.'}),
        'agent_turn': {'schema': 'dfmcp.agent_turn/1', 'operation': 'room.bind_terrain', 'phase': 'verify',
            'session_id': None, 'turn_id': None, 'request_id': None, 'anchor': None,
            'continuity': {'status': 'stale' if known else 'indeterminate', 'basis': None,
                           'gap': 'sampled_endpoints_only', 'reset_reason': None},
            'profile': 'forensic', 'briefing': {'runtime_admitted': False, 'mutation_admissible': False,
                                              'room_completion_proven': False},
            'changes': [], 'attention': [], 'active_work': [], 'affordances': [], 'recommendations': [],
            'coverage': {'original_terrain_history': 'replayed' if known else 'unestablished',
                         'effect_inventory': 'not_queried', 'room_assignments': 'not_queried'},
            'budget': {'maximum_output_bytes': MAX_OUTPUT, 'maximum_native_calls': 4,
                       'maximum_native_bytes': t.e.MAX_WIRE, 'maximum_work_checks': t.MAX_WORK,
                       'token_count_measured': False},
            'references': [handoff.compact()] if known else [],
            'uncertainty': ['Historical terrain and one fresh map do not prove continuous or current safety.',
                'Independent profile generations cannot detect an unseen same-tick restore.',
                'Retained inventory is not a reservation; native placement must revalidate each original item.',
                'The original private terrain journal must be retained; hashes alone do not attest acquisition.']}}


def output(value: dict) -> bytes:
    raw = canonical(value)
    require(len(raw) <= MAX_OUTPUT, 'complete terrain binding report exceeds byte bound')
    return raw


def bind(journal_path: Path, handoff_path: Path, *, emit: str = 'handoff', timeout_ms=10000,
         _budget: t.Budget | None = None) -> bytes:
    require(emit in ('handoff', 'report'), 'unsupported terrain binding export')
    budget = _budget if _budget is not None else t.Budget(timeout_ms)
    with t.open_journal(journal_path, budget) as journal:
        origin = TerrainOrigin.from_journal(journal, budget.checkpoint)
        room = read_input(handoff_path, budget)
        origin.verify(journal, room.room_plan, budget.checkpoint)
        historical = origin.json()
        source = room.allocation.source
        require(source.tick >= historical['completed_tick'], 'allocation predates original terrain completion')
        room.allocation.validate_binding(historical['endpoint'], historical['source']['folder'],
            historical['source']['site'], historical['source']['manifest']['df_version'],
            historical['source']['manifest']['dfhack_version'])
        budget.bind_room_read(*t.environment(journal.history.endpoint))
        journal.verify()
        with t.RoomMapClient(budget, journal.history.goal.region) as client:
            fresh = client.observe()
        handoff = TerrainFurnitureHandoff(room, origin, fresh, checkpoint=budget.checkpoint)
        report = output(packet(handoff))  # Complete reservation, including narrow exports.
        raw = handoff.encode() if emit == 'handoff' else report
        origin.verify(journal, room.room_plan, budget.checkpoint)
        budget.checkpoint()
        return raw


class Parser(argparse.ArgumentParser):
    def error(self, message: str) -> None:
        raise ValueError('invalid terrain binding arguments')


def main(argv: list[str] | None = None) -> int:
    try:
        parser = Parser(description=__doc__)
        parser.add_argument('--journal', required=True, type=Path)
        parser.add_argument('--room-handoff', required=True, type=Path)
        parser.add_argument('--emit', choices=('handoff', 'report'), default='handoff')
        parser.add_argument('--timeout-ms', type=int, default=10000)
        args = parser.parse_args(argv)
        budget = t.Budget(args.timeout_ms)
        raw = bind(args.journal, args.room_handoff, emit=args.emit, _budget=budget)
        budget.checkpoint()
        code = 0
    except (OSError, ValueError, TypeError, KeyError, IndexError, RecursionError, struct.error):
        raw, code = output(packet(None)), 2
    try:
        if sys.stdout.buffer.write(raw) != len(raw):
            return 2
        sys.stdout.buffer.flush()
    except (OSError, ValueError):
        return 2
    return code


if __name__ == '__main__':
    raise SystemExit(main())
