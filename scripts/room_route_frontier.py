"""Require the whole original room at an authenticated placement frontier.

The caller owns route/prefix acquisition and private batch custody. This final
pure gate adds full room terrain on the SAME raw map; it cannot turn an unready
route or unfinished predecessor into ready, grant authority, or retry an effect.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

import hashlib

from furniture_allocation import Guard, idle
from furniture_plan import canonical, integer, require
from room_route_handoff import RoomRouteHandoff, _terrain_report
from route_furniture_allocation import MAX_WORK, Work, decode_terrain
from route_furniture_review import Source, digest_text

INPUT_POLICY = 'original-route-delivery-and-quiescent-construction-prefix/3'
POLICY = 'original-room-route-and-quiescent-construction-prefix/4'
DOMAIN = b'dfmcp-route-furniture-frontier/1\0'
MAX_INPUT = 160 * 1024
MAX_OUTPUT = 192 * 1024


def _bound(value: object, work: Work) -> None:
    """Bound the owner's entire report before hashing or copying nested values."""
    pending, nodes, size = [(value, 0)], 0, 0
    while pending:
        node, depth = pending.pop()
        work()
        nodes += 1
        require(nodes <= 65536 and depth <= 16, 'room frontier nesting or nodes exceed bound')
        if type(node) is dict:
            require(nodes + len(pending) + len(node) <= 65536, 'room frontier node bound exceeded')
            for key, child in node.items():
                require(type(key) is str and len(key) <= 128, 'invalid room frontier field')
                size += len(key.encode('utf-8'))
                pending.append((child, depth + 1))
        elif type(node) is list:
            require(nodes + len(pending) + len(node) <= 65536, 'room frontier node bound exceeded')
            pending.extend((child, depth + 1) for child in node)
        elif type(node) is str:
            require(len(node) <= MAX_INPUT and '\0' not in node, 'invalid room frontier string')
            size += len(node.encode('utf-8'))
        elif type(node) is int:
            require(-(2**63) <= node < 2**64, 'room frontier integer exceeds bound')
        else:
            require(node is None or type(node) is bool, 'invalid room frontier value')
        require(size <= MAX_INPUT, 'room frontier text exceeds bound')
    require(len(canonical(value)) <= MAX_INPUT, 'complete room frontier input exceeds bound')


def gate_frontier(handoff: RoomRouteHandoff, frontier: dict, terrain_raw: bytes,
                  guard: Guard = idle, *, maximum_work: int | None = None) -> dict:
    """Narrow an already verified route/construction frontier with room terrain.

    The input is the execution owner's complete construction_frontier result,
    not client-issued evidence. All original receipts and remaining deliveries
    have already been checked. Reuse its work counter and bind the exact parent,
    original selections and source before checking EVERY required room cell.
    A digest is a comparison identity, not an acquisition or authority issuer.
    """
    guard()
    require(type(handoff) is RoomRouteHandoff and type(frontier) is dict,
            'complete original room and verified execution frontier required')
    if maximum_work is None:
        maximum_work = frontier.get('maximum_work')
    work = Work(maximum_work, guard)
    work.used = integer(frontier.get('work_units'), 1, maximum_work)
    require(maximum_work <= integer(frontier.get('maximum_work'), work.used, MAX_WORK),
            'room gate cannot renew the original work allowance')
    _bound(frontier, work)
    require(frontier.get('schema') == 'dfmcp.route-furniture-frontier/1'
            and frontier.get('policy') == INPUT_POLICY
            and frontier.get('status') in ('ready', 'route_blocked', 'route_unestablished',
                'construction_pending', 'construction_blocked', 'construction_unestablished')
            and frontier.get('all_remaining_selections_checked') is True
            and frontier.get('prefix_matches_route_frontier') is True,
            'room gate requires the complete original route and construction checks')
    unsigned = {key: value for key, value in frontier.items()
                if key not in ('work_units', 'maximum_work', 'evidence_digest')}
    require(frontier.get('evidence_digest') == hashlib.sha256(DOMAIN + canonical(unsigned)).hexdigest(),
            'execution frontier changed before room validation')
    room, review = handoff.unpack(work)
    original, request, baseline, plan = review.unpack(work)
    require(frontier.get('review_digest') == review.digest
            and frontier.get('original_plan_digest') == plan.digest
            and frontier.get('original_plan') == plan.json()
            and frontier.get('max_route_steps') == original['max_route_steps'],
            'execution frontier differs from the retained room selections')
    digest_text(frontier.get('batch_id'))
    prefix = frontier.get('placed_prefix')
    require(type(prefix) is list and len(prefix) < len(plan.ordered),
            'room frontier requires an unfinished original prefix')
    require(all(type(row) is dict and row.get('slot') == step.name
                for row, step in zip(prefix, plan.ordered))
            and frontier.get('next_step') == plan.ordered[len(prefix)].name,
            'room frontier changed the original delivery prefix')
    source = Source.from_json(frontier.get('source'))
    baseline.permits(source)
    terrain = decode_terrain(terrain_raw, work)
    require((terrain.digest, terrain.tick, terrain.paused, terrain.folder, terrain.site,
             terrain.dimensions, terrain.origin, terrain.size) ==
            (source.map_sha256, source.tick, True, request.folder, request.site,
             source.dimensions, source.origin, source.size),
            'room terrain is not the exact paused map used by the execution frontier')
    geometry = _terrain_report(room, terrain_raw, work)
    out = dict(frontier)
    out.update(policy=POLICY, room_handoff_digest=handoff.digest, room_plan_digest=room.digest,
               execution_frontier_status=frontier['status'],
               execution_frontier_evidence_digest=frontier['evidence_digest'],
               room_terrain=geometry, room_constraints_checked_this_call=True,
               room_completion_proven=False, atomic_room_placement_proven=False)
    # Preserve route/construction blockers even when room geometry is satisfied.
    # Missing original walls are UNKNOWN, not an empty or already-finished goal.
    if frontier['status'] == 'ready' and geometry['status'] != 'matched':
        out['status'] = 'room_blocked' if geometry['status'] == 'contradicted' else 'room_unestablished'
    work()
    unsigned = {key: value for key, value in out.items()
                if key not in ('work_units', 'maximum_work', 'evidence_digest')}
    out['evidence_digest'] = hashlib.sha256(DOMAIN + canonical(unsigned)).hexdigest()
    out.update(work_units=work.used, maximum_work=maximum_work)
    require(len(canonical(out)) <= MAX_OUTPUT, 'complete room execution frontier exceeds bound')
    work.check()
    return out
