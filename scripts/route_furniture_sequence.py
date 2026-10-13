"""Compile delivery-safe furniture order in the existing captured route model.

This is opt-in planning, not native pathfinding or a future-route guarantee.
An earlier placement conservatively removes its target from the captured graph.
Selected items and every original constraint are preserved; only dependencies
may be added. Nothing is prepared, placed, reserved, retried or read natively.
Beads: df-franken-storage-mvcc-54h.4; df-dfhack-bridge-plane-c-pic.3/.4.
"""
from __future__ import annotations

from collections import deque
from dataclasses import replace
import hashlib
import struct

from furniture_allocation import Candidate, Guard, Request, idle
from furniture_plan import FurniturePlan, Step, canonical, require
import route_furniture_allocation as routes

POLICY = 'preserve-canonical-else-reverse-feasible-sinks/1'
MODEL = 'captured-route-graph-delete-placed-targets/1'
SCOPE = 'fixed_selected_items_with_original_dependencies_and_route_limit'


def _path(graph: tuple[tuple[int, ...], ...], source: int, target: int,
          blocked: set[int], limit: int, work: routes.Work) -> tuple[int, dict[int, int]] | None:
    """Bounded target-rooted BFS, with the existing stable tile-index tie break.

    Sparse parents avoid allocating a whole-map array on every sink probe. The
    source must remain accessible too: a prior target can cover an unhauled item.
    """
    work()
    if source in blocked or target in blocked:
        return None
    parent = {target: target}
    queue = deque([(target, 0)])
    while queue:
        current, distance = queue.popleft()
        work()
        if current == source:
            return distance, parent
        if distance == limit:
            continue
        for following in graph[current]:
            work()
            if following not in blocked and following not in parent:
                parent[following] = current
                queue.append((following, distance + 1))
    return None


def _witness(terrain: routes.Terrain, parent: dict[int, int], source: int, target: int,
             distance: int, blocked: set[int], limit: int, work: routes.Work) -> dict:
    prefix = hashlib.sha256(b'dfmcp-furniture-blocked-targets/1\0')
    prefix.update(struct.pack('>I', len(blocked)))
    for index in sorted(blocked):
        work()
        prefix.update(struct.pack('>I', index))
    digest = hashlib.sha256(b'dfmcp-furniture-delivery-route/1\0')
    digest.update(MODEL.encode('ascii') + b'\0' + routes.PATH_POLICY.encode('ascii') + b'\0')
    digest.update(bytes.fromhex(terrain.digest) + prefix.digest())
    digest.update(struct.pack('>IIII', source, target, distance + 1, limit))
    current = source
    for offset in range(distance + 1):
        work()
        require(current not in blocked and 0 <= current < len(terrain.shapes),
                'delivery witness crossed a blocked target')
        digest.update(struct.pack('>I', current))
        if offset == distance:
            require(current == target, 'delivery witness missed target')
        else:
            require(current != target and current in parent, 'delivery witness ended early')
            current = parent[current]
    return {'route_steps': distance, 'blocked_target_count': len(blocked),
            'blocked_targets_sha256': prefix.hexdigest(), 'route_sha256': digest.hexdigest()}


def _replay(order: tuple[str, ...], plan: FurniturePlan, terrain: routes.Terrain,
            graph: tuple[tuple[int, ...], ...], sources: dict[str, int],
            targets: dict[str, int], limit: int, work: routes.Work) -> tuple[list[dict], str | None]:
    """Forward replay is separate from reverse-order construction."""
    require(len(order) == len(plan.steps) and set(order) == set(sources),
            'delivery order does not cover the entire original selection')
    by_name = {step.name: step for step in plan.steps}
    done, blocked, witnesses = set(), set(), []
    for name in order:
        work.check()
        work()
        require(set(by_name[name].after) <= done, 'delivery order weakened original dependencies')
        found = _path(graph, sources[name], targets[name], blocked, limit, work)
        if found is None:
            return [], name  # Never publish a partial sequence certificate.
        distance, parent = found
        witnesses.append({'slot': name, 'item': by_name[name].item,
                          **_witness(terrain, parent, sources[name], targets[name],
                                     distance, blocked, limit, work)})
        blocked.add(targets[name])
        done.add(name)
    return witnesses, None


def _reverse_order(plan: FurniturePlan, graph: tuple[tuple[int, ...], ...],
                   sources: dict[str, int], targets: dict[str, int], limit: int,
                   work: routes.Work) -> tuple[tuple[str, ...] | None, dict | None]:
    """Complete feasibility test for this fixed-assignment, monotone model.

    Choose a dependency sink that is deliverable with every other remaining
    target blocked, then remove it to the suffix. Any valid schedule can move
    that sink to the end: this only removes blockers from earlier deliveries.
    Thus any feasible sink is safe to choose; no exponential permutation search
    or greedy forward false-negative is needed. At most N*(N+1)/2 BFS probes.
    Failure is about these selected items, never every possible allocation.
    """
    remaining = {step.name for step in plan.steps}
    successors = {name: set() for name in remaining}
    for step in plan.steps:
        for before in step.after:
            work()
            successors[before].add(step.name)
    reversed_order = []
    while remaining:
        work.check()
        blocked = set()
        for name in remaining:
            work()
            blocked.add(targets[name])
        rejected, chosen = [], None
        # Descending names put lower lexical names earlier when choices agree.
        for name in sorted(remaining, reverse=True):
            work()
            if successors[name] & remaining:
                continue
            blocked.remove(targets[name])
            found = _path(graph, sources[name], targets[name], blocked, limit, work)
            blocked.add(targets[name])
            if found is not None:
                chosen = name
                break
            rejected.append(name)
        if chosen is None:
            require(bool(rejected), 'acyclic delivery plan has no dependency sink')
            return None, {'remaining_slots': sorted(remaining),
                          'infeasible_final_steps': sorted(rejected),
                          'reason': 'no_selected_delivery_route_within_limit_after_other_targets',
                          'alternative_item_assignments_searched': False}
        reversed_order.append(chosen)
        remaining.remove(chosen)
    return tuple(reversed(reversed_order)), None


