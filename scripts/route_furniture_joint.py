"""Bounded alternative-item search for NEW delivery-aware furniture proposals.

A retained review is never an input. First preserve a deliverable existing
allocation. Otherwise search assignments and dependency orders together in the
shared target-removal model. No acquisition, placement or retry authority.
Beads: df-franken-storage-mvcc-54h.4; df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

from collections import deque
import hashlib
import heapq

from furniture_allocation import Candidate, Guard, Request, idle
from furniture_plan import FurniturePlan, Step, canonical, integer, require
import route_furniture_allocation as routes
import route_furniture_sequence as sequence

POLICY = 'preserve-initial-else-bounded-reverse-item-search/1'
SCOPE = 'complete_supplied_items_original_constraints_captured_target_removal_model'
MAX_SEARCH_NODES = 4096


def _options(slot, remaining, used, candidates, terrain, graph, targets, limit, work):
    """Keep N best unused, compatible, currently deliverable items for N slots.

    This preserves feasibility: in any completion the other N-1 slots cannot
    consume all N alternatives. Swapping an unused item into this final delivery
    changes no blocker or other delivery. Unlike initial-route truncation, this
    reduction happens AFTER blocked-prefix routing and suffix-item exclusion.
    One BFS is released before recursion; no depth-by-map matrix is retained.
    """
    blocked = set()
    for name in remaining:
        work()
        if name != slot.name:
            blocked.add(targets[name])
    root = targets[slot.name]
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
    heap = []
    for item in candidates:
        work()
        if item.id in used or routes.distance(slot, item) is None:
            continue
        index = terrain.index(item.position)
        length = lengths.get(index)
        if length is None:
            continue
        entry = (-length, -item.id)
        if len(heap) < len(remaining):
            heapq.heappush(heap, entry)
        elif entry > heap[0]:
            heapq.heapreplace(heap, entry)
    return tuple(identity for _, identity in sorted((-cost, -identity) for cost, identity in heap))


def _search(request, candidates, terrain, graph, limit, maximum_nodes, work):
    slots = {slot.name: slot for slot in request.slots}
    targets = {name: terrain.index(slot.target) for name, slot in slots.items()}
    require(all(index is not None and terrain.shapes[index] for index in targets.values()),
            'joint delivery target is unestablished')
    successors = {name: set() for name in slots}
    for slot in request.slots:
        for before in slot.after:
            work()
            successors[before].add(slot.name)
    excluded = set(request.excluded_items)
    eligible = []
    for item in candidates:
        work()
        if item.id not in excluded and terrain.reason(item.position) == 'candidate':
            eligible.append(item)
    # Candidate order cannot make branch choices or work accounting nondeterministic.
    eligible.sort(key=lambda item: item.id)
    nodes, cut_off = 0, False

    def visit(remaining, used):
        nonlocal nodes, cut_off
        work.check()
        if nodes >= maximum_nodes:
            cut_off = True
            return None
        nodes += 1
        work()
        if not remaining:
            return (), {}
        for name in sorted(remaining, reverse=True):
            work()
            if successors[name] & remaining:
                continue
            choices = _options(slots[name], remaining, used, eligible, terrain,
                               graph, targets, limit, work)
            for identity in choices:
                work()
                found = visit(remaining - {name}, used | {identity})
                if found is not None:
                    order, assignment = found
                    assignment[name] = identity
                    return order + (name,), assignment
                if cut_off:
                    return None
        return None

    found = visit(set(slots), set())
    return found, nodes, cut_off


def _initial_routes(request, chosen, candidates, terrain, graph, work):
    """Reconstruct ordinary assignment evidence for the newly selected items."""
    by_id = {}
    for item in candidates:
        work()
        by_id[item.id] = item
    assignments, total, sources, targets = [], 0, {}, {}
    for slot in request.slots:
        work.check()
        work()
        item = by_id[chosen[slot.name]]
        lower_bound = routes.distance(slot, item)
        require(lower_bound is not None and item.id not in request.excluded_items,
                'joint delivery changed an original selector')
        source, target = terrain.index(item.position), terrain.index(slot.target)
        steps, parent = routes.distances(graph, target, work)
        require(steps[source] >= 0, 'joint delivery initial route is absent')
        assignments.append({'slot': slot.name, **item.json(), 'target': list(slot.target),
            'manhattan_distance': lower_bound, 'route_steps': steps[source],
            'route_sha256': routes.path_digest(terrain, parent, source, target, steps[source], work)})
        sources[slot.name], targets[slot.name] = source, target
        total += steps[source]
    return assignments, total, sources, targets


def allocate_joint_routes(request: Request, candidates: tuple[Candidate, ...], terrain_raw: bytes,
                          guard: Guard = idle, *, max_route_steps: int = routes.MAX_ROUTE_STEPS,
                          maximum_work: int = routes.MAX_WORK,
                          max_search_nodes: int = MAX_SEARCH_NODES) -> tuple[Request, dict]:
    """Return a complete NEW effective request and proposal, or no executable plan.

    Prefer the existing minimum-initial-distance assignment whenever sequencing
    succeeds. Search only after its fixed-item sequence is blocked. The fallback
    is deterministic first-feasible search, NOT a global cost optimizer. Node
    exhaustion returns a distinct incomplete diagnostic; work/cancellation/output
    refusal raises without a partial result. No allowance is replenished.
    """
    integer(max_search_nodes, 1, MAX_SEARCH_NODES)
    effective, result = sequence.allocate_sequenced_routes(request, candidates, terrain_raw, guard,
        max_route_steps=max_route_steps, maximum_work=maximum_work)
    initial_digest = result['decision_digest']
    report = {'policy': POLICY, 'scope': SCOPE, 'max_search_nodes': max_search_nodes,
        'visited_nodes': 0, 'status': 'not_needed', 'search_complete': False,
        'infeasibility_proven': False, 'global_cost_optimality_proven': False,
        'initial_decision_digest': initial_digest, 'item_replacements': [],
        'retained_reviews_modified': False, 'native_pathfinding_proven': False}
    if result['status'] == 'delivery_order_blocked':
        work = routes.Work(maximum_work, guard)
        work.used = result['work_units']
        work()
        terrain = routes.decode_terrain(terrain_raw, work)
        graph = terrain.adjacency(work)
        found, nodes, cut_off = _search(request, candidates, terrain, graph,
                                       max_route_steps, max_search_nodes, work)
        report['visited_nodes'] = nodes
        if found is not None:
            order, chosen = found
            require(len(set(chosen.values())) == len(request.slots), 'joint delivery reused an item')
            effective = sequence._compile(request, order)
            plan = FurniturePlan(tuple(Step(slot.name, slot.kind, chosen[slot.name], slot.target, slot.after)
                                       for slot in effective.slots))
            plan.check_dimensions(terrain.dimensions)
            require(tuple(step.name for step in plan.ordered) == order, 'joint delivery order changed')
            prior = {row['slot']: row['item'] for row in result['assignments']}
            assignments, total, sources, targets = _initial_routes(effective, chosen, candidates, terrain, graph, work)
            # Independently replay the full returned assignment/order before export.
            witnesses, failed = sequence._replay(order, plan, terrain, graph, sources, targets, max_route_steps, work)
            require(failed is None, 'joint delivery failed complete forward verification')
            report.update(status='alternative_found', item_replacements=[
                {'slot': slot.name, 'previous_item': prior[slot.name], 'selected_item': chosen[slot.name]}
                for slot in request.slots if prior[slot.name] != chosen[slot.name]])
            result.update(status='allocated', plan=plan.json(), plan_digest=plan.digest,
                request_digest=effective.digest, assignments=assignments, total_route_steps=total,
                objective='preserve_initial_assignment_else_first_feasible_joint_delivery')
            result['delivery_sequence'].update(status='reassigned', scope=SCOPE,
                effective_request_digest=effective.digest, order=list(order), routes=witnesses,
                total_delivery_route_steps=sum(row['route_steps'] for row in witnesses),
                blockage=None, selected_items_preserved=not report['item_replacements'],
                alternative_item_assignments_searched=True, candidate_model_sequence_proven=True)
        else:
            report.update(status='node_limit_reached' if cut_off else 'no_assignment',
                          search_complete=not cut_off, infeasibility_proven=not cut_off)
            result.update(status='delivery_search_exhausted' if cut_off else 'delivery_assignment_blocked',
                          plan=None, plan_digest=None)
            result['delivery_sequence'].update(status='search_exhausted' if cut_off else 'blocked',
                scope=SCOPE, order=[], routes=[], total_delivery_route_steps=None,
                candidate_model_sequence_proven=False, alternative_item_assignments_searched=True,
                blockage=None if cut_off else {'reason': 'no_assignment_and_order_in_supplied_model',
                    'scope': SCOPE, 'alternative_item_assignments_searched': True})
        result['work_units'] = work.used
        work.check()
    elif result['status'] == 'allocated':
        report['status'] = 'initial_assignment_preserved'
    result['delivery_search'] = report
    result.pop('decision_digest')
    result['decision_digest'] = hashlib.sha256(
        b'dfmcp-route-furniture-decision/1\0' + canonical(result)).hexdigest()
    require(len(canonical(result)) <= routes.MAX_OUTPUT, 'complete joint delivery output exceeds bound')
    guard()
    return effective, result
