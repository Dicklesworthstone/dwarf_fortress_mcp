"""Joint item selection and delivery order before any original review is sealed.

Opt-in fallback for an undeliverable cheapest assignment. Existing successful
results are unchanged. Search never runs during batch recovery/revalidation and
cannot substitute items in a retained original plan or acquire game authority.
Beads: df-franken-storage-mvcc-54h.4; df-dfhack-bridge-plane-c-pic.3/.4.
"""
from __future__ import annotations

from collections import deque
import hashlib
import struct

from furniture_allocation import Candidate, Guard, Request, idle
from furniture_plan import FurniturePlan, Step, canonical, integer, require
import route_furniture_allocation as routes
import route_furniture_sequence as sequence

POLICY = 'preserve-deliverable-else-joint-reverse-sink-item-search/1'
MAX_SEARCH_NODES = 100_000
DEFAULT_SEARCH_NODES = 10_000


def _reachable(graph: tuple[tuple[int, ...], ...], target: int, blocked: set[int],
               limit: int, work: routes.Work) -> set[int]:
    """One limited BFS per sink, shared by all candidate items at this state."""
    work()
    require(target not in blocked, 'joint search blocked its selected sink')
    seen = {target}
    queue = deque([(target, 0)])
    while queue:
        tile, length = queue.popleft()
        work()
        if length == limit:
            continue
        for other in graph[tile]:
            work()
            if other not in blocked and other not in seen:
                seen.add(other)
                queue.append((other, length + 1))
    return seen


def _search(request: Request, candidates: tuple[Candidate, ...], terrain: routes.Terrain,
            graph: tuple[tuple[int, ...], ...], limit: int, maximum_nodes: int,
            work: routes.Work) -> tuple[tuple[dict[str, int], tuple[str, ...]] | None, dict]:
    """Exhaustive bounded reverse scheduling, branching on BOTH sink and item.

    In any complete schedule its last step is a dependency sink. Its source
    must reach its target with every earlier target removed. Enumerating every
    sink and every unused compatible item therefore includes every feasible
    schedule/assignment. The suffix already selected executes later and does
    not block an earlier delivery. No cheapest-N candidate reduction is sound
    here: an item outside that initial-cost subset may be the only deliverable one.

    No memoized subset or slots-by-inventory matrix is allocated. Recursion is
    bounded by the validated 32 slots; at most that many BFS sets remain live.
    Reaching the node cap propagates an inconclusive result, not exhaustion.
    Caller/work exceptions are not caught or converted into search outcomes.
    """
    n = len(request.slots)
    names = {slot.name: i for i, slot in enumerate(request.slots)}
    successors = [0] * n
    targets = []
    for i, slot in enumerate(request.slots):
        work()
        target = terrain.index(slot.target)
        require(target is not None and terrain.shapes[target], 'joint target unestablished')
        targets.append(target)
        for predecessor in slot.after:
            work()
            successors[names[predecessor]] |= 1 << i
    excluded = set(request.excluded_items)
    available = []
    for item in candidates:
        work()
        tile = terrain.index(item.position)
        if item.id not in excluded and tile is not None and terrain.shapes[tile]:
            available.append((item, tile))
    available.sort(key=lambda pair: pair[0].id)
    work.check()
    used, chosen, suffix = set(), {}, []
    stats = {'nodes': 0, 'sink_probes': 0, 'candidate_probes': 0, 'backtracks': 0}
    trace = hashlib.sha256(b'dfmcp-joint-delivery-search/1\0')
    answer = None

    def visit(mask: int) -> bool | None:
        nonlocal answer
        work()
        work.check()
        if stats['nodes'] == maximum_nodes:
            return None
        stats['nodes'] += 1
        trace.update(b'N' + struct.pack('>I', mask))
        if not mask:
            answer = dict(chosen), tuple(reversed(suffix))
            return True
        blocked = set()
        for i, target in enumerate(targets):
            work()
            if mask & (1 << i):
                blocked.add(target)
        for i in range(n - 1, -1, -1):
            work()
            if not mask & (1 << i) or successors[i] & mask:
                continue
            slot = request.slots[i]
            stats['sink_probes'] += 1
            reachable = _reachable(graph, targets[i], blocked - {targets[i]}, limit, work)
            for item, tile in available:
                work()
                stats['candidate_probes'] += 1
                if item.id in used or tile not in reachable or routes.distance(slot, item) is None:
                    continue
                trace.update(b'C' + struct.pack('>II', i, item.id))
                chosen[slot.name] = item.id
                used.add(item.id)
                suffix.append(slot.name)
                outcome = visit(mask & ~(1 << i))
                suffix.pop()
                used.remove(item.id)
                del chosen[slot.name]
                if outcome is not False:
                    return outcome
                stats['backtracks'] += 1
                trace.update(b'B')
        return False

    outcome = visit((1 << n) - 1)
    stats.update(max_search_nodes=maximum_nodes, trace_digest=trace.hexdigest(),
                 status='found' if outcome is True else 'exhausted' if outcome is False else 'limit_reached',
                 search_exhausted=outcome is False, candidate_lists_truncated=False)
    return answer, stats


