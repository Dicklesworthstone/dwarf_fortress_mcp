"""Optimal NEW item selection without changing the original delivery order.

The existing canonical Kahn order fixes every preceding target-removal set.
This reduces complete-delivery optimization to one exact distinct-item matching,
not exponential assignment/order search. Retained reviews are never inputs.
Beads: df-franken-storage-mvcc-54h.4; df-dfhack-bridge-plane-c-pic.3/.4.
"""
from __future__ import annotations

from collections import deque
import hashlib
import heapq

from furniture_allocation import Candidate, Guard, Request, _maximum_matching, _minimum_cost, idle
from furniture_plan import FurniturePlan, Step, canonical, require
import route_furniture_allocation as routes
import route_furniture_sequence as sequence

POLICY = 'minimum-delivery-cost-original-canonical-order/1'
SCOPE = 'all_supplied_items_original_canonical_order_captured_target_removal_model'


def _lengths(graph, target, blocked, limit, work):
    """One bounded BFS; an item on a previous target is unavailable too."""
    require(target not in blocked, 'original delivery target repeated')
    distance, pending = {target: 0}, deque([target])
    while pending:
        current = pending.popleft()
        work()
        if distance[current] == limit:
            continue
        for following in graph[current]:
            work()
            if following not in blocked and following not in distance:
                distance[following] = distance[current] + 1
                pending.append(following)
    return distance


