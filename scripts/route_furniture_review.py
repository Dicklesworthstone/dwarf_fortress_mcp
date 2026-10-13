"""Retain and recheck original route-constrained furniture selections.

A review is immutable intent/evidence data, not custody, a reservation or an
execution permit. Revalidation checks ALL original selections; it never invokes
allocation, substitutes an item, changes the region or relaxes a constraint.
Beads: df-dfhack-bridge-plane-c-pic.3/.4; df-franken-storage-mvcc-54h.4.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import ipaddress
import json
import re

from furniture_allocation import Candidate, Guard, MAX_ITEMS, Request, distance, idle
from furniture_plan import FurniturePlan, Step, canonical, integer, require, unique
import route_furniture_allocation as routes

SCHEMA = 'dfmcp.route-furniture-review/1'
POLICY = 'original-items-and-constraints-fresh-routes/1'
MAX_BYTES = 49152
MAX_NODES = 16384
MAX_TICK = (2**32 - 1) * 403200 + 403199


def exact(value: object, keys: set[str]) -> dict:
    require(type(value) is dict and set(value) == keys, 'invalid closed review fields')
    return value


def digest_text(value: object) -> str:
    require(type(value) is str and re.fullmatch('[0-9a-f]{64}', value) is not None,
            'review requires an exact lowercase SHA-256 identity')
    return value


def triple(value: object, low: int, high: int) -> tuple[int, int, int]:
    require(type(value) is list and len(value) == 3, 'review requires three coordinates')
    return tuple(integer(n, low, high) for n in value)


def bounded(value: object, guard: Guard) -> None:
    """Bound arbitrary in-memory JSON before copying/hashing/normalizing it."""
    pending, nodes, size = [(value, 0)], 0, 0
    while pending:
        node, depth = pending.pop()
        guard()
        nodes += 1
        size += 8
        require(nodes <= MAX_NODES and depth <= 12, 'review shape exceeds profile')
        if type(node) is dict:
            require(nodes + len(pending) + len(node) <= MAX_NODES, 'review node allowance exceeded')
            for key, child in node.items():
                require(type(key) is str and len(key) <= 128, 'invalid review field name')
                size += len(key.encode('utf-8'))
                pending.append((child, depth + 1))
        elif type(node) is list:
            require(nodes + len(pending) + len(node) <= MAX_NODES, 'review node allowance exceeded')
            pending.extend((child, depth + 1) for child in node)
        elif type(node) is str:
            require(len(node) <= MAX_BYTES and '\0' not in node, 'invalid review text')
            size += len(node.encode('utf-8'))
        elif type(node) is int:
            require(-(2**63) <= node < 2**64, 'review integer exceeds profile')
        else:
            require(node is None or type(node) is bool, 'invalid review JSON value')
        # This accounting bounds visits and aggregate text, not serialized size.
        require(size <= 4 * MAX_BYTES, 'review aggregate size exceeded')
    require(len(canonical(value)) <= MAX_BYTES, 'review byte allowance exceeded')
    guard()


@dataclass(frozen=True)
class Source:
    endpoint: str
    df_version: str
    dfhack_version: str
    map_generation: int
    inventory_generation: int
    map_sha256: str
    inventory_sha256: str
    tick: int
    dimensions: tuple[int, int, int]
    origin: tuple[int, int, int]
    size: tuple[int, int, int]
    horizons: tuple[int, int, int]

    def __post_init__(self) -> None:
        require(type(self.endpoint) is str and 1 <= len(self.endpoint) <= 128
                and self.endpoint.count(':') == 1, 'invalid review endpoint')
        host, port = self.endpoint.split(':')
        require(port.isascii() and port.isdecimal() and len(port) <= 5, 'invalid review port')
        address = ipaddress.IPv4Address(host)
        number = integer(int(port), 1, 65535)
        require(address.is_loopback and self.endpoint == f'{address}:{number}',
                'review endpoint must be canonical numeric IPv4 loopback')
        for version in (self.df_version, self.dfhack_version):
            require(type(version) is str and '\0' not in version
                    and 1 <= len(version.encode('utf-8')) <= 128, 'invalid source software')
        for generation in (self.map_generation, self.inventory_generation):
            integer(generation, 1, 2**64 - 2)
        for digest in (self.map_sha256, self.inventory_sha256):
            digest_text(digest)
        integer(self.tick, 0, MAX_TICK)
        for value, low, high in ((self.dimensions, 1, 32768), (self.origin, 0, 32767),
                                 (self.size, 1, 128), (self.horizons, 0, 2**31 - 1)):
            require(type(value) is tuple and len(value) == 3, 'invalid source dimensions/horizons')
            for n in value:
                integer(n, low, high)
        require(self.size[0] * self.size[1] * self.size[2] <= routes.MAX_TILES
                and all(a + b <= c for a, b, c in zip(self.origin, self.size, self.dimensions)),
                'source region exceeds map bounds')

    def json(self) -> dict:
        return {**{name: getattr(self, name) for name in (
            'endpoint', 'df_version', 'dfhack_version', 'map_generation', 'inventory_generation',
            'map_sha256', 'inventory_sha256', 'tick')},
            **{name: list(getattr(self, name)) for name in ('dimensions', 'origin', 'size', 'horizons')}}

    @classmethod
    def from_json(cls, value: object) -> Source:
        value = exact(value, set(cls.__dataclass_fields__))
        numbers = {name: triple(value[name], low, high) for name, low, high in (
            ('dimensions', 1, 32768), ('origin', 0, 32767), ('size', 1, 128), ('horizons', 0, 2**31 - 1))}
        return cls(**{**value, **numbers})

    def permits(self, current: Source) -> None:
        self.__post_init__()
        require(type(current) is Source, 'invalid current review source')
        current.__post_init__()
        for field in ('endpoint', 'df_version', 'dfhack_version', 'map_generation',
                      'inventory_generation', 'dimensions', 'origin', 'size'):
            require(getattr(self, field) == getattr(current, field), 'original review source changed')
        require(current.tick >= self.tick and all(a >= b for a, b in zip(current.horizons, self.horizons)),
                'current source predates the original review')


def candidate(value: object) -> Candidate:
    value = exact(value, {'item', 'kind', 'position', 'material', 'subtype'})
    require(type(value['material']) is list and len(value['material']) == 2, 'invalid review material')
    return Candidate(value['item'], value['kind'], triple(value['position'], 0, 32767),
                     tuple(value['material']), value['subtype'])


def object_digest(raw: bytes) -> str:
    return hashlib.sha256(b'dfmcp-route-furniture-review/1\0' + raw).hexdigest()


def _parse(raw: bytes, expected: str, guard: Guard) -> tuple[dict, Request, Source, FurniturePlan]:
    guard()
    require(type(raw) is bytes and 1 <= len(raw) <= MAX_BYTES, 'invalid review file size')
    digest_text(expected)
    require(object_digest(raw) == expected, 'review differs from independently selected identity')
    # Bound nesting BEFORE entering the general JSON parser, including hostile
    # quoted braces. Byte extent is already bounded before hashing or scanning.
    depth, quoted, escaped = 0, False, False
    for offset, byte in enumerate(raw):
        if offset % 128 == 0:
            guard()
        if quoted:
            if escaped:
                escaped = False
            elif byte == 92:
                escaped = True
            elif byte == 34:
                quoted = False
        elif byte == 34:
            quoted = True
        elif byte in (91, 123):
            depth += 1
            require(depth <= 12, 'review nesting exceeds profile')
        elif byte in (93, 125):
            depth -= 1
            require(depth >= 0, 'unbalanced review JSON')
    require(depth == 0 and not quoted, 'incomplete review JSON')
    value = json.loads(raw.decode('utf-8'), object_pairs_hook=unique)
    bounded(value, guard)
    exact(value, {'schema', 'policy', 'route_policy', 'path_policy', 'request', 'source',
                  'max_route_steps', 'selections', 'plan_digest', 'allocation_digest'})
    require((value['schema'], value['policy'], value['route_policy'], value['path_policy']) ==
            (SCHEMA, POLICY, routes.ROUTE_POLICY, routes.PATH_POLICY), 'unsupported review policy')
    request = Request.from_json(value['request'])
    require(value['request'] == request.json(), 'original request must be normalized, not repaired')
    source = Source.from_json(value['source'])
    limit = integer(value['max_route_steps'], 0, routes.MAX_ROUTE_STEPS)
    digest_text(value['plan_digest'])
    digest_text(value['allocation_digest'])
    rows = value['selections']
    require(type(rows) is list and len(rows) == len(request.slots), 'review omitted original selections')
    steps, types, kinds = [], {}, {}
    for slot, row in zip(request.slots, rows):
        guard()
        exact(row, {'slot', 'candidate', 'native_type', 'route_steps', 'route_sha256'})
        item = candidate(row['candidate'])
        native = integer(row['native_type'], 0, 2**31 - 1)
        integer(row['route_steps'], 0, limit)
        digest_text(row['route_sha256'])
        require(row['slot'] == slot.name and item.id not in request.excluded_items
                and item.id < source.horizons[2] and distance(slot, item) is not None,
                'original selection violates retained request')
        require(all(p < d for p, d in zip(item.position, source.dimensions))
                and all(a <= p < a + n for a, p, n in zip(source.origin, item.position, source.size))
                and all(a <= p < a + n for a, p, n in zip(source.origin, slot.target, source.size)),
                'original selected endpoint outside the retained region')
        require(types.get(native, item.kind) == item.kind and kinds.get(item.kind, native) == native,
                'inconsistent original item type identity')
        types[native], kinds[item.kind] = item.kind, native
        steps.append(Step(slot.name, slot.kind, item.id, slot.target, slot.after))
    plan = FurniturePlan(tuple(steps))
    plan.check_dimensions(source.dimensions)
    require(plan.digest == value['plan_digest'] and canonical(value) == raw,
            'review changed original plan or canonical representation')
    guard()
    return value, request, source, plan


@dataclass(frozen=True)
class Review:
    """Only immutable bytes are retained; accessors never expose shared dicts."""
    raw: bytes
    digest: str

    @classmethod
    def decode(cls, raw: bytes, expected_digest: str, guard: Guard = idle) -> Review:
        _parse(raw, expected_digest, guard)
        return cls(raw, expected_digest)

    def unpack(self, guard: Guard = idle) -> tuple[dict, Request, Source, FurniturePlan]:
        return _parse(self.raw, self.digest, guard)

    def json(self, guard: Guard = idle) -> dict:
        return self.unpack(guard)[0]


def create(request: Request, allocation: dict, source: Source, native_types: dict[int, int],
           guard: Guard = idle) -> Review:
    """Seal an established allocation, without asserting acquisition or authority.

    The acquisition shell supplies native enum identities from the SAME complete
    inventory used by the allocator. No shortage or selected subset is accepted.
    """
    guard()
    require(type(request) is Request and type(source) is Source, 'invalid review inputs')
    request.__post_init__()
    source.__post_init__()
    bounded(allocation, guard)
    require(type(native_types) is dict and len(native_types) == len(request.slots),
            'native type selection must cover precisely the original items')
    for identity, native in native_types.items():
        guard()
        integer(identity, 0, 2**31 - 2)
        integer(native, 0, 2**31 - 1)
    require(allocation.get('schema') == 'dfmcp.route-furniture-allocation/1'
            and allocation.get('status') == 'allocated' and allocation.get('policy') == routes.POLICY
            and allocation.get('route_policy') == routes.ROUTE_POLICY
            and allocation.get('path_policy') == routes.PATH_POLICY
            and allocation.get('request_digest') == request.digest,
            'only a complete original route allocation can be retained')
    unsigned = {key: value for key, value in allocation.items() if key != 'decision_digest'}
    require(allocation.get('decision_digest') == hashlib.sha256(
        b'dfmcp-route-furniture-decision/1\0' + canonical(unsigned)).hexdigest(),
        'original allocation decision digest differs')
    terrain = allocation['terrain']
    require((terrain['sha256'], terrain['game_tick'], terrain['dimensions'], terrain['region'],
             terrain['world_folder'], terrain['site'], terrain['paused']) ==
            (source.map_sha256, source.tick, list(source.dimensions),
             {'origin': list(source.origin), 'size': list(source.size)}, request.folder, request.site, True),
            'original allocation and source do not describe the same paused map')
    rows = allocation['assignments']
    require(type(rows) is list and len(rows) == len(request.slots), 'original allocation is incomplete')
    selections = []
    for row in rows:
        guard()
        item = candidate({key: row[key] for key in ('item', 'kind', 'position', 'material', 'subtype')})
        require(item.id in native_types, 'original native type absent')
        selections.append({'slot': row['slot'], 'candidate': item.json(), 'native_type': native_types[item.id],
                           'route_steps': row['route_steps'], 'route_sha256': row['route_sha256']})
    value = {'schema': SCHEMA, 'policy': POLICY, 'route_policy': routes.ROUTE_POLICY,
             'path_policy': routes.PATH_POLICY, 'request': request.json(), 'source': source.json(),
             'max_route_steps': allocation['max_route_steps'], 'selections': selections,
             'plan_digest': allocation['plan_digest'], 'allocation_digest': allocation['decision_digest']}
    bounded(value, guard)
    raw = canonical(value)
    review = Review.decode(raw, object_digest(raw), guard)
    require(review.unpack(guard)[3].json() == allocation['plan'], 'original plan differs from selected items')
    guard()
    return review


def revalidate(review: Review, candidates: tuple[Candidate, ...], native_types: dict[int, int],
               terrain_raw: bytes, source: Source, guard: Guard = idle,
               *, maximum_work: int = routes.MAX_WORK) -> dict:
    """Recheck every original item, without optimizing or choosing replacements.

    Candidate facts must be supplied by the owner's full-roster projection. A
    missing projected candidate is unestablished, not a claim that the item was
    deleted. Region/source changes refuse the request instead of rebinding it.
    """
    work = routes.Work(maximum_work, guard)
    require(type(review) is Review and type(source) is Source, 'invalid original review/source')
    original, request, baseline, plan = review.unpack(work)
    baseline.permits(source)
    terrain = routes.decode_terrain(terrain_raw, work)
    require((terrain.digest, terrain.tick, terrain.dimensions, terrain.origin, terrain.size,
             terrain.folder, terrain.site, terrain.paused) ==
            (source.map_sha256, source.tick, source.dimensions, source.origin, source.size,
             request.folder, request.site, True), 'current map and independently supplied source disagree')
    require(type(candidates) is tuple and len(candidates) <= MAX_ITEMS
            and type(native_types) is dict and len(native_types) == len(candidates),
            'current inventory projection exceeds bounds or omitted native types')
    for identity in native_types:
        work()
        integer(identity, 0, 2**31 - 2)
    by_id, types, kinds = {}, {}, {}
    for item in candidates:
        work()
        require(type(item) is Candidate, 'invalid projected candidate')
        item.__post_init__()
        require(item.id not in by_id and item.id in native_types and item.id < source.horizons[2],
                'duplicate, missing-type or out-of-horizon candidate')
        native = integer(native_types[item.id], 0, 2**31 - 1)
        require(types.get(native, item.kind) == item.kind and kinds.get(item.kind, native) == native,
                'inconsistent projected enum identity')
        types[native], kinds[item.kind], by_id[item.id] = item.kind, native, item
    graph = terrain.adjacency(work)
    rows = []
    for slot, saved in zip(request.slots, original['selections']):
        work()
        before = candidate(saved['candidate'])
        item = by_id.get(before.id)
        row = {'slot': slot.name, 'item': before.id, 'target': list(slot.target),
               'status': 'unestablished', 'reason': 'selected_item_missing_or_ineligible',
               'current_position': None if item is None else list(item.position),
               'route_steps': None, 'route_sha256': None,
               'original_route_steps': saved['route_steps'], 'original_route_sha256': saved['route_sha256']}
        if item is not None:
            if ((item.kind, item.material, item.subtype, native_types[item.id]) !=
                    (before.kind, before.material, before.subtype, saved['native_type'])):
                row.update(status='constraints_not_met', reason='selected_item_identity_changed')
            elif distance(slot, item) is None:
                row.update(status='constraints_not_met', reason='original_distance_or_level_constraint')
            elif terrain.reason(slot.target) != 'candidate':
                row['reason'] = 'target_' + terrain.reason(slot.target)
            elif terrain.reason(item.position) != 'candidate':
                row['reason'] = 'item_' + terrain.reason(item.position)
            else:
                target, start = terrain.index(slot.target), terrain.index(item.position)
                steps, parent = routes.distances(graph, target, work)
                length = steps[start]
                if length < 0:
                    row.update(status='constraints_not_met', reason='no_route_in_observed_model')
                elif length > original['max_route_steps']:
                    row.update(status='constraints_not_met', reason='original_route_limit_exceeded', route_steps=length)
                else:
                    row.update(status='constraints_met', reason='original_selection_revalidated', route_steps=length,
                               route_sha256=routes.path_digest(terrain, parent, start, target, length, work))
        rows.append(row)
    counts = {status: sum(row['status'] == status for row in rows)
              for status in ('constraints_met', 'constraints_not_met', 'unestablished')}
    status = ('constraints_met' if counts['constraints_met'] == len(rows) else
              'constraints_not_met' if counts['constraints_not_met'] else 'unestablished')
    result = {'schema': 'dfmcp.route-furniture-review-result/1', 'policy': POLICY,
              'status': status, 'review_digest': review.digest, 'original_plan_digest': plan.digest,
              'original_plan': plan.json(), 'original_request_digest': request.digest,
              'original_allocation_digest': original['allocation_digest'],
              'source': source.json(), 'max_route_steps': original['max_route_steps'],
              'counts': counts, 'rows': rows, 'all_original_selections_checked': True,
              'plan': plan.json() if status == 'constraints_met' else None,
              'reallocation_performed': False, 'constraints_relaxed': False,
              'game_effect_performed': False, 'placement_authorized': False,
              'placement_eligibility_proven': False, 'native_pathfinding_proven': False,
              'future_routes_preserved': False, 'active_effects_queried': False,
              'items_reserved': False, 'global_absence_proven': False,
              'production_admitted': False, 'work_units': work.used, 'maximum_work': maximum_work}
    work.check()
    result['decision_digest'] = hashlib.sha256(b'dfmcp-route-furniture-revalidation/1\0' + canonical(result)).hexdigest()
    require(len(canonical(result)) <= MAX_BYTES, 'complete original review result exceeds bound')
    work.check()
    return result
