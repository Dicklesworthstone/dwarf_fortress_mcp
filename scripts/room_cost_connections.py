"""Cost-aware corridor candidates under the original geometry and hard limits.

Compare the existing connector with deterministic cost-first greedy routing.
Costs are planning input, NOT observed floors or permission to excavate. The
caller must independently derive/validate any map evidence. Both attempts share
one work allowance. No global cost optimum or infeasibility proof is implied.
Beads: dfmcp-wp-pln01-spatial-blueprint-layout-pwd; df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import heapq
import json

import room_connections as original

POLICY = 'dfmcp.cost-aware-room-connections/1'
STRATEGY = 'cost-first-with-original-candidate/1'
BOUNDED_STRATEGY = 'bounded-cost-with-original-candidate/1'
BOUNDED_POLICY = 'dfmcp.bounded-cost-aware-room-connections/1'
Point = original.Point
Region = original.Region
Guard = original.Guard
require = original.require


def _canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(',', ':'),
                      ensure_ascii=True, allow_nan=False).encode('ascii')


@dataclass(frozen=True)
class Costs:
    region: Region
    cells: str
    strategy: str = STRATEGY

    @classmethod
    def parse(cls, value: object, guard: Guard = original.idle) -> Costs:
        guard()
        require(type(value) is dict and {'region', 'cells'} <= set(value)
                and set(value) <= {'region', 'cells', 'strategy'},
                'corridor costs require one complete region and cell string')
        strategy = value.get('strategy', STRATEGY)
        require(type(strategy) is str and strategy in (STRATEGY, BOUNDED_STRATEGY),
                'unsupported corridor cost strategy')
        region = value['region']
        require(type(region) is dict and set(region) == {'origin', 'size'}
                and type(region['origin']) is list and type(region['size']) is list
                and len(region['origin']) == len(region['size']) == 3,
                'invalid corridor cost region')
        low, size = tuple(region['origin']), tuple(region['size'])
        require(original.point(low) and all(type(n) is int and 1 <= n <= 128 for n in size)
                and size[2] == 1 and size[0] * size[1] <= original.MAX_CELLS
                and all(a + n <= 32767 for a, n in zip(low, size)),
                'corridor costs exceed the bounded same-level region')
        cells = value['cells']
        require(type(cells) is str and len(cells) == size[0] * size[1],
                'corridor costs must cover every region cell')
        for cell in cells:
            guard()
            require(cell in '01', 'corridor costs must be binary characters')
        return cls((low, size), cells, strategy)

    def json(self) -> dict:
        value = {'region': {'origin': list(self.region[0]), 'size': list(self.region[1])},
                 'cells': self.cells}
        if self.strategy != STRATEGY:
            value['strategy'] = self.strategy
        return value

    def at(self, point: Point) -> int:
        require(original.point(point), 'corridor lookup requires three exact coordinates')
        x, y, z = (p - a for p, a in zip(point, self.region[0]))
        require(z == 0 and 0 <= x < self.region[1][0] and 0 <= y < self.region[1][1],
                'corridor target outside original cost region')
        return int(self.cells[y * self.region[1][0] + x])

    @property
    def digest(self) -> str:
        return hashlib.sha256(b'dfmcp-room-corridor-costs/1\0' + _canonical(self.json())).hexdigest()


@dataclass(frozen=True)
class CostConnections:
    selected: original.Connections
    work_units: int
    digest: str
    cost_digest: str
    baseline_cost: int
    selected_cost: int
    baseline_feasible: bool
    weighted_status: str
    selected_strategy: str
    policy: str = POLICY

    @property
    def added(self):
        return self.selected.added

    @property
    def parts(self):
        return self.selected.parts

    def summary(self) -> dict:
        return {'policy': self.policy, 'all_areas_connected': True,
                'new_corridor_tiles': len(self.added), 'corridor_parts': len(self.parts),
                'connected_walkable_tiles': self.selected.walkable_tiles,
                'links': [{'start': list(link.start), 'end': list(link.end),
                           'new_tiles': link.new_tiles, 'steps': link.steps} for link in self.selected.links],
                'work_units': self.work_units, 'connection_digest': self.digest,
                'routing_costs_digest': self.cost_digest,
                'baseline_corridor_cost': self.baseline_cost,
                'selected_corridor_cost': self.selected_cost,
                'baseline_candidate_feasible': self.baseline_feasible,
                'weighted_candidate_status': self.weighted_status,
                'selected_strategy': self.selected_strategy,
                'comparison_order': ['corridor_cost', 'new_tiles', 'parts', 'prefer_original_on_tie'],
                'corridors_rerouted_by_cost': self.selected_strategy in ('cost_first', 'bounded_cost_first'),
                'global_minimum_excavation_proven': False, 'terrain_observed': False,
                'native_pathfinding_proven': False, 'mutation_authority_granted': False}


def _weighted(floors, blocked, entries, costs, root, allowance, work):
    low, size = costs.region
    high = tuple(a + n for a, n in zip(low, size))
    walkable, added, links = set(floors), set(), []
    connected = original._flood(walkable, root, work)
    while connected != walkable:
        work.charge()
        distances = {p: (0, 0, 0) for p in connected}
        queue = [(0, 0, 0, p) for p in sorted(connected)]
        heapq.heapify(queue)
        parents, target = {}, None
        while queue:
            work.charge()
            cost, tiles, steps, current = heapq.heappop(queue)
            if distances[current] != (cost, tiles, steps):
                continue
            if current in walkable and current not in connected:
                target = current
                break
            for point in original.neighbors(current):
                work.charge()
                if point in blocked or not all(a <= p < b for a, p, b in zip(low, point, high)):
                    continue
                fresh = point not in walkable
                score = cost + (costs.at(point) if fresh else 0), tiles + int(fresh), steps + 1
                if point not in distances or score < distances[point]:
                    distances[point], parents[point] = score, current
                    heapq.heappush(queue, (*score, point))
        require(target is not None, 'costed route lost original feasible connectivity')
        path = [target]
        while path[-1] not in connected:
            work.charge()
            path.append(parents[path[-1]])
        path.reverse()
        fresh = set(path) - walkable
        if len(added) + len(fresh) > allowance:
            return None  # Discard the WHOLE alternative, never a routed prefix.
        links.append(original.Link(path[0], path[-1], len(fresh), len(path) - 1))
        added.update(fresh)
        walkable.update(fresh)
        connected = original._flood(walkable, root, work)
    require(all(point in connected for _, point in entries), 'costed route lost an original entry')
    parts = original._partition(added, work)
    return original.Connections(tuple(sorted(added)), parts, tuple(links), len(connected), 0, '')


def connect(floors: set[Point] | frozenset[Point], blocked: set[Point] | frozenset[Point],
            entries: tuple[tuple[str, Point], ...], costs: Costs, *, max_new_tiles: int,
            max_parts: int = 32, root: Point | None = None, guard: Guard = original.idle,
            max_work: int = original.MAX_WORK) -> CostConnections:
    """Compare two complete candidates, never weaken tile/part/protected limits.

    The original connector establishes a baseline first. The cost-first attempt
    can be discarded on geometric capacity alone; cancellation and work failures
    propagate even when a baseline exists. A baseline outside the part limit is
    not an eligible fallback. Caller-owned guards are never caught or renewed.
    """
    work = original._Work(guard, max_work)
    work.charge()
    require(type(max_parts) is int and 0 <= max_parts <= 32, 'invalid remaining corridor part allowance')
    require(type(floors) in (set, frozenset) and 1 <= len(floors) <= 512
            and type(blocked) in (set, frozenset) and len(blocked) <= original.MAX_CELLS,
            'costed connection masks exceed bounds')
    floors, blocked = frozenset(floors), frozenset(blocked)
    require(type(costs) is Costs, 'complete original corridor costs required')
    costs = Costs.parse(costs.json(), work.charge)
    baseline = original.connect(floors, blocked, entries, costs.region, max_new_tiles=max_new_tiles,
                                root=root, guard=work.charge, max_work=max_work)
    entries = tuple(sorted(entries))
    root = entries[0][1] if root is None else root

    def score(candidate):
        cost = 0
        for point in candidate.added:
            work.charge()
            cost += costs.at(point)
        return cost, len(candidate.added), len(candidate.parts)

    baseline_score = score(baseline)
    eligible = len(baseline.parts) <= max_parts
    selected, selected_score, strategy = baseline, baseline_score, 'original'
    status = 'not_needed_zero_cost' if baseline_score[0] == 0 else 'complete'
    bounded = costs.strategy == BOUNDED_STRATEGY
    if baseline_score[0] or (bounded and not eligible):
        if bounded:
            from room_bounded_cost_paths import connect as bounded_connect
            alternative = bounded_connect(floors, blocked, entries, costs, root, max_new_tiles, work)
        else:
            alternative = _weighted(floors, blocked, entries, costs, root, max_new_tiles, work)
        if alternative is None:
            status = 'tile_limit'
        elif len(alternative.parts) > max_parts:
            status = 'part_limit'
        else:
            alternative_score = score(alternative)
            if not eligible or alternative_score < selected_score:
                selected, selected_score = alternative, alternative_score
                strategy = 'bounded_cost_first' if bounded else 'cost_first'
    require(len(selected.parts) <= max_parts, 'no compared corridor candidate fits original part allowance')
    expanded = set()
    for (x, y, z), (width, height, levels) in selected.parts:
        require(levels == 1, 'costed corridor unexpectedly crosses levels')
        for yy in range(y, y + height):
            for xx in range(x, x + width):
                work.charge()
                point = (xx, yy, z)
                require(point not in expanded and point not in floors and point not in blocked,
                        'costed corridor partition overlaps or changes protected geometry')
                expanded.add(point)
    require(expanded == set(selected.added) and len(expanded) <= max_new_tiles,
            'costed corridor partition changed complete target mask')
    walkable = set(floors) | expanded
    require(original._flood(walkable, root, work) == walkable,
            'selected costed corridor is not completely connected')
    policy = BOUNDED_POLICY if bounded else POLICY
    witness = {'policy': policy, 'costs': costs.json(), 'entries': entries, 'root': root,
               'original_floor': sorted(floors), 'blocked': sorted(blocked),
               'max_new_tiles': max_new_tiles, 'max_parts': max_parts,
               'baseline_digest': baseline.digest, 'baseline_score': baseline_score,
               'selected_score': selected_score, 'selected_strategy': strategy,
               'weighted_status': status, 'added': selected.added, 'parts': selected.parts}
    raw = _canonical(witness)
    require(len(raw) <= 128 * 1024, 'complete costed connection witness exceeds bound')
    digest = hashlib.sha256(b'dfmcp-cost-aware-room-connections/1\0' + raw).hexdigest()
    work.charge()
    return CostConnections(selected, work.used, digest, costs.digest, baseline_score[0],
                           selected_score[0], eligible, status, strategy, policy)
