"""Whole-plan furniture assignment using bounded, observed candidate routes.

This is an additive read model, not native pathfinding or placement authority.
Retain every existing material/subtype/z/Manhattan constraint, then minimize
summed shortest candidate-route lengths and the lexical slot/item-ID vector.
Beads: df-franken-storage-mvcc-54h.4; df-dfhack-bridge-plane-c-pic.3/.4.
"""
from __future__ import annotations

from collections import deque
from dataclasses import dataclass
import hashlib
import heapq
import struct

from furniture_allocation import (
    Candidate, Guard, MAX_ITEMS, Request, _maximum_matching, _minimum_cost,
    distance, idle,
)
from furniture_plan import FurniturePlan, Step, canonical, integer, require

POLICY = 'dfmcp.route-furniture-allocation/1'
ROUTE_POLICY = 'observed-dry-cardinal-floor-stairs/1'
PATH_POLICY = 'target-rooted-bfs-stable-tile-index/1'
MAX_TILES = 16384
MAX_ROUTE_STEPS = MAX_TILES - 1
MAX_CAPTURE_BYTES = 575 + 20 * MAX_TILES
MAX_WORK = 20_000_000
MAX_OUTPUT = 49152


class Work:
    """One allowance spans validation, all target BFSs, matching and witnesses."""
    def __init__(self, maximum: int, guard: Guard):
        self.maximum = integer(maximum, 1, MAX_WORK)
        self.used, self.guard = 0, guard
        guard()

    def __call__(self) -> None:
        require(self.used < self.maximum, 'route allocation work allowance exhausted')
        self.used += 1
        if self.used == 1 or self.used % 128 == 0:
            self.guard()

    def check(self) -> None:
        self.guard()


class Reader:
    def __init__(self, raw: bytes):
        require(type(raw) is bytes and 1 <= len(raw) <= MAX_CAPTURE_BYTES,
                'invalid bounded map capture')
        self.raw, self.offset = raw, 0

    def take(self, count: int) -> bytes:
        end = self.offset + count
        require(0 <= count and end <= len(self.raw), 'truncated map capture')
        value = self.raw[self.offset:end]
        self.offset = end
        return value

    def number(self, fmt: str) -> int:
        return struct.unpack('>' + fmt, self.take(struct.calcsize('>' + fmt)))[0]