def allocate_joint_routes(request: Request, candidates: tuple[Candidate, ...],
                          terrain_raw: bytes, guard: Guard = idle, *,
                          max_route_steps: int = routes.MAX_ROUTE_STEPS,
                          maximum_work: int = routes.MAX_WORK,
                          max_search_nodes: int = DEFAULT_SEARCH_NODES) -> tuple[Request, dict]:
    """Propose a complete deliverable assignment, never revise retained intent.

    First preserve the legacy minimum-initial-cost assignment and its feasible
    order when possible. Only a delivery_order_blocked result invokes joint
    search over the full supplied inventory. Fallback chooses the first feasible
    solution in descending sink-name/ascending item-ID order, NOT a cost optimum.
    One work allowance spans the initial solve, search, forward replay and output.
    """
    integer(max_search_nodes, 1, MAX_SEARCH_NODES)
    effective, result = sequence.allocate_sequenced_routes(
        request, candidates, terrain_raw, guard,
        max_route_steps=max_route_steps, maximum_work=maximum_work)
    if result['status'] != 'delivery_order_blocked':
        return effective, result
    work = routes.Work(maximum_work, guard)
    work.used = result['work_units']
    work()
    terrain = routes.decode_terrain(terrain_raw, work)
    graph = terrain.adjacency(work)
    initial_decision = result['decision_digest']
    initial_items = {row['slot']: row['item'] for row in result['assignments']}
    answer, stats = _search(request, candidates, terrain, graph,
                            max_route_steps, max_search_nodes, work)
    report = result['delivery_sequence']
    report.update(policy=POLICY,
                  scope='all_supplied_items_with_original_dependencies_and_route_limit',
                  alternative_item_assignments_searched=True,
                  initial_assignment_digest=initial_decision,
                  initial_assignment=[{'slot': name, 'item': item}
                                      for name, item in sorted(initial_items.items())],
                  initial_assignment_blockage=report['blockage'], search=stats,
                  minimum_initial_cost_proven=False)
    # Baseline assignment rows are not an executable fallback on failure.
    result.update(assignments=[], plan=None, plan_digest=None, total_route_steps=None,
                  objective='first_deliverable_reverse_sink_then_item_id/1', selection_policy=POLICY)
    if answer is None:
        limited = stats['status'] == 'limit_reached'
        result['status'] = 'delivery_assignment_search_limit' if limited else 'delivery_assignment_blocked'
        report.update(status='search_limit_reached' if limited else 'blocked',
                      order=[], routes=[], total_delivery_route_steps=None,
                      selected_items_preserved=False, candidate_model_sequence_proven=False,
                      blockage={'reason': 'joint_search_node_limit' if limited else
                                'no_assignment_and_order_in_captured_target_removal_model',
                                'alternative_item_assignments_searched': True,
                                'search_exhausted': not limited})
    else:
        items, order = answer
        original = FurniturePlan(tuple(Step(slot.name, slot.kind, items[slot.name],
                                            slot.target, slot.after) for slot in request.slots))
        effective = request if tuple(step.name for step in original.ordered) == order else sequence._compile(request, order)
        compiled = FurniturePlan(tuple(Step(slot.name, slot.kind, items[slot.name],
                                            slot.target, slot.after) for slot in effective.slots))
        compiled.check_dimensions(terrain.dimensions)
        require(tuple(step.name for step in compiled.ordered) == order,
                'joint solution did not preserve the compiled delivery order')
        by_id = {}
        for item in candidates:
            work()
            by_id[item.id] = item
        sources, targets, assignments = {}, {}, []
        for slot in request.slots:
            work()
            item = by_id[items[slot.name]]
            source, target = terrain.index(item.position), terrain.index(slot.target)
            require(item.id not in request.excluded_items and routes.distance(slot, item) is not None,
                    'joint assignment violated an original selector')
            steps, parent = routes.distances(graph, target, work)
            require(source is not None and 0 <= steps[source] <= max_route_steps,
                    'joint initial route is not established')
            sources[slot.name], targets[slot.name] = source, target
            assignments.append({'slot': slot.name, **item.json(), 'target': list(slot.target),
                'manhattan_distance': routes.distance(slot, item), 'route_steps': steps[source],
                'route_sha256': routes.path_digest(terrain, parent, source, target, steps[source], work)})
        witnesses, failed = sequence._replay(order, compiled, terrain, graph, sources,
                                             targets, max_route_steps, work)
        require(failed is None, 'joint assignment failed independent forward replay')
        result.update(status='allocated', plan=compiled.json(), plan_digest=compiled.digest,
                      request_digest=effective.digest, assignments=assignments,
                      total_route_steps=sum(row['route_steps'] for row in assignments))
        report.update(status='reassigned', effective_request_digest=effective.digest,
                      selected_plan_before_ordering_digest=original.digest,
                      order=list(order), routes=witnesses, blockage=None,
                      total_delivery_route_steps=sum(row['route_steps'] for row in witnesses),
                      changed_item_count=sum(items[name] != item for name, item in initial_items.items()),
                      selected_items_preserved=items == initial_items, candidate_model_sequence_proven=True)
    work()
    result['work_units'] = work.used
    result.pop('decision_digest')
    result['decision_digest'] = hashlib.sha256(
        b'dfmcp-route-furniture-decision/1\0' + canonical(result)).hexdigest()
    require(len(canonical(result)) <= routes.MAX_OUTPUT, 'complete joint allocation exceeds output bound')
    work.check()
    return effective, result
