"""Choose room origins and allowed orientations, then lower through RoomPlan.

Offline intended geometry only. No terrain observation, inventory selection,
mutation, hidden relaxation, random search, or globally optimal layout claim.
Beads: dfmcp-wp-pln01-spatial-blueprint-layout-pwd; df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

from dataclasses import dataclass, replace
import hashlib
import json
from typing import Callable

from furniture_plan import canonical, integer, require
from room_site_mask import SiteMask
from room_access import Access
from room_connections import WorkLimit as ConnectionWorkLimit
from room_orientation import ROTATIONS, rotation
from room_provisioning import (
    INTENT_SCHEMA, MAX_AREAS, MAX_CAPTURE_VOLUME, MAX_PARTS, MAX_PLAN_BYTES,
    MAX_TILES, RoomPlan, _json, _object, _vector,
)

REQUEST_SCHEMA = 'dfmcp.room-layout-request/1'
RESULT_SCHEMA = 'dfmcp.room-layout-result/1'
POLICY = 'dfmcp.mrv-connected-room-layout/1'
ORIENTED_POLICY = 'dfmcp.mrv-oriented-room-layout/1'
ACCESS_POLICY = 'dfmcp.mrv-entrance-room-layout/1'
FALLBACK_POLICY = 'dfmcp.mrv-minimum-fallback-room-layout/1'
FALLBACK_STRATEGY = 'greedy-then-minimum-tiles/1'
MAX_INPUT = 16 * 1024
MAX_OUTPUT = 96 * 1024
MAX_NODES = 100_000
MAX_WORK = 20_000_000
DEFAULT_NODES = 10_000
DEFAULT_WORK = 2_000_000
MAX_CANDIDATES = MAX_AREAS * MAX_CAPTURE_VOLUME * len(ROTATIONS)
Guard = Callable[[], None]
Point = tuple[int, int, int]


def _idle() -> None:
    """Pure callers retain the fixed work and structural limits."""


class WorkLimit(ValueError):
    """No result was established within the aggregate computation allowance."""


class _NodeLimit(Exception):
    pass


class _Work:
    def __init__(self, maximum: int, guard: Guard):
        self.maximum = integer(maximum, 1, MAX_WORK)
        self.guard, self.used, self.interrupted = guard, 0, False

    def __call__(self) -> None:
        if self.used >= self.maximum:
            self.interrupted = True
            raise WorkLimit('room layout aggregate work allowance exhausted')
        self.used += 1
        try:
            self.guard()
        except BaseException:
            # A caller's ValueError/InterruptedError must NEVER be mistaken for
            # an infeasible candidate and swallowed by search backtracking.
            self.interrupted = True
            raise


def _region(value: object, *, domain: bool = False) -> tuple[Point, Point]:
    _object(value, {'origin', 'size'})
    low = _vector(value['origin'], 1 if domain else 0, 32766 if domain else 32767)
    size = _vector(value['size'], 1, 128 if domain else 32768)
    require(all(a + n <= (32767 if domain else 32768) for a, n in zip(low, size)),
            'room layout region exceeds coordinate bounds')
    if domain:
        require(size[2] == 1 and size[0] * size[1] <= MAX_CAPTURE_VOLUME,
                'layout requires one level and at most 1024 domain cells')
    return low, size


def _region_json(value: tuple[Point, Point]) -> dict:
    return {'origin': list(value[0]), 'size': list(value[1])}


def _contains(region: tuple[Point, Point], point: Point) -> bool:
    return all(a <= p < a + n for a, n, p in zip(*region, point))


@dataclass(frozen=True)
class _Profile:
    area: bytes
    floors: tuple[Point, ...]
    walls: tuple[Point, ...]
    entry: Point
    low: Point
    high: Point
    parts: int
    slots: int
    variants: tuple[_Profile, ...] = ()


def _profile(area: dict, folder: str, site: int, level: int, work: _Work) -> _Profile:
    """Derive shape/constraints from the actual compiler, never a second template."""
    anchor = (128, 128, level)
    value = {key: area[key] for key in ('name', 'template', 'item_constraints', 'rotation') if key in area}
    plan = RoomPlan.compile({'schema': INTENT_SCHEMA, 'world_folder': folder, 'site': site,
                             'areas': [{**value, 'origin': list(anchor)}]}, work)
    body = plan.json()
    normalized = body['intent']['areas'][0]
    normalized.pop('origin')
    floors, walls = set(), set()

    def relative(point: Point) -> Point:
        return tuple(p - a for p, a in zip(point, anchor))

    for part in body['excavation_blueprint']['parts']:
        x, y, z = part['region']['origin']
        width, height, _ = part['region']['size']
        for yy in range(y, y + height):
            for xx in range(x, x + width):
                work()
                floors.add(relative((xx, yy, z)))
    if normalized['template']['kind'] == 'bedroom_cluster':
        for unit in body['areas'][0]['units']:
            x, y, z = unit['origin']
            width, height, _ = unit['size']
            door = tuple(unit['doorway'])
            for yy in range(y - 1, y + height + 1):
                for xx in range(x - 1, x + width + 1):
                    work()
                    if (xx in (x - 1, x + width) or yy in (y - 1, y + height)) and (xx, yy, z) != door:
                        walls.add(relative((xx, yy, z)))
    require(not floors & walls, 'compiled room floor conflicts with required walls')
    cells = floors | walls
    for key in ('origin', 'origin_region'):
        if key in area:
            normalized[key] = area[key]
    return _Profile(canonical(normalized), tuple(sorted(floors)), tuple(sorted(walls)),
                    relative(tuple(body['areas'][0]['entry_point'])),
                    tuple(min(p[i] for p in cells) for i in range(3)),
                    tuple(max(p[i] for p in cells) for i in range(3)),
                    len(body['excavation_blueprint']['parts']), body['furniture_count'])


def _layout_profile(area: dict, folder: str, site: int, level: int, work: _Work) -> _Profile:
    """One variable per area, with at most four actual compiler-derived shapes."""
    require(not ('rotation' in area and 'rotations' in area),
            'choose a fixed rotation or allowed rotations, not both')
    if 'rotations' not in area:
        if 'rotation' not in area:
            return _profile(area, folder, site, level, work)
        angle = rotation(area['rotation'])
        normalized = {key: value for key, value in area.items() if key != 'rotation'}
        if angle:
            normalized['rotation'] = angle
        return _profile(normalized, folder, site, level, work)
    choices = area['rotations']
    require(type(choices) is list and 1 <= len(choices) <= len(ROTATIONS),
            'layout requires 1..4 allowed quarter-turn rotations')
    angles = tuple(sorted(rotation(value) for value in choices))
    require(len(set(angles)) == len(angles), 'duplicate allowed room rotation')
    base = {key: value for key, value in area.items() if key != 'rotations'}
    variants = tuple(_profile({**base, **({'rotation': angle} if angle else {})},
                              folder, site, level, work) for angle in angles)
    first = variants[0]
    if len(variants) == 1:
        return first
    require(all((len(v.floors), v.parts, v.slots) == (len(first.floors), first.parts, first.slots)
                for v in variants), 'rotation changed complete template cardinality')
    normalized = json.loads(first.area)
    normalized.pop('rotation', None)
    normalized['rotations'] = list(angles)
    return replace(first, area=canonical(normalized), variants=variants)


def _normalize(value: object, work: _Work) -> tuple[dict, tuple[_Profile, ...]]:
    work()
    _object(value, {'schema', 'world_folder', 'site', 'region', 'areas'},
            {'focus', 'excluded_items', 'excluded_regions', 'search', 'site_mask', 'access_point',
             'connection_strategy'})
    require(value['schema'] == REQUEST_SCHEMA, 'wrong room layout request generation')
    if 'connection_strategy' in value:
        require(type(value['connection_strategy']) is str
                and value['connection_strategy'] == FALLBACK_STRATEGY,
                'unsupported room connection strategy')
    domain = _region(value['region'], domain=True)
    focus = _vector(value.get('focus', list(domain[0])), 1, 32766)
    require(_contains(domain, focus), 'layout focus must be inside the same-level domain')
    areas = value['areas']
    require(type(areas) is list and 1 <= len(areas) <= MAX_AREAS, 'layout needs 1..8 complete areas')
    profiles = []
    for area in areas:
        work()
        _object(area, {'name', 'template'}, {'item_constraints', 'origin', 'origin_region', 'rotation', 'rotations'})
        require(not ('origin' in area and 'origin_region' in area),
                'choose a fixed origin or an origin region, not both')
        if 'origin' in area:
            require(_contains(domain, _vector(area['origin'], 1, 32766)),
                    'fixed origin is outside the layout domain')
        if 'origin_region' in area:
            low, size = _region(area['origin_region'], domain=True)
            require(_contains(domain, low) and _contains(domain, tuple(a + n - 1 for a, n in zip(low, size))),
                    'area origin domain must fit the complete layout domain')
        profiles.append(_layout_profile(area, value['world_folder'], value['site'], domain[0][2], work))
    profiles.sort(key=lambda p: json.loads(p.area)['name'])
    normalized_areas = [json.loads(p.area) for p in profiles]
    require(len({a['name'] for a in normalized_areas}) == len(profiles), 'duplicate layout area name')
    require(sum(len(p.floors) for p in profiles) <= MAX_TILES
            and sum(p.parts for p in profiles) <= MAX_PARTS
            and sum(p.slots for p in profiles) <= 32,
            'complete layout exceeds existing floor, part or furnishing limits')
    exclusions = value.get('excluded_regions', [])
    require(type(exclusions) is list and len(exclusions) <= 32, 'too many layout exclusions')
    regions = []
    for row in exclusions:
        work()
        regions.append(_region(row))
    require(len(set(regions)) == len(regions), 'duplicate layout exclusion')
    items = value.get('excluded_items', [])
    require(type(items) is list and len(items) <= 4096, 'too many excluded inventory identities')
    for identity in items:
        work()
        integer(identity, 0, 2147483646)
    require(len(set(items)) == len(items), 'duplicate excluded item')
    search = value.get('search', {})
    _object(search, set(), {'max_nodes', 'max_work'})
    search = {'max_nodes': integer(search.get('max_nodes', DEFAULT_NODES), 1, MAX_NODES),
              'max_work': integer(search.get('max_work', DEFAULT_WORK), 1, MAX_WORK)}
    request = {'schema': REQUEST_SCHEMA, 'world_folder': value['world_folder'], 'site': value['site'],
               'region': _region_json(domain), 'focus': list(focus), 'areas': normalized_areas,
               'excluded_items': sorted(items), 'excluded_regions': [_region_json(r) for r in sorted(regions)],
               'search': search}
    if 'site_mask' in value:
        site_mask = SiteMask.from_json(value['site_mask'], work)
        require((site_mask.origin, site_mask.size) == domain, 'site mask must match the complete layout domain')
        request['site_mask'] = site_mask.json()
    if 'access_point' in value:
        work()
        access = Access.from_json({'point': value['access_point'], 'region': request['region']})
        require(not any(_contains(region, access.point) for region in regions),
                'access point intersects an excluded region')
        if 'site_mask' in request:
            require(site_mask.permits_floor(access.point), 'access point forbidden by site mask')
        request['access_point'] = list(access.point)
    if 'connection_strategy' in value:
        request['connection_strategy'] = FALLBACK_STRATEGY
    require(len(canonical(request)) <= MAX_INPUT, 'normalized layout request exceeds 16 KiB')
    work()
    return request, tuple(profiles)


@dataclass(frozen=True)
class _Candidate:
    origin: Point
    floors: int
    walls: int
    rotation: int = 0


def _candidates(profile: _Profile, request: dict, forbidden: int, work: _Work,
                wall_forbidden: int = 0) -> tuple[_Candidate, ...]:
    """At most 1024 bitset candidates per allowed orientation; no plan copies."""
    if profile.variants:
        # Alternatives share one MRV variable and one caller-owned work/node
        # allowance. Never restart the solver or drop areas for another angle.
        candidates = []
        entries = {}
        for variant in profile.variants:
            work()
            entries[json.loads(variant.area).get('rotation', 0)] = variant.entry
            candidates.extend(_candidates(variant, request, forbidden, work, wall_forbidden))
        require(len(candidates) <= len(ROTATIONS) * MAX_CAPTURE_VOLUME,
                'oriented room candidate inventory exceeds bound')
        def score(candidate: _Candidate):
            work()
            entry = entries[candidate.rotation]
            distance = sum(abs(candidate.origin[i] + entry[i] - request['focus'][i]) for i in range(3))
            return distance, candidate.origin[1], candidate.origin[0], candidate.origin[2], candidate.rotation
        candidates.sort(key=score)
        return tuple(candidates)
    low, size = _region(request['region'], domain=True)
    area = json.loads(profile.area)
    width = size[0]
    floor_bits, wall_bits = 0, 0
    # Offsets are relative to the profile's complete floor+wall bounding box.
    # A too-wide profile must not wrap across rows when represented as bits.
    if any(profile.high[i] - profile.low[i] + 1 > size[i] for i in range(3)):
        return ()
    for cells, kind in ((profile.floors, 'floor'), (profile.walls, 'wall')):
        bits = 0
        for x, y, _ in cells:
            work()
            bits |= 1 << ((y - profile.low[1]) * width + x - profile.low[0])
        if kind == 'floor':
            floor_bits = bits
        else:
            wall_bits = bits
    constraint = _region(area['origin_region'], domain=True) if 'origin_region' in area else (low, size)
    fixed = tuple(area['origin']) if 'origin' in area else None
    candidates = []
    for y in range(low[1] - profile.low[1], low[1] + size[1] - profile.high[1]):
        for x in range(low[0] - profile.low[0], low[0] + size[0] - profile.high[0]):
            work()
            origin = (x, y, low[2])
            if not _contains(constraint, origin) or (fixed is not None and origin != fixed):
                continue
            shift = (y + profile.low[1] - low[1]) * width + x + profile.low[0] - low[0]
            floors = floor_bits << shift
            if not (floors & forbidden or (wall_bits << shift) & wall_forbidden):
                candidates.append(_Candidate(origin, floors, wall_bits << shift, area.get('rotation', 0)))
    focus = request['focus']
    candidates.sort(key=lambda c: (sum(abs(c.origin[i] + profile.entry[i] - focus[i]) for i in range(3)),
                                    c.origin[1], c.origin[0], c.origin[2]))
    work()
    return tuple(candidates)


def _digest(domain: bytes, value: object) -> str:
    return hashlib.sha256(domain + canonical(value)).hexdigest()


@dataclass(frozen=True)
class Layout:
    """Immutable advisory result. Import repeats search and compares every byte."""
    _body: bytes

    @property
    def digest(self) -> str:
        return hashlib.sha256(b'dfmcp-room-layout-result/1\0' + self._body).hexdigest()

    def json(self) -> dict:
        return {**json.loads(self._body), 'layout_digest': self.digest}

    def encode(self) -> bytes:
        return canonical(self.json())

    def room_plan(self, guard: Guard = _idle) -> RoomPlan:
        value = self.json()
        require(value['status'] == 'layout_found', 'no complete room layout is available')
        return RoomPlan.decode(canonical(value['room_plan']), guard)

    @classmethod
    def decode(cls, raw: bytes, guard: Guard = _idle) -> Layout:
        value = _json(raw, MAX_OUTPUT, guard)
        require(type(value) is dict and 'request' in value, 'layout result lacks its original request')
        result = solve(canonical(value['request']), guard)
        require(result.encode() == raw, 'layout result differs from regenerated original search')
        return result


def solve(raw: bytes, guard: Guard = _idle) -> Layout:
    """First compiler-valid full layout under deterministic MRV backtracking.

    No-layout is relative to every allowed orientation/translation in this domain
    AND the existing connected RoomPlan compiler. Search-node exhaustion is
    inconclusive; work/cancellation refusal raises without returning any plan.
    """
    value = _json(raw, MAX_INPUT, guard)
    require(type(value) is dict, 'layout request must be an object')
    search = value.get('search', {})
    _object(search, set(), {'max_nodes', 'max_work'})
    work = _Work(search.get('max_work', DEFAULT_WORK), guard)
    request, profiles = _normalize(value, work)
    low, size = _region(request['region'], domain=True)
    regions = [_region(row) for row in request['excluded_regions']]
    forbidden = 0
    for y in range(size[1]):
        for x in range(size[0]):
            work()
            for region in regions:
                work()
                if _contains(region, (low[0] + x, low[1] + y, low[2])):
                    forbidden |= 1 << (y * size[0] + x)
                    break
    wall_forbidden = 0
    if 'site_mask' in request:
        for index, cell in enumerate(request['site_mask']['cells']):
            work()
            if cell not in '.F':
                forbidden |= 1 << index
            if cell not in '.W':
                wall_forbidden |= 1 << index
    if 'access_point' in request:
        work()
        x, y, _ = request['access_point']
        # A retained bedroom wall cannot consume the selected entrance. Furniture
        # collision and whole-route feasibility are checked by the real compiler.
        wall_forbidden |= 1 << ((y - low[1]) * size[0] + x - low[0])
    domains = tuple(_candidates(p, request, forbidden, work, wall_forbidden) for p in profiles)
    require(sum(map(len, domains)) <= MAX_CANDIDATES, 'layout candidate inventory exceeds bound')
    nodes, leaves, rejected, backtracks = 0, 0, 0, 0
    trace = hashlib.sha256(b'dfmcp-room-layout-search/1\0')
    trace.update(canonical(request))
    chosen: dict[int, _Candidate] = {}
    solution = None

    def search_tree(floors: int, walls: int) -> RoomPlan | None:
        nonlocal nodes, leaves, rejected, backtracks
        work()
        if nodes >= request['search']['max_nodes']:
            raise _NodeLimit()
        nodes += 1
        if len(chosen) == len(profiles):
            leaves += 1
            areas = []
            for index, profile in enumerate(profiles):
                work()
                area = json.loads(profile.area)
                area.pop('origin_region', None)
                area.pop('rotations', None)
                area.pop('rotation', None)
                if chosen[index].rotation:
                    area['rotation'] = chosen[index].rotation
                area['origin'] = list(chosen[index].origin)
                areas.append(area)
            lowered = {'schema': INTENT_SCHEMA, 'world_folder': request['world_folder'],
                       'site': request['site'], 'areas': areas, 'connect_areas': True,
                       'excluded_items': request['excluded_items'], 'excluded_regions': request['excluded_regions']}
            if 'site_mask' in request:
                lowered['site_mask'] = request['site_mask']
            if 'access_point' in request:
                lowered['access'] = {'point': request['access_point'], 'region': request['region']}
            try:
                # New corridor feasibility changes earlier search branches and
                # work counts, even if a later legacy plan already succeeded.
                # Seal the choice in the request; old artifacts must replay the
                # original greedy search, not inherit a new search policy.
                plan = RoomPlan.compile(lowered, work,
                    _connection_fallback='connection_strategy' in request)
            except ConnectionWorkLimit as cause:
                raise WorkLimit('room connection work allowance exhausted') from cause
            except ValueError:
                if work.interrupted:
                    raise
                rejected += 1
                trace.update(b'compiler_refused\0')
                return None
            trace.update(bytes.fromhex(plan.digest))
            return plan
        available = []
        for index, candidates in enumerate(domains):
            if index in chosen:
                continue
            fits = []
            for candidate in candidates:
                work()
                if not (candidate.floors & (floors | walls) or candidate.walls & floors):
                    fits.append(candidate)
            if not fits:
                trace.update(canonical(['empty_domain', index]))
                return None
            available.append((len(fits), index, fits))
        _, index, fits = min(available, key=lambda row: (row[0], row[1]))
        trace.update(canonical(['select', index, len(fits)]))
        for candidate in fits:
            work()
            event = ['try', index, candidate.origin]
            if candidate.rotation:
                event.append(candidate.rotation)
            trace.update(canonical(event))
            chosen[index] = candidate
            plan = search_tree(floors | candidate.floors, walls | candidate.walls)
            if plan is not None:
                return plan
            del chosen[index]
            backtracks += 1
        return None

    try:
        solution = search_tree(0, 0)
        status = 'layout_found' if solution is not None else 'no_layout_in_domain'
    except _NodeLimit:
        status = 'search_limit_reached'
        trace.update(b'node_limit\0')
    work()
    oriented = any('rotation' in area or 'rotations' in area for area in request['areas'])
    body = {'schema': RESULT_SCHEMA, 'policy': FALLBACK_POLICY if 'connection_strategy' in request
            else ACCESS_POLICY if 'access_point' in request else ORIENTED_POLICY if oriented else POLICY,
            'status': status, 'request': request,
            'request_digest': _digest(b'dfmcp-room-layout-request/1\0', request),
            'room_plan': None if solution is None else solution.json(),
            'search': {'nodes_visited': nodes, 'complete_layouts_tested': leaves,
                       'compiler_refusals': rejected, 'backtracks': backtracks,
                       'candidate_counts': [{'area': a['name'], 'count': len(d)} for a, d in zip(request['areas'], domains)],
                       'node_limit': request['search']['max_nodes'], 'work_units': work.used,
                       'work_limit': work.maximum, 'trace_digest': trace.hexdigest(),
                       'domain_exhausted': status == 'no_layout_in_domain',
                       'all_layouts_ranked': False},
            'complete_original_request_retained': True,
            'all_areas_placed': solution is not None, 'partial_plan_returned': False,
            'global_optimum_proven': False, 'terrain_observed': False,
            'native_pathfinding_proven': False, 'existing_fort_access_proven': False,
            'items_reserved': False, 'prepared_mutation_plan_created': False,
            'game_effect_performed': False, 'production_admitted': False}
    result = Layout(canonical(body))
    require(len(result.encode()) <= MAX_OUTPUT, 'complete layout result exceeds output bound')
    guard()
    return result