@dataclass(frozen=True)
class Terrain:
    """Derived only by complete map/1.5 decoding; no supplied JSON tile claims."""
    digest: str
    tick: int
    paused: bool
    folder: str
    site: int
    dimensions: tuple[int, int, int]
    origin: tuple[int, int, int]
    size: tuple[int, int, int]
    shapes: tuple[int, ...]  # zero means excluded, never an unknown floor
    reasons: tuple[str, ...]

    def index(self, position: tuple[int, int, int]) -> int | None:
        relative = tuple(a - b for a, b in zip(position, self.origin))
        if any(not 0 <= n < bound for n, bound in zip(relative, self.size)):
            return None
        x, y, z = relative
        return (z * self.size[1] + y) * self.size[0] + x

    def position(self, index: int) -> tuple[int, int, int]:
        x, y, _ = self.size
        return (self.origin[0] + index % x,
                self.origin[1] + (index // x) % y,
                self.origin[2] + index // (x * y))

    def reason(self, position: tuple[int, int, int]) -> str:
        index = self.index(position)
        return 'outside_observed_region' if index is None else self.reasons[index]

    def adjacency(self, work: Work) -> tuple[tuple[int, ...], ...]:
        graph = []
        for current, shape in enumerate(self.shapes):
            work()
            neighbors = []
            if shape:
                position = self.position(current)
                for axis in range(3):
                    for delta in (-1, 1):
                        work()
                        next_position = list(position)
                        next_position[axis] += delta
                        target = self.index(tuple(next_position))
                        if target is None or not self.shapes[target]:
                            continue
                        other = self.shapes[target]
                        if axis == 2:
                            lower, upper = (shape, other) if delta > 0 else (other, shape)
                            if lower not in (6, 8) or upper not in (7, 8):
                                continue
                        neighbors.append(target)
            graph.append(tuple(sorted(neighbors)))
        return tuple(graph)


def decode_terrain(raw: bytes, work: Guard = idle) -> Terrain:
    """Strict unchanged DFMM1500 layout; hidden cells have no attribute bytes.

    The digest names raw bytes only, not acquisition, manifest or authorization.
    The enclosing live owner independently binds these to its selected source.
    """
    work()
    r = Reader(raw)
    require(r.take(8) == b'DFMM1500', 'unsupported terrain profile')
    year, year_tick = r.number('I'), integer(r.number('I'), 0, 403199)
    paused = bool(integer(r.number('B'), 0, 1))
    site = integer(r.number('I'), 0, 2**31 - 1)
    folder_raw = r.take(integer(r.number('H'), 1, 512))
    require(b'\0' not in folder_raw, 'invalid map folder')
    folder = folder_raw.decode('utf-8')
    dimensions = tuple(integer(r.number('I'), 1, 32768) for _ in range(3))
    origin = tuple(integer(r.number('I'), 0, 32767) for _ in range(3))
    size = tuple(integer(r.number('I'), 1, 128) for _ in range(3))
    volume = size[0] * size[1] * size[2]
    require(volume <= MAX_TILES and all(a + b <= c for a, b, c in zip(origin, size, dimensions)),
            'terrain selection exceeds map or tile bound')
    require(r.number('I') == volume, 'terrain selection omitted cells')
    shapes, reasons = [], []
    for _ in range(volume):
        work()
        presence = integer(r.number('B'), 0, 2)
        shape = 0
        if presence != 2:
            reason = 'hidden' if presence == 1 else 'unallocated'
        else:
            values = struct.unpack('>IBBBBBBBIHH', r.take(19))
            integer(values[0], 0, 2**31 - 1)
            for value, maximum in zip(values[1:8], (8, 7, 1, 3, 7, 7, 3)):
                integer(value, 0, maximum)
            _, native_shape, depth, _, _, _, building, units, walkable, _, _ = values
            reason = ('unsupported_shape' if native_shape not in (3, 6, 7, 8) else
                      'liquid' if depth else 'building_occupied' if building else
                      'unit_occupied' if units else 'walkability_unknown' if not walkable else
                      'candidate')
            if reason == 'candidate':
                shape = native_shape
        shapes.append(shape)
        reasons.append(reason)
    require(r.offset == len(raw), 'trailing terrain bytes')
    work()
    return Terrain(hashlib.sha256(raw).hexdigest(), year * 403200 + year_tick,
                   paused, folder, site, dimensions, origin, size, tuple(shapes), tuple(reasons))


def distances(graph: tuple[tuple[int, ...], ...], root: int, work: Guard) -> tuple[list[int], list[int]]:
    """Unweighted BFS; stable flattened-index ties, never greedy item assignment."""
    steps, parent = [-1] * len(graph), [-1] * len(graph)
    steps[root], parent[root] = 0, root
    queue = deque([root])
    while queue:
        current = queue.popleft()
        work()
        for following in graph[current]:
            work()
            if parent[following] == -1:
                parent[following] = current
                steps[following] = steps[current] + 1
                queue.append(following)
    return steps, parent


def path_digest(terrain: Terrain, parent: list[int], source: int, target: int,
                steps: int, work: Guard) -> str:
    """Streaming commitment to the whole shortest route, not a truncated path."""
    require(0 <= steps < len(parent), 'invalid selected route length')
    digest = hashlib.sha256(b'dfmcp-furniture-candidate-route/1\0')
    digest.update(PATH_POLICY.encode('ascii') + b'\0' + bytes.fromhex(terrain.digest))
    digest.update(struct.pack('>III', source, target, steps + 1))
    current = source
    for position in range(steps + 1):
        work()
        require(0 <= current < len(parent), 'route witness escaped terrain')
        digest.update(struct.pack('>I', current))
        if position == steps:
            require(current == target, 'route witness did not reach target')
        else:
            require(current != target, 'route witness ended early')
            current = parent[current]
    return digest.hexdigest()


def allocate_routes(request: Request, candidates: tuple[Candidate, ...], terrain_raw: bytes,
                    guard: Guard = idle, *, max_route_steps: int = MAX_ROUTE_STEPS,
                    maximum_work: int = MAX_WORK) -> dict:
    """Allocate ALL slots under original constraints and the captured route model.

    N best routed items per slot preserve the global optimum and complete Hall
    witnesses. Retain only one BFS frontier at a time: O(tiles + items + N*N)
    working space, never a slots-by-items or slots-by-tiles matrix. Paths for the
    chosen items are rechecked on a second bounded BFS before publication.
    """
    work = Work(maximum_work, guard)
    integer(max_route_steps, 0, MAX_ROUTE_STEPS)
    require(type(request) is Request, 'invalid route allocation request')
    request.__post_init__()
    require(type(candidates) is tuple and len(candidates) <= MAX_ITEMS, 'candidate bound exceeded')
    terrain = decode_terrain(terrain_raw, work)
    require((terrain.folder, terrain.site) == (request.folder, request.site),
            'route terrain belongs to another fortress')
    by_id = {}
    for candidate in candidates:
        work()
        require(type(candidate) is Candidate, 'invalid route candidate')
        candidate.__post_init__()
        require(candidate.id not in by_id, 'duplicate route candidate identity')
        by_id[candidate.id] = candidate
    work.check()
    ordered = sorted(by_id)
    inventory = hashlib.sha256(b'dfmcp-route-furniture-candidates/1\0')
    inventory.update(struct.pack('>I', len(ordered)))
    for identity in ordered:
        work()
        value = canonical(by_id[identity].json())
        inventory.update(struct.pack('>I', len(value)) + value)
    targets = [{'slot': slot.name, 'reason': terrain.reason(slot.target)} for slot in request.slots]
    result = {'schema': 'dfmcp.route-furniture-allocation/1', 'policy': POLICY,
              'route_policy': ROUTE_POLICY, 'path_policy': PATH_POLICY,
              'request_digest': request.digest, 'candidate_digest': inventory.hexdigest(),
              'terrain': {'sha256': terrain.digest, 'bytes': len(terrain_raw),
                          'region': {'origin': list(terrain.origin), 'size': list(terrain.size)},
                          'dimensions': list(terrain.dimensions), 'world_folder': terrain.folder,
                          'site': terrain.site, 'game_tick': terrain.tick, 'paused': terrain.paused},
              'status': 'target_unestablished', 'targets': targets,
              'candidate_count': len(candidates), 'maximum_assignable': None,
              'max_route_steps': max_route_steps, 'compatible_counts': [],
              'objective': 'total_candidate_route_steps_then_lexical_slot_item_ids',
              'original_manhattan_constraints_preserved': True, 'same_level_endpoints_only': True,
              'plan': None, 'plan_digest': None, 'assignments': [], 'total_route_steps': None,
              'shortage': None, 'native_acquisition_proven': False,
              'placement_eligibility_proven': False, 'pathfinding_proven': False,
              'future_routes_preserved': False, 'global_absence_proven': False,
              'items_reserved': False, 'game_effect_performed': False, 'production_admitted': False}
    # An unobserved/unsupported target cannot become a certified zero supply.
    if all(target['reason'] == 'candidate' for target in targets):
        graph = terrain.adjacency(work)
        excluded = set(request.excluded_items)
        edges, counts = [], []
        n = len(request.slots)
        for slot in request.slots:
            work.check()
            root = terrain.index(slot.target)
            steps, _ = distances(graph, root, work)
            heap, count = [], 0
            for identity in ordered:
                work()
                item = by_id[identity]
                if identity in excluded or distance(slot, item) is None:
                    continue
                index = terrain.index(item.position)
                if index is None or not terrain.shapes[index] or not 0 <= steps[index] <= max_route_steps:
                    continue
                count += 1
                entry = (-steps[index], -identity)
                if len(heap) < n:
                    heapq.heappush(heap, entry)
                elif entry > heap[0]:
                    heapq.heapreplace(heap, entry)
            edges.append({-identity: -cost for cost, identity in heap})
            counts.append({'slot': slot.name, 'count': count})
        edges = tuple(edges)
        owners, shortage = _maximum_matching(edges, work)
        result.update(status='shortage' if shortage is not None else 'allocated',
                      maximum_assignable=len(owners), compatible_counts=counts)
        if shortage is not None:
            result['shortage'] = {'slots': [request.slots[i].name for i in shortage['rows']],
                                  'candidate_items': shortage['candidate_items'],
                                  'missing': shortage['missing'],
                                  'scope': 'complete_supplied_candidate_graph_in_captured_route_model'}
        else:
            chosen = _minimum_cost(edges, work)
            plan = FurniturePlan(tuple(Step(slot.name, slot.kind, identity, slot.target, slot.after)
                                       for slot, identity in zip(request.slots, chosen)))
            plan.check_dimensions(terrain.dimensions)
            total, assignments = 0, []
            for row, (slot, identity) in enumerate(zip(request.slots, chosen)):
                work.check()
                item = by_id[identity]
                source, target = terrain.index(item.position), terrain.index(slot.target)
                steps, parent = distances(graph, target, work)
                require(steps[source] == edges[row][identity], 'selected route cost differs from allocation')
                witness = path_digest(terrain, parent, source, target, steps[source], work)
                assignments.append({'slot': slot.name, **item.json(), 'target': list(slot.target),
                                    'manhattan_distance': distance(slot, item),
                                    'route_steps': steps[source], 'route_sha256': witness})
                total += steps[source]
            result.update(plan=plan.json(), plan_digest=plan.digest, assignments=assignments,
                          total_route_steps=total)
    work.check()
    result['work_units'] = work.used
    result['maximum_work'] = maximum_work
    result['decision_digest'] = hashlib.sha256(
        b'dfmcp-route-furniture-decision/1\0' + canonical(result)).hexdigest()
    require(len(canonical(result)) <= MAX_OUTPUT, 'complete route allocation exceeds output bound')
    work.check()
    return result
