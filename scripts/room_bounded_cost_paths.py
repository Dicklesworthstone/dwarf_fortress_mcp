"""Internal tile-budgeted paths for an already admitted room connection graph.

Keep nondominated (charged cost, new tiles, steps) labels at each vertex. A
cheaper but longer prefix cannot erase a prefix needed to fit the hard tile
allowance. Parent labels are immutable even when a frontier label is dominated.
The caller owns the single shrinking work/deadline guard; no source or effect
is acquired here. Beads: dfmcp-wp-pln01-spatial-blueprint-layout-pwd;
df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

from dataclasses import dataclass
import heapq

import room_connections as original

POLICY = 'pareto-cost-new-tiles-steps-coordinate/1'
MAX_LABELS = 32768  # Includes retired labels retained for immutable parent chains.


@dataclass(frozen=True, slots=True)
class _Label:
    point: original.Point
    cost: int
    tiles: int
    steps: int
    parent: int | None


def _dominates(left: _Label, right: _Label) -> bool:
    # Steps break ties only after both costs tie. A strictly better primary or
    # secondary cost dominates even if its prefix takes more existing-floor hops.
    return (left.cost <= right.cost and left.tiles <= right.tiles
            and ((left.cost, left.tiles) != (right.cost, right.tiles)
                 or left.steps <= right.steps))


def link(walkable, connected, blocked, costs, allowance, work):
    """Least-cost feasible next link, not an optimal multi-component network.

    The enclosing connector has admitted exact cells, region, source component
    and binary costs. None means this current partial network has no next link
    within the remaining tile allowance, NOT global project infeasibility.
    Resource exhaustion raises, including after a complete baseline was found.
    """
    work.charge()
    original.require(type(allowance) is int and 0 <= allowance <= 512
                     and connected and connected <= walkable
                     and len(walkable) + allowance <= 512,
                     'invalid remaining bounded-cost link allowance')
    low, size = costs.region
    high = tuple(a + n for a, n in zip(low, size))
    labels, active, queue = [], [], []
    frontier = {}

    def retain(label):
        if len(labels) >= MAX_LABELS:
            raise original.WorkLimit('bounded-cost corridor label allowance exhausted')
        identity = len(labels)
        labels.append(label)
        active.append(True)
        heapq.heappush(queue, (label.cost, label.tiles, label.steps, label.point, identity))
        return identity

    for point in sorted(connected):
        work.charge()
        frontier[point] = [retain(_Label(point, 0, 0, 0, None))]

    while queue:
        work.charge()
        _, _, _, _, identity = heapq.heappop(queue)
        if not active[identity]:
            continue
        current = labels[identity]
        if current.point in walkable and current.point not in connected:
            path, seen = [], set()
            cursor = identity
            while cursor is not None:
                work.charge()
                label = labels[cursor]
                original.require(label.point not in seen and len(path) < original.MAX_CELLS,
                                 'bounded-cost link parent chain is not a simple bounded path')
                seen.add(label.point)
                path.append(label.point)
                cursor = label.parent
            path.reverse()
            fresh = seen - walkable
            original.require(path[0] in connected and len(path) - 1 == current.steps
                             and len(fresh) == current.tiles <= allowance,
                             'bounded-cost link lost its complete tile accounting')
            cost = 0
            for point in sorted(fresh):
                work.charge()
                cost += costs.at(point)
            original.require(cost == current.cost, 'bounded-cost link cost changed during reconstruction')
            return tuple(path)

        for point in original.neighbors(current.point):
            work.charge()
            if point in blocked or not all(a <= p < b for a, p, b in zip(low, point, high)):
                continue
            fresh = point not in walkable
            tiles = current.tiles + int(fresh)
            if tiles > allowance:
                continue
            candidate = _Label(point, current.cost + (costs.at(point) if fresh else 0),
                               tiles, current.steps + 1, identity)
            previous = frontier.get(point, ())
            dominated = False
            for old in previous:
                work.charge()
                if _dominates(labels[old], candidate):
                    dominated = True
                    break
            if dominated:
                continue
            retained = []
            for old in previous:
                work.charge()
                if _dominates(candidate, labels[old]):
                    active[old] = False
                else:
                    retained.append(old)
            retained.append(retain(candidate))
            frontier[point] = retained
    return None


def connect(floors, blocked, entries, costs, root, allowance, work):
    """Connect every original floor or discard the entire alternative."""
    walkable, added, links = set(floors), set(), []
    connected = original._flood(walkable, root, work)
    while connected != walkable:
        path = link(walkable, connected, blocked, costs, allowance - len(added), work)
        if path is None:
            return None
        fresh = set(path) - walkable
        original.require(fresh and len(added) + len(fresh) <= allowance,
                         'bounded-cost corridor failed to extend within the original allowance')
        links.append(original.Link(path[0], path[-1], len(fresh), len(path) - 1))
        added.update(fresh)
        walkable.update(fresh)
        connected = original._flood(walkable, root, work)
    original.require(all(point in connected for _, point in entries),
                     'bounded-cost corridor omitted an original entry')
    parts = original._partition(added, work)
    work.charge()
    return original.Connections(tuple(sorted(added)), parts, tuple(links), len(connected), 0, '')