def _compile(request: Request, order: tuple[str, ...]) -> Request:
    previous = {name: order[index - 1] for index, name in enumerate(order) if index}
    slots = tuple(replace(slot, after=tuple(sorted(set(slot.after) |
                  ({previous[slot.name]} if slot.name in previous else set()))))
                  for slot in request.slots)
    # Reuse the original bounded JSON, selector and DAG validator. A dependency
    # expansion that no longer fits is a refusal, not silent truncation.
    return Request(request.folder, request.site, slots, request.excluded_items)


def allocate_sequenced_routes(request: Request, candidates: tuple[Candidate, ...],
                              terrain_raw: bytes, guard: Guard = idle, *,
                              max_route_steps: int = routes.MAX_ROUTE_STEPS,
                              maximum_work: int = routes.MAX_WORK) -> tuple[Request, dict]:
    """Return the effective request and allocation; keep them together downstream.

    First run the existing exact route allocator. Retain its item choices and
    initial-route objective. Keep an already viable canonical order unchanged;
    otherwise strengthen `after` constraints to encode a complete feasible order.
    A blocked selected assignment has status `delivery_order_blocked`, no plan,
    and no claim that different selections or actual native routes are absent.
    One work allowance covers allocation, sequencing and forward certificates.
    """
    result = routes.allocate_routes(request, candidates, terrain_raw, guard,
                                    max_route_steps=max_route_steps, maximum_work=maximum_work)
    if result['status'] != 'allocated':
        return request, result
    work = routes.Work(maximum_work, guard)
    work.used = result['work_units']  # Continue the reserved allowance; do not replenish it.
    work()
    terrain = routes.decode_terrain(terrain_raw, work)
    graph = terrain.adjacency(work)
    original = FurniturePlan.from_json(result['plan'])
    sources, targets = {}, {}
    for row in result['assignments']:
        work()
        source, target = terrain.index(tuple(row['position'])), terrain.index(tuple(row['target']))
        require(source is not None and target is not None and terrain.shapes[source]
                and terrain.shapes[target], 'allocated delivery endpoint became unestablished')
        sources[row['slot']], targets[row['slot']] = source, target
    original_order = tuple(step.name for step in original.ordered)
    witnesses, first_blocked = _replay(original_order, original, terrain, graph,
                                      sources, targets, max_route_steps, work)
    order, blocked, effective = original_order, None, request
    if first_blocked is not None:
        order, blocked = _reverse_order(original, graph, sources, targets, max_route_steps, work)
        if order is not None:
            effective = _compile(request, order)
            items = {step.name: step.item for step in original.steps}
            compiled = FurniturePlan(tuple(Step(slot.name, slot.kind, items[slot.name],
                                                slot.target, slot.after) for slot in effective.slots))
            require(tuple(step.name for step in compiled.ordered) == order,
                    'compiled request does not enforce the complete delivery order')
            witnesses, failed = _replay(order, compiled, terrain, graph, sources,
                                        targets, max_route_steps, work)
            require(failed is None, 'compiled delivery order failed forward replay')
            result.update(plan=compiled.json(), plan_digest=compiled.digest,
                          request_digest=effective.digest)
        else:
            result.update(status='delivery_order_blocked', plan=None, plan_digest=None)
    report = {'schema': 'dfmcp.furniture-delivery-sequence/1', 'policy': POLICY, 'model': MODEL,
              'scope': SCOPE, 'status': 'blocked' if order is None else
              'preserved' if first_blocked is None else 'reordered',
              'original_request': request.json(), 'original_request_digest': request.digest,
              'original_plan_digest': original.digest,
              'effective_request_digest': effective.digest,
              'original_order_first_blocked_slot': first_blocked,
              'order': list(order) if order is not None else [], 'routes': witnesses,
              'total_delivery_route_steps': sum(row['route_steps'] for row in witnesses)
              if order is not None else None,
              'blockage': blocked, 'selected_items_preserved': True,
              'original_constraints_preserved': True,
              'candidate_model_sequence_proven': order is not None,
              'native_pathfinding_proven': False, 'future_routes_guaranteed': False,
              'alternative_item_assignments_searched': False}
    work.check()
    result['delivery_sequence'] = report
    result['work_units'] = work.used
    result.pop('decision_digest')
    result['decision_digest'] = hashlib.sha256(
        b'dfmcp-route-furniture-decision/1\0' + canonical(result)).hexdigest()
    require(len(canonical(result)) <= routes.MAX_OUTPUT, 'complete sequenced allocation exceeds output bound')
    work.check()
    return effective, result
