"""Minimize complete furniture-delivery cost before original intent is retained.

Initial shortest-route matching is only a lower bound: preceding placements can
force detours or require other items. This opt-in bounded branch-and-bound search
optimizes assignment AND order in the existing target-removal model. It grants
no native authority and never runs on a retained review or executing batch.
Beads: df-franken-storage-mvcc-54h.4; df-dfhack-bridge-plane-c-pic.3/.4.
"""
from __future__ import annotations

from collections import deque
import hashlib
import heapq
import struct

from furniture_allocation import Candidate, Guard, Request, _maximum_matching, _minimum_cost, idle
from furniture_plan import FurniturePlan, Step, canonical, integer, require
import route_furniture_allocation as routes
import route_furniture_sequence as sequence

POLICY = 'minimum-complete-delivery-cost-bounded-reverse-assignment/1'
MAX_SEARCH_NODES = 4096


def _lengths(graph, root, blocked, limit, work):
    require(root not in blocked, 'delivery bound removed its own target')
    lengths, queue = {root: 0}, deque([root])
    while queue:
        current = queue.popleft()
        work()
        if lengths[current] == limit:
            continue
        for following in graph[current]:
            work()
            if following not in blocked and following not in lengths:
                lengths[following] = lengths[current] + 1
                queue.append(following)
    return lengths


def _choices(slot, remaining, used, candidates, terrain, graph, targets, limit, work):
    """N best AFTER routing and suffix exclusion preserve the cost optimum.

    Other N-1 remaining slots can use at most N-1 items. A dropped item in a
    minimum solution can be replaced by an unused retained item of no greater
    final-delivery cost, without changing any target, dependency or other route.
    This does not justify truncating by the INITIAL unblocked route costs.
    The full BFS is released before recursion; retained choices are at most 32.
    """
    blocked = set()
    for name in remaining:
        work()
        if name != slot.name:
            blocked.add(targets[name])
    lengths = _lengths(graph, targets[slot.name], blocked, limit, work)
    heap = []
    for item in candidates:
        work()
        if item.id in used or routes.distance(slot, item) is None:
            continue
        length = lengths.get(terrain.index(item.position))
        if length is None:
            continue
        value = (-length, -item.id)
        if len(heap) < len(remaining):
            heapq.heappush(heap, value)
        elif value > heap[0]:
            heapq.heapreplace(heap, value)
    return tuple(sorted((-cost, -identity) for cost, identity in heap))


