"""Bounded, deterministic whole-plan furniture allocation; no game effects.

Minimize total same-level Manhattan distance, then the item-ID vector in lexical
slot order. A shortage never emits a partial executable plan. Candidate facts are
historical projections: they do not establish pathfinding, current availability,
native placement eligibility, reservations, or authority to prepare/commit.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import heapq
from typing import Callable

from furniture_plan import (
    MAX_BYTES, MAX_STEPS, FurniturePlan, Step, canonical, decode_json, integer, require,
)

REQUEST_SCHEMA = 'dfmcp.furniture-request/1'
MAX_ITEMS = 65536
MAX_DISTANCE = 65532
MAX_ID = 2147483646
Guard = Callable[[], None]


def idle() -> None:
    """Pure callers still have fixed structural bounds; live callers add a budget."""


def material_pair(value: object) -> tuple[int, int]:
    require(type(value) is tuple and len(value) == 2, 'material requires exact type and index')
    integer(value[0], 0, 2147483647)
    integer(value[1], -1, 2147483647)
    return value


@dataclass(frozen=True)
class Slot:
    name: str
    kind: str
    target: tuple[int, int, int]
    after: tuple[str, ...] = ()
    material: tuple[int, int] | None = None
    subtype: int | None = None
    max_distance: int = MAX_DISTANCE

    def __post_init__(self) -> None:
        step = Step(self.name, self.kind, 0, self.target, self.after)
        object.__setattr__(self, 'after', step.after)
        if self.material is not None:
            material_pair(self.material)
        if self.subtype is not None:
            integer(self.subtype, -1, 2147483647)
        integer(self.max_distance, 0, MAX_DISTANCE)

    def json(self) -> dict:
        return {'name': self.name, 'kind': self.kind, 'target': list(self.target),
                'after': list(self.after), 'material': None if self.material is None else list(self.material),
                'subtype': self.subtype, 'max_distance': self.max_distance}

    @classmethod
    def from_json(cls, value: object) -> Slot:
        require(type(value) is dict and {'name', 'kind', 'target'} <= set(value)
                and set(value) <= {'name', 'kind', 'target', 'after', 'material', 'subtype', 'max_distance'},
                'invalid allocation slot fields')
        require(type(value['target']) is list and type(value.get('after', [])) is list,
                'invalid allocation target or dependency array')
        material = value.get('material')
        require(material is None or type(material) is list, 'invalid material array')
        return cls(value['name'], value['kind'], tuple(value['target']), tuple(value.get('after', [])),
                   None if material is None else tuple(material), value.get('subtype'),
                   value.get('max_distance', MAX_DISTANCE))


@dataclass(frozen=True)
class Request:
    folder: str
    site: int
    slots: tuple[Slot, ...]
    excluded_items: tuple[int, ...] = ()

    def __post_init__(self) -> None:
        require(type(self.folder) is str and '\0' not in self.folder
                and 1 <= len(self.folder.encode('utf-8')) <= 512, 'invalid fortress folder')
        integer(self.site, 0, 2147483647)
        require(type(self.slots) is tuple and 1 <= len(self.slots) <= MAX_STEPS
                and all(type(s) is Slot for s in self.slots), 'request needs 1..32 slots')
        for slot in self.slots:
            slot.__post_init__()
        # Reuse the actual executable plan's geometry, names and DAG contract.
        FurniturePlan(tuple(Step(s.name, s.kind, i, s.target, s.after) for i, s in enumerate(self.slots)))
        object.__setattr__(self, 'slots', tuple(sorted(self.slots, key=lambda s: s.name)))
        require(type(self.excluded_items) is tuple and len(self.excluded_items) <= 4096,
                'excluded item allowance exceeded')
        for identity in self.excluded_items:
            integer(identity, 0, MAX_ID)
        require(len(set(self.excluded_items)) == len(self.excluded_items), 'duplicate excluded item')
        object.__setattr__(self, 'excluded_items', tuple(sorted(self.excluded_items)))
        require(len(canonical(self.json())) <= MAX_BYTES, 'normalized request exceeds 16 KiB')

    def json(self) -> dict:
        return {'schema': REQUEST_SCHEMA, 'world_folder': self.folder, 'site': self.site,
                'slots': [s.json() for s in self.slots], 'excluded_items': list(self.excluded_items)}

    @property
    def digest(self) -> str:
        return hashlib.sha256(b'dfmcp-furniture-request/1\0' + canonical(self.json())).hexdigest()

    @classmethod
    def from_json(cls, value: object) -> Request:
        require(type(value) is dict and {'schema', 'world_folder', 'site', 'slots'} <= set(value)
                and set(value) <= {'schema', 'world_folder', 'site', 'slots', 'excluded_items'}
                and value['schema'] == REQUEST_SCHEMA and type(value['slots']) is list
                and 1 <= len(value['slots']) <= MAX_STEPS, 'invalid allocation request')
        require(type(value.get('excluded_items', [])) is list, 'excluded items must be an array')
        return cls(value['world_folder'], value['site'], tuple(Slot.from_json(s) for s in value['slots']),
                   tuple(value.get('excluded_items', [])))

    @classmethod
    def decode(cls, raw: bytes) -> Request:
        return cls.from_json(decode_json(raw))


@dataclass(frozen=True)
class Candidate:
    id: int
    kind: str
    position: tuple[int, int, int]
    material: tuple[int, int]
    subtype: int

    def __post_init__(self) -> None:
        integer(self.id, 0, MAX_ID)
        require(type(self.kind) is str and self.kind in ('bed', 'chair', 'table'), 'unsupported candidate kind')
        require(type(self.position) is tuple and len(self.position) == 3, 'invalid candidate position')
        for coordinate in self.position:
            integer(coordinate, 0, 32767)
        material_pair(self.material)
        integer(self.subtype, -1, 2147483647)

    def json(self) -> dict:
        return {'item': self.id, 'kind': self.kind, 'position': list(self.position),
                'material': list(self.material), 'subtype': self.subtype}


def distance(slot: Slot, item: Candidate) -> int | None:
    if (slot.kind != item.kind or slot.target[2] != item.position[2]
            or (slot.material is not None and slot.material != item.material)
            or (slot.subtype is not None and slot.subtype != item.subtype)):
        return None
    value = abs(slot.target[0] - item.position[0]) + abs(slot.target[1] - item.position[1])
    return value if value <= slot.max_distance else None


def _maximum_matching(edges: tuple[dict[int, int], ...], guard: Guard) -> tuple[dict[int, int], dict | None]:
    """Maximum matching plus an exact Hall-deficiency witness, not greedy counts."""
    owners: dict[int, int] = {}

    def visit(row: int, seen: set[int]) -> bool:
        for identity in sorted(edges[row]):
            guard()
            if identity in seen:
                continue
            seen.add(identity)
            if identity not in owners or visit(owners[identity], seen):
                owners[identity] = row
                return True
        return False

    for row in range(len(edges)):
        guard()
        visit(row, set())
    if len(owners) == len(edges):
        return owners, None
    rows = set(range(len(edges))) - set(owners.values())
    queue, neighbors = sorted(rows), set()
    for row in queue:
        for identity in sorted(edges[row]):
            guard()
            neighbors.add(identity)
            require(identity in owners, 'internal augmenting path left unmatched')
            owner = owners[identity]
            if owner not in rows:
                rows.add(owner)
                queue.append(owner)
    require(len(rows) > len(neighbors), 'invalid shortage witness')
    return owners, {'rows': sorted(rows), 'candidate_items': sorted(neighbors),
                    'missing': len(rows) - len(neighbors)}


def _minimum_cost(edges: tuple[dict[int, int], ...], guard: Guard) -> tuple[int, ...]:
    """Rectangular Hungarian assignment with exact integer lexicographic costs."""
    n = len(edges)
    identities = sorted({identity for row in edges for identity in row})
    m = len(identities)
    require(n <= m <= n * n, 'invalid reduced allocation graph')
    base = 2**31
    scale = base**n
    infinity = (n * MAX_DISTANCE + 2) * scale
    costs = [{identity: value * scale + identity * base**(n - row - 1)
              for identity, value in edge.items()} for row, edge in enumerate(edges)]
    u, v, p, way = [0] * (n + 1), [0] * (m + 1), [0] * (m + 1), [0] * (m + 1)
    for row in range(1, n + 1):
        p[0], column = row, 0
        minimum, used = [infinity] * (m + 1), [False] * (m + 1)
        while True:
            guard()
            used[column] = True
            active = p[column]
            delta, following = infinity, 0
            for j in range(1, m + 1):
                guard()
                if not used[j]:
                    cost = costs[active - 1].get(identities[j - 1], infinity) - u[active] - v[j]
                    if cost < minimum[j]:
                        minimum[j], way[j] = cost, column
                    if minimum[j] < delta:
                        delta, following = minimum[j], j
            require(following != 0, 'allocation augmenting column unavailable')
            for j in range(m + 1):
                if used[j]:
                    u[p[j]] += delta
                    v[j] -= delta
                else:
                    minimum[j] -= delta
            column = following
            if p[column] == 0:
                break
        while column:
            previous = way[column]
            p[column] = p[previous]
            column = previous
    chosen = [-1] * n
    for column in range(1, m + 1):
        if p[column]:
            chosen[p[column] - 1] = identities[column - 1]
    require(all(identity in edges[row] for row, identity in enumerate(chosen))
            and len(set(chosen)) == n, 'invalid optimal allocation')
    return tuple(chosen)


def allocate(request: Request, candidates: tuple[Candidate, ...], guard: Guard = idle) -> dict:
    """Allocate all slots, or return a bounded shortage with no executable plan.

    Keep each slot's N best compatible items for N slots. This preserves an
    optimum: among its first N candidates at least one is unused by the other
    N-1 slots. The reduced union has at most N*N items, independent of the full
    65,536-item inventory. A deficient Hall set has fewer than N neighbors, so
    none of its lists was truncated: the shortage witness covers the full input.
    """
    guard()
    require(type(request) is Request, 'invalid allocation request type')
    request.__post_init__()
    require(type(candidates) is tuple and len(candidates) <= MAX_ITEMS, 'candidate bound exceeded')
    n, seen, excluded = len(request.slots), set(), set(request.excluded_items)
    heaps: list[list[tuple[int, int]]] = [[] for _ in request.slots]
    counts = [0] * n
    # One inventory index; never an O(N*inventory) distance matrix.
    by_id = {}
    for item in candidates:
        guard()
        require(type(item) is Candidate, 'invalid candidate type')
        item.__post_init__()
        require(item.id not in seen, 'duplicate candidate item identity')
        seen.add(item.id)
        by_id[item.id] = item
        if item.id in excluded:
            continue
        for row, slot in enumerate(request.slots):
            guard()
            value = distance(slot, item)
            if value is None:
                continue
            counts[row] += 1
            entry = (-value, -item.id)
            if len(heaps[row]) < n:
                heapq.heappush(heaps[row], entry)
            elif entry > heaps[row][0]:
                heapq.heapreplace(heaps[row], entry)
    edges = tuple({-identity: -value for value, identity in heap} for heap in heaps)
    owners, shortage = _maximum_matching(edges, guard)
    result = {'schema': 'dfmcp.furniture-allocation/1', 'request_digest': request.digest,
              'status': 'shortage' if shortage is not None else 'allocated',
              'candidate_count': len(candidates), 'maximum_assignable': len(owners),
              'compatible_counts': [{'slot': s.name, 'count': counts[i]} for i, s in enumerate(request.slots)],
              'objective': 'total_same_level_manhattan_then_lexical_slot_item_ids',
              'same_level_only': True, 'plan': None, 'plan_digest': None, 'assignments': [],
              'total_distance': None, 'shortage': None,
              'placement_eligibility_proven': False, 'pathfinding_proven': False,
              'items_reserved': False, 'game_effect_performed': False, 'production_admitted': False}
    if shortage is not None:
        result['shortage'] = {'slots': [request.slots[i].name for i in shortage['rows']],
                              'candidate_items': shortage['candidate_items'], 'missing': shortage['missing'],
                              'scope': 'complete_supplied_same_level_candidate_graph'}
    else:
        chosen = _minimum_cost(edges, guard)
        plan = FurniturePlan(tuple(Step(s.name, s.kind, identity, s.target, s.after)
                                   for s, identity in zip(request.slots, chosen)))
        result['plan'], result['plan_digest'] = plan.json(), plan.digest
        result['total_distance'] = sum(edges[i][identity] for i, identity in enumerate(chosen))
        result['assignments'] = [{'slot': s.name, **by_id[identity].json(),
                                  'distance': edges[i][identity]}
                                 for i, (s, identity) in enumerate(zip(request.slots, chosen))]
    guard()
    require(len(canonical(result)) <= 49152, 'complete allocation output exceeds reservation')
    return result
