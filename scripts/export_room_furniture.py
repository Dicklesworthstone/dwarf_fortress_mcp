#!/usr/bin/env python3
"""Export original furnishings after whole-room completion and a fresh map check.

Read-only development bridge into the existing furniture-request/1 consumer.
Never allocate items, prepare/commit effects, renew a goal, or repair a journal.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5 (WP-05/WP-10; remain open).
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import struct
import sys
from typing import Callable

SCHEMA = 'dfmcp.room-furniture-export/1'
POLICY = 'dfmcp.completed-room-fresh-terrain-handoff/1'
MAX_OUTPUT = 262144
MAX_REQUEST = 16384


def _require(ok: bool, message: str) -> None:
    if not ok:
        raise ValueError(message)


def _canonical(value: object) -> bytes:
    # The existing furniture-request/1 canonical encoding, including ASCII
    # escaping. No new request schema, digest domain or normalization is used.
    return json.dumps(value, sort_keys=True, separators=(',', ':'),
                      ensure_ascii=True, allow_nan=False).encode('ascii')


def _verify_transition(history, observed, diagnosis: dict, request,
                       guard: Callable[[], None]) -> int:
    """Additional stage guards AFTER trusted journal replay and raw decoding.

    This private helper does not authenticate caller-created History/Capture
    objects. Only export() supplies inputs to the publishing path.
    """
    guard()
    p, goal = history.progress, history.goal
    _require(p.status == 'satisfied' and history.pending_read is False,
             'whole original room terrain goal must already be satisfied')
    _require(p.streak >= goal.required_samples and p.since_tick is not None
             and p.latest.tick - p.since_tick >= goal.stable_ticks,
             'retained room stability evidence is incomplete')
    _require(observed.binding() == p.first.binding() == p.latest.binding(),
             'fresh map differs from the retained room source')
    gap = observed.tick - p.latest.tick
    _require(0 <= gap <= goal.max_gap_ticks,
             'fresh map clock regressed or completion-to-export gap exceeded')
    _require(diagnosis['all_required_shapes_at_sample'] is True
             and diagnosis['deficits']['count'] == 0
             and diagnosis['observation_witness'] == observed.witness
             and diagnosis['observed_tick'] == observed.tick
             and diagnosis['room_plan_digest'] == goal.room_plan.digest,
             'every original floor and required wall must still hold together')
    # Whole-room terrain permits occupied floors; actual furnishing targets
    # must additionally be visibly dry, undesignated, and free of buildings
    # and units. This remains only a map observation, not placement eligibility.
    ox, oy, oz = goal.region.origin
    sx, sy, sz = goal.region.size
    _require(1 <= len(request.slots) <= 32, 'complete original furniture request required')
    for slot in request.slots:
        guard()
        x, y, z = slot.target
        _require(ox <= x < ox + sx and oy <= y < oy + sy and oz <= z < oz + sz,
                 'original furnishing target lies outside the coherent map capture')
        tile = observed.tiles[((z - oz) * sy + y - oy) * sx + x - ox]
        _require(tile.presence == 2, 'furnishing target is hidden or missing')
        _, shape, depth, magma, _, dig, building, units, *_ = tile.attributes
        _require(shape == 3 and not (depth or magma or dig or building or units),
                 'furnishing target is not a clear dry undesignated floor')
    guard()
    return gap


def _select_output(value: dict, emit: str, guard: Callable[[], None]) -> bytes:
    _require(emit in ('report', 'request'), 'unsupported room furniture export')
    guard()
    report = _canonical(value)
    _require(len(report) <= MAX_OUTPUT, 'complete room furniture report exceeds byte bound')
    # Reserve the COMPLETE evidence report even when exporting only the request.
    request = _canonical(value['furniture_request'])
    _require(1 <= len(request) <= MAX_REQUEST, 'original furniture request exceeds byte bound')
    guard()
    return request if emit == 'request' else report


def _turn(reference: dict | None) -> dict:
    known = reference is not None
    return {
        'schema': 'dfmcp.agent_turn/1', 'operation': 'room.furniture_export', 'phase': 'verify',
        'session_id': None, 'turn_id': None, 'request_id': None, 'anchor': None,
        'continuity': {'status': 'stale' if known else 'indeterminate',
                       'basis': None, 'gap': 'sampled_endpoints_only', 'reset_reason': None},
        'profile': 'tactical',
        'briefing': {'runtime_admitted': False, 'mutation_admissible': False,
                     'room_completion_proven': False, 'current_terrain_proven': False},
        'changes': [], 'attention': [], 'active_work': [], 'affordances': [], 'recommendations': [],
        'uncertainty': [
            'Fresh sampled terrain is not current or continuously stable terrain.',
            'Item availability, paths, placement eligibility and room assignments are not observed.',
            'Native effect inventory is not reconciled; no replacement or retry keys are authorized.'],
        'coverage': {'retained_room_goal': 'replayed' if known else 'unestablished',
                     'terrain': 'one_complete_original_capture' if known else 'unestablished',
                     'furniture_inventory': 'not_queried', 'native_effect_inventory': 'not_queried'},
        'budget': {'maximum_output_bytes': MAX_OUTPUT, 'maximum_request_bytes': MAX_REQUEST,
                   'maximum_native_calls': 4, 'token_count_measured': False},
        'references': [reference] if known else [],
    }


def refused() -> bytes:
    # Do not expose credentials, filesystem paths or unverified partial slots.
    return _canonical({
        'ok': False, 'schema': SCHEMA, 'policy': POLICY,
        'error': 'Whole-room history, fresh terrain, source, custody, authority or budget refused the export.',
        'game_mutations_dispatched': False, 'journal_written': False,
        'retry_designation_permitted': False, 'production_admitted': False,
        'agent_turn': _turn(None),
    })


def export(path: Path, *, emit: str = 'report', timeout_ms: int = 10000, _budget=None) -> bytes:
    # Import the existing owner only on the execution path. Pure format/stage
    # guard tests need no native transport or writable journal implementation.
    import room_terrain_goal as r
    import track_excavation as t

    _require(emit in ('report', 'request'), 'unsupported room furniture export')
    budget = _budget if _budget is not None else t.Budget(timeout_ms)
    with t.open_journal(path, budget) as journal:
        history = journal.history  # Whole bounded history was replayed under the lock.
        _require(history.profile is t.ROOM_PROFILE,
                 'floor or residual blueprint completion cannot replace the original room goal')
        _require(history.progress.status == 'satisfied' and not history.pending_read,
                 'original room terrain goal is not satisfied')
        goal = history.goal.validated(budget.checkpoint)
        request = goal.room_plan.request()
        # No source override and no network access for an incomplete/wrong goal.
        budget.bind_room_read(*t.environment(history.endpoint))
        journal.verify()
        budget.checkpoint()
        with t.RoomMapClient(budget, goal.region) as client:
            capture = client.observe()
        # End native connection custody before local re-decoding/projection.
        observed = r.decode(goal, capture, budget.checkpoint)
        diagnosis = r.diagnose(goal, observed, guard=budget.checkpoint)
        gap = _verify_transition(history, observed, diagnosis, request, budget.checkpoint)
        reference = {'journal_id': history.identity, 'journal_head': history.head,
                     'room_goal_digest': goal.digest, 'room_plan_digest': goal.room_plan.digest,
                     'furniture_request_digest': request.digest,
                     'observation_witness': observed.witness, 'observed_game_tick': observed.tick}
        value = {
            'ok': True, 'schema': SCHEMA, 'policy': POLICY,
            'origin': {**reference, 'endpoint': history.endpoint, 'journal_events': history.events,
                       'completion_tick': history.progress.latest.tick,
                       'completion_observation_witness': history.progress.latest.witness,
                       'matching_samples': history.progress.streak,
                       'matching_since_tick': history.progress.since_tick},
            'room_goal': goal.json(), 'furniture_request': request.json(),
            'source_profile': 'map/1.5', 'native_observations': 1,
            'source': observed.binding(), 'fresh_sample': t.sample_value(observed),
            'terrain_at_export_sample': diagnosis, 'completion_to_export_gap_ticks': gap,
            'original_room_goal_satisfied_at_retained_sample': True,
            'all_furniture_target_tiles_clear_at_export_sample': True,
            'complete_original_room_intent_retained': True, 'residual_goal_substitution_permitted': False,
            'furniture_placement_eligibility_proven': False, 'current_conditions_proven': False,
            'continuous_wall_preservation_proven': False, 'room_completion_proven': False,
            'construction_completion_proven': False, 'room_assignments_observed': False,
            'items_allocated': False, 'native_effect_inventory_verified': False,
            'game_mutations_dispatched': False, 'journal_written': False,
            'retry_designation_permitted': False, 'production_admitted': False,
            'agent_turn': _turn(reference),
        }
        value['agent_turn']['budget'].update(maximum_work_checks=t.MAX_WORK,
                                              maximum_native_bytes=t.e.MAX_WIRE)
        budget.checkpoint()
        value['export_digest'] = hashlib.sha256(
            b'dfmcp-room-furniture-export/1\0' + _canonical(value)).hexdigest()
        output = _select_output(value, emit, budget.checkpoint)
        journal.verify()  # Byte/path custody must survive the read and serialization.
        budget.checkpoint()
        return output


def main(argv: list[str] | None = None) -> int:
    import track_excavation as t

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--journal', type=Path, required=True)
    parser.add_argument('--emit', choices=('report', 'request'), default='report')
    parser.add_argument('--timeout-ms', type=int, default=10000)
    args = parser.parse_args(argv)
    try:
        budget = t.Budget(args.timeout_ms)
        output = export(args.journal, emit=args.emit, _budget=budget)
        budget.checkpoint()  # Recheck original authority/deadline before publication.
        code = 0
    except (ValueError, OSError, TypeError, KeyError, IndexError, RecursionError, struct.error):
        output, code = refused(), 2
    try:
        if sys.stdout.buffer.write(output) != len(output):
            return 2
        sys.stdout.buffer.flush()
    except (OSError, ValueError):
        return 2  # Never reconnect, mutate, or emit a second object on output failure.
    return code


if __name__ == '__main__':
    sys.exit(main())