def _optimize(request, candidates, terrain, graph, limit, maximum_nodes, work,
              initial_order, initial_items, initial_cost, global_lower):
    slots = {slot.name: slot for slot in request.slots}
    targets = {name: terrain.index(slot.target) for name, slot in slots.items()}
    require(all(index is not None and terrain.shapes[index] for index in targets.values()),
            'delivery optimization target unestablished')
    excluded = set(request.excluded_items)
    eligible = []
    for item in candidates:
        work()
        if item.id not in excluded and terrain.reason(item.position) == 'candidate':
            eligible.append(item)
    eligible.sort(key=lambda item: item.id)
    work.check()
    successors = {name: set() for name in slots}
    for slot in request.slots:
        for before in slot.after:
            work()
            successors[before].add(slot.name)
    ancestors = {}
    def predecessors(name):
        if name not in ancestors:
            result = set()
            for before in slots[name].after:
                work()
                result.add(before)
                result.update(predecessors(before))
            ancestors[name] = result
        return ancestors[name]

    # Mandatory predecessors are blocked in EVERY valid schedule. Their routes
    # yield a stronger admissible bound than initial distances, without guessing
    # a total order. This avoids enumerating permutations of independent work
    # just to rediscover an unavoidable dependency-induced detour.
    # Retain N static
    # candidates per slot. After K suffix items are used, at least N-K of
    # these remain whenever the full row has that many candidates. Reducing
    # each remaining row again to R cheapest unused items preserves the exact
    # distinct-item matching lower bound (but not final-route feasibility).
    relaxed_edges = {}
    for name, slot in slots.items():
        steps = _lengths(graph, targets[name], {targets[p] for p in predecessors(name)}, limit, work)
        heap = []
        for item in eligible:
            work()
            length = steps.get(terrain.index(item.position))
            if routes.distance(slot, item) is None or length is None:
                continue
            value = (-length, -item.id)
            if len(heap) < len(slots):
                heapq.heappush(heap, value)
            elif value > heap[0]:
                heapq.heapreplace(heap, value)
        relaxed_edges[name] = tuple(sorted((-cost, -identity) for cost, identity in heap))
    best_cost, best_order, best_items = initial_cost, initial_order, initial_items
    counters = {'visited_nodes': 0, 'bound_prunes': 0, 'matching_prunes': 0,
                'complete_candidates': 0, 'improvements': 0, 'sink_probes': 0}
    trace = hashlib.sha256(b'dfmcp-optimal-delivery-search/1\0')
    cutoff = False

    def visit(remaining, used, suffix, assigned, cost):
        nonlocal best_cost, best_order, best_items, cutoff, global_lower
        work.check()
        work()
        if counters['visited_nodes'] == maximum_nodes:
            cutoff = True
            return
        counters['visited_nodes'] += 1
        mask = sum(1 << i for i, name in enumerate(slots) if name in remaining)
        trace.update(b'N' + struct.pack('>IQ', mask, cost))
        if not remaining:
            counters['complete_candidates'] += 1
            if best_cost is None or cost < best_cost:
                best_cost, best_order, best_items = cost, tuple(reversed(suffix)), dict(assigned)
                counters['improvements'] += 1
            return
        edges = []
        for name in sorted(remaining):
            row = {}
            for length, identity in relaxed_edges[name]:
                work()
                if identity not in used:
                    row[identity] = length
                    if len(row) == len(remaining):
                        break
            edges.append(row)
        _, shortage = _maximum_matching(tuple(edges), work)
        if shortage is not None:
            counters['matching_prunes'] += 1
            return
        chosen = _minimum_cost(tuple(edges), work)
        lower = cost + sum(row[identity] for row, identity in zip(edges, chosen))
        if not assigned:
            global_lower = max(global_lower, lower)
        trace.update(b'L' + struct.pack('>Q', lower))
        if best_cost is not None and lower >= best_cost:
            counters['bound_prunes'] += 1
            return
        for name in sorted(remaining, reverse=True):
            work()
            if successors[name] & remaining:
                continue
            counters['sink_probes'] += 1
            options = _choices(slots[name], remaining, used, eligible, terrain,
                               graph, targets, limit, work)
            for length, identity in options:
                work()
                if best_cost is not None and cost + length >= best_cost:
                    counters['bound_prunes'] += 1
                    continue
                trace.update(b'C' + struct.pack('>II', list(slots).index(name), identity))
                visit(remaining - {name}, used | {identity}, suffix + (name,),
                      {**assigned, name: identity}, cost + length)
                if cutoff or best_cost == global_lower:
                    return

    visit(set(slots), set(), (), {}, 0)
    complete = not cutoff or best_cost == global_lower
    return best_order, best_items, best_cost, {**counters, 'search_complete': complete,
        'node_limit_reached': cutoff, 'trace_digest': trace.hexdigest(),
        'relaxation_lower_bound': global_lower}


