"""One read acquisition for complete original rooms and route-aware furnishings.

The injected acquisition port is trusted I/O, not caller-supplied JSON. The
ordinary implementation delegates to the existing strict route-review reader.
No placement, batch write, native method, new credential or retry is introduced.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5; dfmcp-wp-pln01-spatial-blueprint-layout-pwd.
"""
from __future__ import annotations

from typing import Callable

from furniture_plan import canonical, integer, require
from room_provisioning import RoomPlan
import room_route_handoff as model
import route_furniture_allocation as routes
import route_furniture_fixed_order as fixed
import route_furniture_review as reviews

MAX_OUTPUT = 256 * 1024
EXPORT_WORK = 100_000
SCHEMA = 'dfmcp.room-route-workflow/1'


def acquire_native(request, origin, size, authority, budget, original=None):
    """Reuse one complete map/inventory bracket, closed before graph analysis."""
    from route_furniture_inventory import Region
    from route_furniture_review_runtime import acquire
    return acquire(request, Region(origin, size), authority, budget, original)


def _guard(authority, budget) -> None:
    budget.remaining()
    authority.guard()
    budget.remaining()


def _acquire(request, origin, size, authority, budget, original, acquire):
    _guard(authority, budget)
    if original is not None:
        require(original.endpoint == f'{authority.address[0]}:{authority.address[1]}'
                and (original.origin, original.size) == (origin, size),
                'current endpoint or region differs from original room-route scope')
    raw, source, candidates, types, counts = acquire(
        request, origin, size, authority, budget, original)
    _guard(authority, budget)
    require(type(source) is reviews.Source, 'closed room-route source required')
    source.__post_init__()
    require(source.endpoint == f'{authority.address[0]}:{authority.address[1]}'
            and (source.origin, source.size) == (origin, size), 'acquisition substituted room-route scope')
    if original is not None:
        original.permits(source)
    terrain = routes.decode_terrain(raw, budget.work)
    require((terrain.digest, terrain.folder, terrain.site, terrain.tick, terrain.paused,
             terrain.dimensions, terrain.origin, terrain.size) ==
            (source.map_sha256, request.folder, request.site, source.tick, True,
             source.dimensions, origin, size), 'room-route map differs from selected source')
    return raw, source, candidates, types, counts


def _reserve(authority, budget) -> routes.Work:
    allowance = min(routes.MAX_WORK, budget.work_steps - EXPORT_WORK)
    require(allowance > 0, 'no room-route solve allowance after export reservation')
    budget.work_steps -= allowance
    return routes.Work(allowance, lambda: _guard(authority, budget))


def packet(operation: str, result=None, handoff=None) -> dict:
    success = result is not None
    return {'schema': SCHEMA, 'operation': operation, 'ok': success,
        'result': result, 'handoff': handoff,
        'error': None if success else {'code': 'ROOM_ROUTE_REFUSED',
            'detail': 'Original intent, source, input, authority or budget refused. No room-route result established.'},
        'agent_turn': {'schema': 'dfmcp.agent_turn/1', 'operation': 'rooms.route.' + operation,
            'phase': 'propose' if operation in ('create', 'bind') else 'inspect',
            'session_id': None, 'turn_id': None, 'request_id': None, 'anchor': None,
            'continuity': {'status': 'indeterminate', 'basis': None, 'gap': None},
            'profile': 'tactical', 'briefing': {'runtime_admitted': False,
                'mutation_admissible': False, 'room_completion_proven': False,
                'status': result['status'] if success else 'unestablished'},
            'changes': [], 'attention': [], 'active_work': [], 'affordances': [],
            'recommendations': [], 'references': [] if not success else [
                {'kind': 'original_room_plan', 'digest': result['room_plan_digest'],
                 'canonical_world_anchor': False},
                {'kind': 'original_room_route_handoff', 'digest': result.get('handoff_digest'),
                 'canonical_world_anchor': False},
                *([{'kind': 'current_room_route_source', **result['source']}]
                  if result.get('source') is not None else [])],
            'coverage': {'complete_original_intent': success,
                'fresh_read_requested': operation in ('create', 'check'),
                'active_effects': 'not_queried', 'canonical_world_anchor': False},
            'uncertainty': [
                'Room intent, original items and full delivery order must all match; nested allocation alone is not readiness.',
                'Equal paused map endpoints bracket inventory, not an atomic cross-profile snapshot.',
                'This unstarted-plan diagnostic grants no placement or room-assignment authority and is not a native commit gate.',
                'No current room usability, continuous stability, native hauling or effect reconciliation is proved.'],
            'budget': {'maximum_output_bytes': MAX_OUTPUT, 'token_count_measured': False}}}


