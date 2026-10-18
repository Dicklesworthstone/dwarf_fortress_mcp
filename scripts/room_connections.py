"""Bounded connections between intended room floors, never native pathfinding.

Each greedy link minimizes new excavation, then cardinal path length. Stable
coordinate ordering breaks ties. If greedy links exceed the tile allowance,
retry the complete original geometry with bounded minimum-new-tile subset DP.
The fallback shares the original work allowance and never returns a partial tree.
The caller supplies required walls, furniture and exclusions as blocked cells.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5 (WP-05/WP-10).
"""
from __future__ import annotations

from collections import deque
from dataclasses import dataclass
import hashlib
import heapq
import json
from typing import Callable

POLICY = 'dfmcp.room-project-connections/1'
MINIMUM_POLICY = 'dfmcp.room-project-minimum-connections/1'
MAX_CELLS = 1024
MAX_WORK = 100_000
Point = tuple[int, int, int]
Region = tuple[Point, Point]
Guard = Callable[[], None]


def idle() -> None:
    pass


def require(ok: bool, message: str) -> None:
    if not ok:
        raise ValueError(message)


def point(value: object) -> bool:
    return (type(value) is tuple and len(value) == 3
            and all(type(n) is int and 1 <= n <= 32766 for n in value))


def neighbors(p: Point) -> tuple[Point, ...]:
    x, y, z = p
    return (x - 1, y, z), (x, y - 1, z), (x, y + 1, z), (x + 1, y, z)


class WorkLimit(ValueError):
    """Routing stopped without proving either a connection or impossibility."""


class _Work:
    def __init__(self, guard: Guard, maximum: int) -> None:
        require(type(maximum) is int and 1 <= maximum <= MAX_WORK,
                'invalid room-connection work allowance')
        self.guard, self.maximum, self.used = guard, maximum, 0

    def charge(self) -> None:
        if self.used >= self.maximum:
            raise WorkLimit('room connections exhausted bounded work')
        self.used += 1
        self.guard()


def _flood(floors: set[Point], start: Point, work: _Work) -> set[Point]:
    seen, queue = {start}, deque([start])
    while queue:
        work.charge()
        for p in neighbors(queue.popleft()):
            work.charge()
            if p in floors and p not in seen:
                seen.add(p)
                queue.append(p)
    return seen


def _partition(cells: set[Point], work: _Work) -> tuple[Region, ...]:
    """Merge equal horizontal runs; emit exactly the added mask, never its hull."""
    rows: dict[tuple[int, int], list[int]] = {}
    for x, y, z in sorted(cells):
        work.charge()
        rows.setdefault((z, y), []).append(x)
    runs: dict[tuple[int, int, int], list[int]] = {}
    for (z, y), xs in sorted(rows.items()):
        start = previous = xs[0]
        for x in xs[1:]:
            work.charge()
            if x != previous + 1:
                runs.setdefault((z, start, previous - start + 1), []).append(y)
                start = x
            previous = x
        runs.setdefault((z, start, previous - start + 1), []).append(y)
    parts = []
    for (z, x, width), ys in sorted(runs.items()):
        start = previous = ys[0]
        for y in ys[1:]:
            work.charge()
            if y != previous + 1:
                parts.append(((x, start, z), (width, previous - start + 1, 1)))
                start = y
            previous = y
        parts.append(((x, start, z), (width, previous - start + 1, 1)))
    return tuple(sorted(parts, key=lambda p: (p[0][::-1], p[1][::-1])))


@dataclass(frozen=True)
class Link:
    start: Point
    end: Point
    new_tiles: int
    steps: int


@dataclass(frozen=True)
class Connections:
    added: tuple[Point, ...]
    parts: tuple[Region, ...]
    links: tuple[Link, ...]
    walkable_tiles: int
    work_units: int
    digest: str
    minimum_excavation: bool = False
    component_count: int = 0
    subset_count: int = 0

    def summary(self) -> dict:
        out = {'policy': POLICY, 'all_areas_connected': True,
                'new_corridor_tiles': len(self.added), 'corridor_parts': len(self.parts),
                'connected_walkable_tiles': self.walkable_tiles,
                'links': [{'start': list(link.start), 'end': list(link.end),
                           'new_tiles': link.new_tiles, 'steps': link.steps} for link in self.links],
                'work_units': self.work_units, 'connection_digest': self.digest,
                'global_minimum_excavation_proven': False,
                'native_pathfinding_proven': False, 'terrain_observed': False}
        if self.minimum_excavation:
            out.update(policy=MINIMUM_POLICY,
                fallback_reason='greedy_corridors_exceeded_tile_allowance',
                minimum_excavation_in_region_proven=True,
                minimum_excavation_scope='fixed_same_level_floor_and_protected_masks',
                original_floor_components=self.component_count,
                subset_states=self.subset_count,
                rectangle_count_optimal=False)
        return out