def allocate_fixed_order_routes(request: Request, candidates: tuple[Candidate, ...],
                                terrain_raw: bytes, guard: Guard = idle, *,
                                max_route_steps: int = routes.MAX_ROUTE_STEPS,
                                maximum_work: int = routes.MAX_WORK) -> tuple[Request, dict]:
    """Minimize delivery cost, then item IDs in lexical SLOT (not execution) order.

    Return the exact original request with a complete new selection or a scoped
    shortage. Every row is priced after ALL earlier canonical targets disappear.
    Assignment choices do not change those blockers, so Hungarian matching gives
    the exact cost optimum for this one order. No alternative order is claimed.

    N best candidates per row, reduced AFTER blocked-target routing, preserve
    both the numeric/lexical optimum and a complete Hall-deficiency witness.
    The original static allocator validates the full inputs and supplies baseline
    diagnostics. Its work is charged to this same shrinking allowance. Working
    memory stays O(items + tiles + N*N); no per-slot map/inventory matrix remains.
    """
    result = routes.allocate_routes(request, candidates, terrain_raw, guard,
                                    max_route_steps=max_route_steps, maximum_work=maximum_work)
    if result['status'] != 'allocated':
        # Unknown targets and static shortage retain their original meaning.
        return request, result
    work = routes.Work(maximum_work, guard)
    work.used = result['work_units']
    work()
    original = FurniturePlan.from_json(result['plan'])
    order = tuple(step.name for step in original.ordered)
    initial_decision = result['decision_digest']
    initial_items = {row['slot']: row['item'] for row in result['assignments']}
    terrain = routes.decode_terrain(terrain_raw, work)
    graph = terrain.adjacency(work)
    by_id = {}
    for item in candidates:
        work()
        by_id[item.id] = item
    identities = sorted(by_id)
    excluded = set(request.excluded_items)
    slots = {slot.name: slot for slot in request.slots}
    targets = {name: terrain.index(slot.target) for name, slot in slots.items()}
    require(all(index is not None and terrain.shapes[index] for index in targets.values()),
            'original target is unestablished')
    rows, counts, blocked = {}, {}, set()
    count = len(slots)
    for name in order:
        work.check()
        lengths = _lengths(graph, targets[name], blocked, max_route_steps, work)
        heap, compatible = [], 0
        for identity in identities:
            work()
            item = by_id[identity]
            if identity in excluded or routes.distance(slots[name], item) is None:
                continue
            length = lengths.get(terrain.index(item.position))
            if length is None:
                continue
            compatible += 1
            entry = (-length, -identity)
            if len(heap) < count:
                heapq.heappush(heap, entry)
            elif entry > heap[0]:
                heapq.heapreplace(heap, entry)
        rows[name] = {-identity: -length for length, identity in heap}
        counts[name] = compatible
        blocked.add(targets[name])
        del lengths
    # Stable lexical row order preserves the original item-vector tie contract,
    # even when dependencies make execution order differ from lexical names.
    edges = tuple(rows[slot.name] for slot in request.slots)
    owners, shortage = _maximum_matching(edges, work)
    total, assignments, witnesses, selected = None, [], [], {}
    plan = None
    if shortage is None:
        chosen = _minimum_cost(edges, work)
        selected = {slot.name: identity for slot, identity in zip(request.slots, chosen)}
        plan = FurniturePlan(tuple(Step(slot.name, slot.kind, selected[slot.name], slot.target, slot.after)
                                   for slot in request.slots))
        plan.check_dimensions(terrain.dimensions)
        require(tuple(step.name for step in plan.ordered) == order,
                'item matching changed the original execution order')
        sources = {}
        for slot in request.slots:
            work.check()
            work()
            item = by_id[selected[slot.name]]
            source, target = terrain.index(item.position), targets[slot.name]
            steps, parent = routes.distances(graph, target, work)
            require(source is not None and 0 <= steps[source] <= max_route_steps,
                    'selected initial route is unavailable')
            sources[slot.name] = source
            assignments.append({'slot': slot.name, **item.json(), 'target': list(slot.target),
                'manhattan_distance': routes.distance(slot, item), 'route_steps': steps[source],
                'route_sha256': routes.path_digest(terrain, parent, source, target, steps[source], work)})
        # Separate forward implementation verifies every chosen route/blocker
        # and its cost; successful matching alone is not a published witness.
        witnesses, failed = sequence._replay(order, plan, terrain, graph, sources,
                                             targets, max_route_steps, work)
        total = sum(rows[name][selected[name]] for name in order)
        require(failed is None and len(witnesses) == count
                and all(row['route_steps'] == rows[row['slot']][row['item']] for row in witnesses)
                and sum(row['route_steps'] for row in witnesses) == total,
                'fixed-order matching failed independent full delivery replay')
    result.update(status='allocated' if plan is not None else 'delivery_order_shortage',
        selection_policy=POLICY,
        objective='total_original_order_delivery_steps_then_lexical_slot_item_ids',
        plan=plan.json() if plan is not None else None,
        plan_digest=plan.digest if plan is not None else None,
        assignments=assignments, total_route_steps=sum(row['route_steps'] for row in assignments)
            if plan is not None else None, maximum_assignable=len(owners),
        shortage=None if shortage is None else {
            'slots': [request.slots[i].name for i in shortage['rows']],
            'candidate_items': shortage['candidate_items'], 'missing': shortage['missing'],
            'scope': SCOPE})
    result['fixed_order_delivery'] = {
        'policy': POLICY, 'scope': SCOPE, 'original_order': list(order),
        'request_unchanged': True, 'dependencies_unchanged': True, 'order_optimized': False,
        'assignment_optimality_proven': plan is not None,
        'infeasibility_proven_for_original_order': shortage is not None,
        'alternative_orders_ruled_out': False, 'complete_delivery_cost': total,
        'compatible_counts': [{'slot': slot.name, 'count': counts[slot.name]} for slot in request.slots],
        'initial_decision_digest': initial_decision,
        'item_replacements': [{'slot': slot.name, 'previous_item': initial_items[slot.name],
                               'selected_item': selected[slot.name]}
                              for slot in request.slots if plan is not None
                              and initial_items[slot.name] != selected[slot.name]],
        'retained_reviews_modified': False, 'native_hauling_time_minimized': False}
    result['delivery_sequence'] = {
        'schema': 'dfmcp.furniture-delivery-sequence/1', 'policy': POLICY, 'model': sequence.MODEL,
        'scope': SCOPE, 'status': 'fixed_order_optimized' if plan is not None else 'blocked',
        'original_request': request.json(), 'original_request_digest': request.digest,
        'original_plan_digest': original.digest, 'effective_request_digest': request.digest,
        'order': list(order) if plan is not None else [], 'routes': witnesses,
        'total_delivery_route_steps': total, 'blockage': result['shortage'],
        'selected_items_preserved': selected == initial_items,
        'original_constraints_preserved': True, 'candidate_model_sequence_proven': plan is not None,
        'native_pathfinding_proven': False, 'future_routes_guaranteed': False,
        'alternative_item_assignments_searched': True}
    work()
    result['work_units'] = work.used
    result.pop('decision_digest')
    result['decision_digest'] = hashlib.sha256(
        b'dfmcp-route-furniture-decision/1\0' + canonical(result)).hexdigest()
    require(len(canonical(result)) <= routes.MAX_OUTPUT, 'complete fixed-order result exceeds bound')
    work.check()
    return request, result
