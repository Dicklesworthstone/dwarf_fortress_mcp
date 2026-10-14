"""Recheck an immutable remaining delivery order on one captured map.

This pure boundary cannot authenticate a placed prefix or current item facts.
Its execution owner must first verify original receipts, source and selectors.
No allocation, reordering, native call, custody change or permission is created.
Beads: df-dfhack-bridge-plane-c-pic.4/.5; df-franken-storage-mvcc-54h.4.
"""
from __future__ import annotations

import hashlib

from furniture_allocation import Guard, MAX_ID, idle
from furniture_plan import FurniturePlan, MAX_STEPS, Step, canonical, integer, label, require
import route_furniture_allocation as routes
from route_furniture_sequence import MODEL, _path, _witness

POLICY = 'original-fixed-suffix-after-placed-target-removal/1'
SCHEMA = 'dfmcp.furniture-delivery-suffix/1'


def assess_remaining(plan: FurniturePlan, placed_prefix: tuple[str, ...],
                     positions: dict[int, tuple[int, int, int]], terrain_raw: bytes,
                     guard: Guard = idle, *, max_route_steps: int = routes.MAX_ROUTE_STEPS,
                     maximum_work: int = routes.MAX_WORK) -> dict:
    """Verify every remaining delivery in the ORIGINAL canonical order.

    `placed_prefix` must be the owner's complete receipt-verified prefix. Only
    its shape is checked here; knowing these names never grants effect authority.
    `positions` must cover exactly the remaining original items after qualified
    full-inventory/selector checks, not replacement candidates. Even a currently
    open predecessor target remains removed in this conservative model.

    One allowance spans validation, decoding, graph construction, all BFSs and
    full-route commitments. Blockage or unknown endpoints yields NO partial
    route certificate. Failure says nothing about another order or assignment.
    """
    work = routes.Work(maximum_work, guard)
    integer(max_route_steps, 0, routes.MAX_ROUTE_STEPS)
    require(type(plan) is FurniturePlan and type(plan.steps) is tuple
            and 1 <= len(plan.steps) <= MAX_STEPS, 'invalid original delivery plan')
    # Validate without mutating the supplied frozen records or trusting cached
    # canonical order. Bounds precede copying and all graph allocations.
    checked = []
    for step in plan.steps:
        work()
        require(type(step) is Step, 'invalid original delivery step')
        checked.append(Step(step.name, step.kind, step.item, step.target, step.after))
    fixed = FurniturePlan(tuple(checked))
    require(fixed.steps == plan.steps and fixed.ordered == plan.ordered,
            'original delivery plan is not canonical')
    require(type(placed_prefix) is tuple and len(placed_prefix) < len(fixed.steps),
            'delivery suffix requires an unfinished exact prefix')
    for name in placed_prefix:
        work()
        label(name)
    prefix_steps = fixed.ordered[:len(placed_prefix)]
    require(placed_prefix == tuple(step.name for step in prefix_steps),
            'delivery prefix omits, reorders or substitutes original work')
    remaining = fixed.ordered[len(placed_prefix):]
    require(type(positions) is dict and len(positions) == len(remaining),
            'positions must cover exactly the remaining original items')
    for identity, position in positions.items():
        work()
        integer(identity, 0, MAX_ID)
        require(type(position) is tuple and len(position) == 3, 'invalid current item position')
        for coordinate in position:
            integer(coordinate, 0, 32767)
    require(set(positions) == {step.item for step in remaining},
            'remaining delivery item missing or substituted')
    terrain = routes.decode_terrain(terrain_raw, work)
    require(terrain.paused, 'delivery suffix requires a paused capture')
    fixed.check_dimensions(terrain.dimensions)
    targets = {}
    for step in fixed.steps:
        work()
        index = terrain.index(step.target)
        require(index is not None, 'original delivery target outside captured region')
        targets[step.name] = index
    endpoints, unavailable = [], []
    for step in remaining:
        work()
        position = positions[step.item]
        endpoints.append({'slot': step.name, 'item': step.item, 'position': list(position)})
        for role, point in (('source', position), ('target', step.target)):
            work()
            reason = terrain.reason(point)
            if reason != 'candidate':
                unavailable.append({'slot': step.name, 'endpoint': role, 'reason': reason})
    status, blocked_slot, witnesses = 'unestablished', None, []
    if not unavailable:
        graph = terrain.adjacency(work)
        blocked = {targets[step.name] for step in prefix_steps}
        done = set(placed_prefix)
        status = 'ready'
        for step in remaining:
            work.check()
            work()
            require(set(step.after) <= done, 'original delivery dependency not satisfied')
            source, target = terrain.index(positions[step.item]), targets[step.name]
            found = _path(graph, source, target, blocked, max_route_steps, work)
            if found is None:
                status, blocked_slot, witnesses = 'blocked', step.name, []
                break
            length, parent = found
            witnesses.append({'slot': step.name, 'item': step.item,
                **_witness(terrain, parent, source, target, length, blocked, max_route_steps, work)})
            blocked.add(target)
            done.add(step.name)
    work()
    result = {'schema': SCHEMA, 'policy': POLICY, 'model': MODEL,
              'path_policy': routes.PATH_POLICY, 'status': status,
              'original_plan_digest': fixed.digest, 'terrain_sha256': terrain.digest,
              'placed_prefix': list(placed_prefix),
              'remaining_order': [step.name for step in remaining], 'positions': endpoints,
              'max_route_steps': max_route_steps, 'first_blocked_slot': blocked_slot,
              'unestablished_endpoints': unavailable, 'routes': witnesses,
              'total_delivery_route_steps': sum(row['route_steps'] for row in witnesses)
                  if status == 'ready' else None,
              'scope': 'original_fixed_suffix_in_captured_target_removal_model',
              'candidate_model_sequence_proven': status == 'ready',
              'prefix_receipts_authenticated': False, 'item_selectors_verified': False,
              'reordering_performed': False, 'reallocation_performed': False,
              'native_pathfinding_proven': False, 'future_routes_guaranteed': False,
              'placement_authorized': False, 'game_effect_performed': False}
    # A paired execution seal must not change solely because a later check has
    # less remaining computational allowance. All semantic inputs stay bound.
    result['evidence_digest'] = hashlib.sha256(
        b'dfmcp-furniture-delivery-suffix/1\0' + canonical(result)).hexdigest()
    result.update(work_units=work.used + 1, maximum_work=maximum_work)
    require(len(canonical(result)) <= routes.MAX_OUTPUT, 'complete delivery suffix exceeds output bound')
    work()
    work.check()
    return result