def connect(floors: set[Point] | frozenset[Point], blocked: set[Point] | frozenset[Point],
            entries: tuple[tuple[str, Point], ...], region: Region, *,
            max_new_tiles: int, guard: Guard = idle, max_work: int = MAX_WORK,
            root: Point | None = None, minimum_fallback: bool = True) -> Connections:
    """Connect the complete intended floor in one fixed same-level capture.

    Inputs are copied only after size admission. No vertical movement or outside
    capture detour is invented. Refusal/cancellation returns no partial result.
    Legacy search replay can disable the fallback without changing greedy work.
    The injected guard belongs to the caller's enclosing authority/work deadline.
    """
    work = _Work(guard, max_work)
    work.charge()
    require(type(minimum_fallback) is bool, 'minimum fallback selection must be an exact Boolean')
    require(type(floors) in (set, frozenset) and 1 <= len(floors) <= 512
            and type(blocked) in (set, frozenset) and len(blocked) <= MAX_CELLS,
            'room-connection mask exceeds bounds')
    require(type(entries) is tuple and 1 <= len(entries) <= 8,
            'room connections require 1..8 named entries')
    require(type(region) is tuple and len(region) == 2 and point(region[0])
            and type(region[1]) is tuple and len(region[1]) == 3
            and all(type(n) is int and 1 <= n <= 128 for n in region[1]),
            'invalid connection capture')
    low, size = region
    require(size[2] == 1 and size[0] * size[1] <= MAX_CELLS
            and all(a + n <= 32767 for a, n in zip(low, size)),
            'connections need one bounded same-level capture with native halo')
    require(type(max_new_tiles) is int and 0 <= max_new_tiles <= 512
            and len(floors) + max_new_tiles <= 512, 'invalid corridor tile allowance')
    high = tuple(a + n for a, n in zip(low, size))

    def within(p: Point) -> bool:
        return all(a <= n < b for a, n, b in zip(low, p, high))

    for p in floors | blocked:
        work.charge()
        require(point(p) and within(p), 'connection cell outside capture')
    require(not floors & blocked, 'intended circulation crosses a protected cell')
    names = set()
    for entry in entries:
        work.charge()
        require(type(entry) is tuple and len(entry) == 2 and type(entry[0]) is str
                and 1 <= len(entry[0]) <= 24 and entry[0].isascii()
                and all(c.isalnum() or c in '._-' for c in entry[0])
                and entry[0] not in names and point(entry[1]) and entry[1] in floors,
                'invalid or duplicate room entry')
        names.add(entry[0])
    entries = tuple(sorted(entries))
    explicit_root = root
    if root is None:
        root = entries[0][1]
    else:
        work.charge()
        require(point(root) and root in floors, 'connection root must be an admitted floor cell')
    original, walkable, added, links, paths = set(floors), set(floors), set(), [], []
    connected = _flood(walkable, root, work)
    while connected != walkable:
        distances = {p: (0, 0) for p in connected}
        parents: dict[Point, Point] = {}
        queue = [(0, 0, p) for p in sorted(connected)]
        heapq.heapify(queue)
        target = None
        while queue:
            work.charge()
            excavation, steps, current = heapq.heappop(queue)
            if distances[current] != (excavation, steps):
                continue
            if current in walkable and current not in connected:
                target = current
                break
            for p in neighbors(current):
                work.charge()
                if not within(p) or p in blocked:
                    continue
                cost = excavation + int(p not in walkable), steps + 1
                if p not in distances or cost < distances[p]:
                    distances[p] = cost
                    parents[p] = current
                    heapq.heappush(queue, (*cost, p))
        require(target is not None, 'no corridor connects the complete original room project')
        path = [target]
        while path[-1] not in connected:
            work.charge()
            path.append(parents[path[-1]])
        path.reverse()
        fresh = set(path) - walkable
        if len(added) + len(fresh) > max_new_tiles:
            require(minimum_fallback, 'complete room project exceeds corridor tile allowance')
            # Greedy exhaustion is not an impossibility proof. Start from the
            # ORIGINAL masks, not the locally selected prefix; reuse this exact
            # work owner so failure cannot manufacture another budget or plan.
            return _minimum_connection(original, set(blocked), entries, region,
                                       root, max_new_tiles, work)
        links.append(Link(path[0], path[-1], len(fresh), len(path) - 1))
        paths.append(path)
        added.update(fresh)
        walkable.update(fresh)
        connected = _flood(walkable, root, work)
    require(all(p in connected for _, p in entries), 'room entry lost from connected project')
    parts = _partition(added, work)
    witness = {'policy': POLICY, 'region': region, 'entries': entries,
               'original_floor': sorted(original), 'blocked': sorted(blocked),
               'paths': paths, 'walkable': sorted(connected), 'parts': parts}
    if explicit_root is not None:
        witness['root'] = explicit_root
    raw = json.dumps(witness, sort_keys=True, separators=(',', ':'), ensure_ascii=True,
                     allow_nan=False).encode('ascii')
    require(len(raw) <= 128 * 1024, 'room-connection witness exceeds bound')
    digest = hashlib.sha256(b'dfmcp-room-project-connections/1\0' + raw).hexdigest()
    work.charge()
    return Connections(tuple(sorted(added)), parts, tuple(links), len(connected), work.used, digest)