def allocate_optimal_routes(request: Request, candidates: tuple[Candidate, ...],
                            terrain_raw: bytes, guard: Guard = idle, *,
                            max_route_steps: int = routes.MAX_ROUTE_STEPS,
                            maximum_work: int = routes.MAX_WORK,
                            max_search_nodes: int = MAX_SEARCH_NODES,
                            require_optimal: bool = False) -> tuple[Request, dict]:
    """Return a full best-known proposal with explicit optimality/gap evidence.

    Cost is the sum of shortest captured-model routes AT EACH delivery, after
    earlier target removal, not initial distances or measured hauling time.
    Equal-cost solutions keep the first deterministic incumbent; no global
    lexical tie optimum is claimed. A node limit may retain a complete feasible
    incumbent, but strict callers receive no plan until cost optimality is proved.
    Work exhaustion/cancellation raises, never publishes an incumbent or renews
    the budget. The independent forward replay covers every proposed delivery.
    """
    integer(max_search_nodes, 1, MAX_SEARCH_NODES)
    require(type(require_optimal) is bool, 'optimal delivery requirement must be Boolean')
    effective, result = sequence.allocate_sequenced_routes(request, candidates, terrain_raw, guard,
        max_route_steps=max_route_steps, maximum_work=maximum_work)
    if result['status'] not in ('allocated', 'delivery_order_blocked'):
        return effective, result
    work = routes.Work(maximum_work, guard)
    work.used = result['work_units']
    work()
    initial_decision = result['decision_digest']
    global_lower = result['total_route_steps']
    delivered = result['status'] == 'allocated'
    best_order = tuple(result['delivery_sequence']['order']) if delivered else None
    best_items = {row['slot']: row['item'] for row in result['assignments']} if delivered else None
    best_cost = result['delivery_sequence']['total_delivery_route_steps'] if delivered else None
    original_cost = best_cost
    stats = {'visited_nodes': 0, 'bound_prunes': 0, 'matching_prunes': 0,
             'complete_candidates': 0, 'improvements': 0, 'sink_probes': 0,
             'search_complete': True, 'node_limit_reached': False, 'trace_digest': None,
             'relaxation_lower_bound': global_lower}
    terrain = routes.decode_terrain(terrain_raw, work)
    graph = terrain.adjacency(work)
    if best_cost != global_lower:
        best_order, best_items, best_cost, stats = _optimize(request, candidates, terrain, graph,
            max_route_steps, max_search_nodes, work, best_order, best_items, best_cost, global_lower)
    global_lower = stats['relaxation_lower_bound']
    proven = best_cost is not None and (stats['search_complete'] or best_cost == global_lower)
    lower = best_cost if proven else global_lower
    initial_items = {row['slot']: row['item'] for row in result['assignments']}
    witnesses, assignments, total = [], [], None
    if best_items is not None:
        effective = sequence._compile(request, best_order)
        plan = FurniturePlan(tuple(Step(slot.name, slot.kind, best_items[slot.name], slot.target, slot.after)
                                   for slot in effective.slots))
        plan.check_dimensions(terrain.dimensions)
        require(tuple(step.name for step in plan.ordered) == best_order, 'optimized order changed during lowering')
        by_id, sources, targets = {}, {}, {}
        for item in candidates:
            work()
            by_id[item.id] = item
        for slot in request.slots:
            work()
            item = by_id[best_items[slot.name]]
            require(item.id not in request.excluded_items and routes.distance(slot, item) is not None,
                    'optimized selection changed an original constraint')
            source, target = terrain.index(item.position), terrain.index(slot.target)
            steps, parent = routes.distances(graph, target, work)
            require(source is not None and 0 <= steps[source] <= max_route_steps, 'optimized initial route unavailable')
            sources[slot.name], targets[slot.name] = source, target
            assignments.append({'slot': slot.name, **item.json(), 'target': list(slot.target),
                'manhattan_distance': routes.distance(slot, item), 'route_steps': steps[source],
                'route_sha256': routes.path_digest(terrain, parent, source, target, steps[source], work)})
        witnesses, failed = sequence._replay(best_order, plan, terrain, graph, sources, targets, max_route_steps, work)
        require(failed is None and sum(row['route_steps'] for row in witnesses) == best_cost,
                'optimized delivery cost failed independent forward replay')
        total = sum(row['route_steps'] for row in assignments)
    export = best_items is not None and (proven or not require_optimal)
    result.update(status='allocated' if export else 'delivery_optimization_incomplete'
        if not stats['search_complete'] else 'delivery_assignment_blocked',
        plan=plan.json() if export else None, plan_digest=plan.digest if export else None,
        request_digest=effective.digest if export else request.digest,
        assignments=assignments if export else [], total_route_steps=total if export else None,
        objective='minimum_total_delivery_route_steps_in_target_removal_model', selection_policy=POLICY)
    result['delivery_sequence'].update(status='optimized' if export and proven else
        'feasible_unproven' if export else 'search_incomplete' if not stats['search_complete'] else 'blocked',
        scope='all_supplied_items_original_dependencies_captured_target_removal_model',
        effective_request_digest=effective.digest if export else request.digest,
        order=list(best_order) if export else [], routes=witnesses if export else [],
        total_delivery_route_steps=best_cost if export else None,
        selected_items_preserved=best_items == initial_items if export else False,
        alternative_item_assignments_searched=stats['visited_nodes'] > 0,
        candidate_model_sequence_proven=export, blockage=None)
    result['delivery_optimization'] = {'policy': POLICY, 'model': sequence.MODEL,
        **stats, 'max_search_nodes': max_search_nodes, 'require_optimal': require_optimal,
        'cost_optimality_proven': proven,
        'proof_basis': 'matching_lower_bound_attained' if proven and best_cost == global_lower else
            'branch_and_bound_complete' if proven else 'incomplete_search', 'best_complete_cost': best_cost, 'cost_lower_bound': lower,
        'absolute_cost_gap': None if best_cost is None else best_cost - lower,
        'initial_complete_cost': original_cost, 'initial_decision_digest': initial_decision,
        'infeasibility_proven_in_supplied_model': best_cost is None and stats['search_complete'],
        'retained_reviews_modified': False, 'native_hauling_time_minimized': False}
    work()
    result['work_units'] = work.used
    result.pop('decision_digest')
    result['decision_digest'] = hashlib.sha256(b'dfmcp-route-furniture-decision/1\0' + canonical(result)).hexdigest()
    require(len(canonical(result)) <= routes.MAX_OUTPUT, 'complete optimal delivery response exceeds bound')
    work.check()
    return effective if export else request, result