def encode(value: dict) -> bytes:
    raw = canonical(value) + b'\n'
    require(len(raw) <= MAX_OUTPUT, 'complete room-route response exceeds output reservation')
    return raw


def _finish(operation, room, allocation, assessment, source, handoff, work, authority, budget):
    _guard(authority, budget)
    result = {'status': assessment['status'] if assessment is not None else allocation['status'],
        'room_plan': room.json(), 'room_plan_digest': room.digest,
        'handoff_digest': handoff.digest if handoff is not None else None,
        'allocation': allocation, 'assessment': assessment,
        'plan': assessment['plan'] if assessment is not None else None,
        'source': source.json(), 'structural_work_units': work.used,
        'structural_work_reserved': work.maximum,
        'original_request_unchanged': True, 'room_completion_proven': False,
        'placement_gate_enforced': False, 'game_effect_performed': False,
        'native_acquisition_independently_attested': False}
    # Export an original artifact only for create. Check returns its identity and
    # complete diagnostics, never a newly allocated or silently replaced review.
    artifact = handoff.json(budget.work) if operation == 'create' and handoff is not None else None
    output = encode(packet(operation, result, artifact))
    _guard(authority, budget)  # Serialization cannot bypass current authority.
    return output


def create(room: RoomPlan, origin: tuple[int, int, int], size: tuple[int, int, int],
           authority, budget, *, max_route_steps: int = routes.MAX_ROUTE_STEPS,
           acquire: Callable = acquire_native) -> bytes:
    """Select NEW items in the original order, then retain and assess the whole room.

    A complete handoff may retain blocked/unknown intent for later rechecking;
    only result.plan is conditional on combined readiness. Shortage has no handoff.
    """
    integer(max_route_steps, 0, routes.MAX_ROUTE_STEPS)
    require(budget.work_steps > EXPORT_WORK, 'reserve complete room-route export before contact')
    room = model.check_region(room, origin, size, budget.work)
    request = room.request()
    raw, source, candidates, types, _ = _acquire(
        request, origin, size, authority, budget, None, acquire)
    work = _reserve(authority, budget)
    effective, allocation = fixed.allocate_fixed_order_routes(request, candidates, raw, work.check,
        max_route_steps=max_route_steps, maximum_work=work.maximum)
    work.used = allocation['work_units']
    require(canonical(effective.json()) == canonical(request.json()), 'allocator changed original room request')
    assessment, handoff = None, None
    if allocation['status'] == 'allocated':
        selected_types = {}
        for row in allocation['assignments']:
            work()
            selected_types[row['item']] = types[row['item']]
        review = reviews.create(request, allocation, source, selected_types, work)
        handoff = model.RoomRouteHandoff.create(room, review, work)
        allowance = work.maximum - work.used
        require(allowance > 0, 'no complete room check allowance remains')
        assessment = model.revalidate(handoff, candidates, types, raw, source,
                                      work.check, maximum_work=allowance)
        work.used += assessment['work_units']
    work.check()
    return _finish('create', room, allocation, assessment, source, handoff, work, authority, budget)


def check(handoff: model.RoomRouteHandoff, authority, budget,
          *, acquire: Callable = acquire_native) -> bytes:
    """Refresh exactly the retained region/items/room; never rerun allocation."""
    require(type(handoff) is model.RoomRouteHandoff, 'complete original room-route handoff required')
    require(budget.work_steps > EXPORT_WORK, 'reserve complete room check export before contact')
    room, review = handoff.unpack(budget.work)
    _, request, original, _ = review.unpack(budget.work)
    raw, source, candidates, types, _ = _acquire(
        request, original.origin, original.size, authority, budget, original, acquire)
    work = _reserve(authority, budget)
    assessment = model.revalidate(handoff, candidates, types, raw, source,
                                  work.check, maximum_work=work.maximum)
    work.used = assessment['work_units']
    return _finish('check', room, None, assessment, source, handoff, work, authority, budget)