def _minimum_connection(original: set[Point], blocked: set[Point],
                        entries: tuple[tuple[str, Point], ...], region: Region,
                        root: Point, allowance: int, work: _Work) -> Connections:
    from room_connection_solver import ComponentLimit, TIE_POLICY, minimum_tiles

    (x, y, z), (width, height, _) = region
    admitted = set()
    for px in range(x, x + width):
        for py in range(y, y + height):
            work.charge()
            point = px, py, z
            if point not in blocked:
                admitted.add(point)
    try:
        minimum = minimum_tiles(original, admitted, root, allowance, work.charge)
    except ComponentLimit as error:
        # The layout solver already propagates this resource-exhaustion type.
        # It must not misreport an unsearched large component set as no layout.
        raise WorkLimit(str(error)) from error
    added = set(minimum.added)
    final = original | added
    require(not added & original and not final & blocked and final <= admitted
            and len(added) <= allowance and len(final) <= 512,
            'minimum connection escaped the original geometry or tile allowance')
    require(_flood(final, root, work) == final,
            'minimum connection did not connect every original floor')

    # Re-express the selected union as exact, reproducible connection paths.
    # No new search outside that minimum mask, hull expansion or partial output.
    built, links, paths = set(original), [], []
    connected = _flood(built, root, work)
    while connected != built:
        queue = deque(sorted(connected))
        parent = {p: None for p in connected}
        target = None
        while queue:
            work.charge()
            current = queue.popleft()
            if current in original and current not in connected:
                target = current
                break
            for following in neighbors(current):
                work.charge()
                if following in final and following not in parent:
                    parent[following] = current
                    queue.append(following)
        require(target is not None, 'minimum connection path reconstruction failed')
        path = [target]
        while parent[path[-1]] is not None:
            work.charge()
            path.append(parent[path[-1]])
        path.reverse()
        fresh = set(path) - built
        links.append(Link(path[0], path[-1], len(fresh), len(path) - 1))
        paths.append(path)
        built.update(fresh)
        connected = _flood(built, root, work)
    require(built == final and all(p in connected for _, p in entries),
            'minimum connection retained unused cells or lost an original entry')
    parts = _partition(added, work)
    witness = {'policy': MINIMUM_POLICY, 'region': region, 'entries': entries,
               'root': root, 'original_floor': sorted(original),
               'blocked': sorted(blocked), 'paths': paths,
               'walkable': sorted(connected), 'parts': parts,
               'minimum_new_tiles': minimum.cost,
               'component_representatives': minimum.representatives,
               'subset_count': minimum.subset_count, 'tie_policy': TIE_POLICY}
    raw = json.dumps(witness, sort_keys=True, separators=(',', ':'), ensure_ascii=True,
                     allow_nan=False).encode('ascii')
    require(len(raw) <= 128 * 1024, 'minimum room-connection witness exceeds bound')
    digest = hashlib.sha256(b'dfmcp-room-project-minimum-connections/1\0' + raw).hexdigest()
    work.charge()
    return Connections(tuple(sorted(added)), parts, tuple(links), len(connected), work.used,
                       digest, True, len(minimum.representatives), minimum.subset_count)
