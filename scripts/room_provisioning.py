"""Compile room-count intentions into existing excavation and furniture artifacts.

Pure, bounded geometry; no observation, item reservation, room assignment or
mutation permission. Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5 (WP-05/WP-10).
"""
from __future__ import annotations

from collections import deque
from dataclasses import dataclass
import hashlib
import json

import room_connections as connections
from room_site_mask import SiteMask
from room_orientation import compile_rotated_area
from room_access import Access

from furniture_allocation import Guard, Request, Slot, idle
from furniture_plan import MAX_BYTES, canonical, integer, label, require, unique

INTENT_SCHEMA = 'dfmcp.room-provisioning-request/1'
PLAN_SCHEMA = 'dfmcp.room-provisioning-plan/1'
POLICY = 'dfmcp.connected-room-furnishings/1'
MAX_PLAN_BYTES = 49152
MAX_AREAS = 8
MAX_PARTS = 32
MAX_TILES = 512
MAX_CAPTURE_VOLUME = 1024
Point = tuple[int, int, int]


def _object(value: object, required: set[str], optional: set[str] = frozenset()) -> dict:
    require(type(value) is dict and required <= set(value) <= required | optional,
            'unknown or missing room-provisioning fields')
    return value


def _json(raw: bytes, maximum: int, guard: Guard) -> object:
    require(type(raw) is bytes and 1 <= len(raw) <= maximum, 'room-provisioning byte bound exceeded')
    depth, quoted, escaped = 0, False, False
    for offset, byte in enumerate(raw):
        if offset % 256 == 0:
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
            require(depth <= 10, 'room-provisioning nesting bound exceeded')
        elif byte in (93, 125):
            depth -= 1
    def nonfinite(_value: str):
        raise ValueError('nonfinite room-provisioning number')
    return json.loads(raw.decode('utf-8'), object_pairs_hook=unique, parse_constant=nonfinite)


def _vector(value: object, low: int, high: int) -> Point:
    require(type(value) is list and len(value) == 3, 'three exact coordinates required')
    return tuple(integer(n, low, high) for n in value)


def _constraints(value: object, kinds: tuple[str, ...]) -> dict:
    _object(value, set(), set(kinds))
    out = {}
    for kind in kinds:
        row = _object(value.get(kind, {}), set(),
                      {'material', 'subtype', 'max_distance', 'max_vertical_distance'})
        # Reuse the actual allocation contract, including exact integer types.
        slot = Slot.from_json({'name': kind, 'kind': kind, 'target': [1, 1, 1], **row})
        out[kind] = {key: slot.json()[key] for key in ('material', 'subtype', 'max_distance')}
        # Zero is the legacy same-level contract; retain its exact bytes.
        # A nonzero limit requires the observed-route allocator, which verifies
        # actual floor/stair connectivity rather than inventing vertical access.
        if slot.max_vertical_distance:
            out[kind]['max_vertical_distance'] = slot.max_vertical_distance
    return out


@dataclass(frozen=True)
class _Rect:
    origin: Point
    size: Point
    role: str
    area: str

    @property
    def end(self) -> Point:
        return tuple(a + n for a, n in zip(self.origin, self.size))

    @property
    def volume(self) -> int:
        return self.size[0] * self.size[1] * self.size[2]

    def overlaps(self, other: _Rect) -> bool:
        return all(a < d and c < b for a, b, c, d in
                   zip(self.origin, self.end, other.origin, other.end))

    def points(self, guard: Guard):
        x, y, z = self.origin
        w, h, levels = self.size
        for pz in range(z, z + levels):
            for py in range(y, y + h):
                for px in range(x, x + w):
                    guard()
                    yield px, py, pz

    def part(self) -> dict:
        return {'region': {'origin': list(self.origin), 'size': list(self.size)}, 'shape': 'floor'}


def _neighbors(point: Point) -> tuple[Point, ...]:
    x, y, z = point
    # Fixed x/y cardinal order. No diagonal/vertical/native path assumptions.
    return (x - 1, y, z), (x, y - 1, z), (x, y + 1, z), (x + 1, y, z)


def _area(value: object, guard: Guard) -> tuple[dict, list[_Rect], list[dict], list[dict], Point]:
    _object(value, {'name', 'origin', 'template'}, {'item_constraints', 'rotation'})
    if 'rotation' in value:
        return compile_rotated_area(value, guard, _area)
    name = label(value['name'])
    require(len(name) <= 24, 'area name exceeds room-slot namespace allowance')
    x, y, z = origin = _vector(value['origin'], 1, 32766)
    template = _object(value['template'], {'kind'}, {'rooms_count', 'room_size', 'table_count', 'columns'})
    kind = template['kind']
    parts, units, selections = [], [], []

    def rectangle(px: int, py: int, width: int, height: int, role: str) -> None:
        guard()
        require(1 <= px and 1 <= py and px + width <= 32767 and py + height <= 32767,
                'room geometry or full native halo exceeds coordinate bounds')
        parts.append(_Rect((px, py, z), (width, height, 1), role, name))

    def unit(index: int, px: int, py: int, width: int, height: int,
             doorway: Point | None, items: tuple[tuple[str, Point], ...]) -> None:
        unit_name = f'{name}.{index:03d}'
        previous = ()
        for item_kind, target in items:
            slot_name = unit_name + '.' + item_kind
            selections.append({'name': slot_name, 'kind': item_kind, 'target': list(target),
                               'after': list(previous), **constraints[item_kind]})
            previous = (slot_name,)
        units.append({'name': unit_name, 'origin': [px, py, z], 'size': [width, height, 1],
                      'doorway': None if doorway is None else list(doorway),
                      'slots': [unit_name + '.' + item_kind for item_kind, _ in items]})

    if kind == 'bedroom_cluster':
        _object(template, {'kind', 'rooms_count', 'room_size'})
        count = integer(template['rooms_count'], 1, 10)
        size = template['room_size']
        require(type(size) is list and len(size) == 2, 'bedroom interior requires width and height')
        width, height = (integer(n, 3, 7) for n in size)
        constraints = _constraints(value.get('item_constraints', {}), ('bed', 'chair', 'table'))
        rows = (count + 3) // 4
        for row in range(rows):
            in_row = min(4, count - row * 4)
            py = y + row * (height + 3)
            for column in range(in_row):
                px = x + column * (width + 1)
                doorway = (px + (width - 1) // 2, py + height, z)
                rectangle(px, py, width, height, 'room')
                rectangle(doorway[0], doorway[1], 1, 1, 'doorway')
                unit(row * 4 + column + 1, px, py, width, height, doorway,
                     (('bed', (px, py, z)), ('chair', (px + width - 1, py, z)),
                      ('table', (px + width - 1, py + 1, z))))
            rectangle(x - 1, py + height + 1, in_row * (width + 1), 1, 'corridor')
        entrance = (x - 2, y + height + 1, z)
        rectangle(entrance[0], entrance[1], 1, (rows - 1) * (height + 3) + 1, 'corridor')
        normalized_template = {'kind': kind, 'rooms_count': count, 'room_size': [width, height]}
    elif kind == 'dining_hall':
        _object(template, {'kind', 'table_count'}, {'columns'})
        count = integer(template['table_count'], 1, 16)
        columns = integer(template.get('columns', 4), 1, 4)
        active_columns = min(count, columns)
        rows = (count + columns - 1) // columns
        width, height = active_columns * 3 + 1, rows * 2 + 1
        constraints = _constraints(value.get('item_constraints', {}), ('chair', 'table'))
        rectangle(x, y, width, height, 'room')
        entrance = origin
        for index in range(count):
            px, py = x + (index % columns) * 3 + 1, y + (index // columns) * 2 + 1
            unit(index + 1, px, py, 2, 1, None,
                 (('chair', (px, py, z)), ('table', (px + 1, py, z))))
        normalized_template = {'kind': kind, 'table_count': count, 'columns': columns}
    else:
        raise ValueError('unsupported room-provisioning template')
    return ({'name': name, 'origin': list(origin), 'template': normalized_template,
             'item_constraints': constraints}, parts, units, selections, entrance)


def _connect_project(compiled, floors: set[Point], targets: set[Point],
                     forbidden: list[_Rect], low: Point, extent: Point,
                     guard: Guard, site_mask: SiteMask | None = None,
                     root: Point | None = None, *,
                     minimum_fallback: bool = True) -> connections.Connections:
    """Preserve every required bedroom wall while joining complete area floors."""
    require(extent[2] == 1, 'connected room projects require one level; stairs are not inferred')
    protected = set(targets)
    high = tuple(a + n for a, n in zip(low, extent))
    for area, _, units, _, _ in compiled:
        if area['template']['kind'] != 'bedroom_cluster':
            continue
        for unit in units:
            x, y, z = unit['origin']
            width, height, _ = unit['size']
            doorway = tuple(unit['doorway'])
            for yy in range(y - 1, y + height + 1):
                for xx in range(x - 1, x + width + 1):
                    guard()
                    p = (xx, yy, z)
                    if ((xx in (x - 1, x + width) or yy in (y - 1, y + height))
                            and p != doorway and all(a <= n < b for a, n, b in zip(low, p, high))):
                        protected.add(p)
    # Only the already bounded capture is scanned. Huge excluded cuboids are
    # membership tests, never expanded volumes or permission to enlarge a route.
    for p in _Rect(low, extent, 'capture', '').points(guard):
        if site_mask is not None and not site_mask.permits_floor(p):
            protected.add(p)
        for excluded in forbidden:
            guard()
            if all(a <= n < b for a, n, b in zip(excluded.origin, p, excluded.end)):
                protected.add(p)
                break
    return connections.connect(floors - targets, protected,
        tuple((area['name'], entry) for area, _, _, _, entry in compiled),
        (low, extent), max_new_tiles=MAX_TILES - len(floors), guard=guard, root=root,
        minimum_fallback=minimum_fallback)


@dataclass(frozen=True)
class RoomPlan:
    """Immutable compiled bytes. Decode regenerates, rather than trusting outputs."""
    _body: bytes

    @classmethod
    def compile(cls, value: object, guard: Guard = idle, *,
                _connection_fallback: bool = True) -> RoomPlan:
        # Only legacy layout-search replay narrows this internal choice. Every
        # emitted plan still regenerates normally: successful greedy routes are
        # identical with or without fallback. Never use a process-global switch.
        require(type(_connection_fallback) is bool, 'invalid room connection fallback selection')
        guard()
        _object(value, {'schema', 'world_folder', 'site', 'areas'}, {'excluded_items', 'excluded_regions', 'connect_areas', 'site_mask', 'access'})
        require(value['schema'] == INTENT_SCHEMA, 'wrong room-provisioning request generation')
        connect_areas = value.get('connect_areas', False)
        require(type(connect_areas) is bool, 'connect_areas must be an exact Boolean')
        entrance_access = Access.from_json(value['access']) if 'access' in value else None
        require(entrance_access is None or connect_areas, 'access requires complete project connection')
        site_mask = SiteMask.from_json(value['site_mask'], guard) if 'site_mask' in value else None
        areas = value['areas']
        require(type(areas) is list and 1 <= len(areas) <= MAX_AREAS, 'room plan needs 1..8 areas')
        exclusions = value.get('excluded_regions', [])
        require(type(exclusions) is list and len(exclusions) <= 32, 'too many excluded regions')
        forbidden = []
        for region in exclusions:
            guard()
            _object(region, {'origin', 'size'})
            start, size = _vector(region['origin'], 0, 32767), _vector(region['size'], 1, 32768)
            require(all(a + n <= 32768 for a, n in zip(start, size)), 'excluded region exceeds tile bounds')
            forbidden.append(_Rect(start, size, 'excluded', ''))
        require(len(set(forbidden)) == len(forbidden), 'duplicate excluded region')
        forbidden.sort(key=lambda r: (r.origin, r.size))
        compiled = [_area(area, guard) for area in areas]
        require(len({a[0]['name'] for a in compiled}) == len(compiled), 'duplicate area name')
        compiled.sort(key=lambda a: a[0]['name'])
        parts = [part for _, group, _, _, _ in compiled for part in group]
        require(len(parts) <= MAX_PARTS and sum(p.volume for p in parts) <= MAX_TILES,
                'complete excavation exceeds existing 32-part/512-target bounds')
        low = tuple(min(p.origin[i] for p in parts) for i in range(3))
        high = tuple(max(p.end[i] for p in parts) for i in range(3))
        extent = tuple(b - a for a, b in zip(low, high))
        require(max(extent) <= 128 and extent[0] * extent[1] * extent[2] <= MAX_CAPTURE_VOLUME,
                'complete excavation exceeds one existing coherent capture')
        for index, part in enumerate(parts):
            for other in parts[:index] + forbidden:
                guard()
                require(not part.overlaps(other), 'overlapping excavation or excluded target region')
        # This constructor enforces all slot, exclusion, byte and DAG bounds before
        # floor expansion, and never silently drops an area to fit the allocator.
        request = Request.from_json({'schema': 'dfmcp.furniture-request/1',
            'world_folder': value['world_folder'], 'site': value['site'],
            'slots': [slot for _, _, _, group, _ in compiled for slot in group],
            'excluded_items': value.get('excluded_items', [])})
        targets = {slot.target for slot in request.slots}
        floors = set().union(*(set(part.points(guard)) for part in parts))
        require(targets <= floors, 'furnishing outside complete excavation mask')
        if site_mask is not None:
            for point in sorted(floors):
                guard()
                require(site_mask.permits_floor(point), 'room floor forbidden by site mask')
        # Enforce separating room walls across ALL areas, not merely within each
        # cluster. A corridor from another area cannot open a bedroom wall.
        for area, _, units, _, _ in compiled:
            if area['template']['kind'] != 'bedroom_cluster':
                continue
            for unit in units:
                px, py, pz = unit['origin']
                width, height, _ = unit['size']
                door = tuple(unit['doorway'])
                for yy in range(py - 1, py + height + 1):
                    for xx in range(px - 1, px + width + 1):
                        guard()
                        if xx in (px - 1, px + width) or yy in (py - 1, py + height):
                            require((xx, yy, pz) == door or (xx, yy, pz) not in floors,
                                    'another excavation opens a required bedroom wall')
                            if site_mask is not None and (xx, yy, pz) != door:
                                require(site_mask.permits_wall((xx, yy, pz)),
                                        'required bedroom wall forbidden by site mask')
        geometry = []
        for area, group, units, slots, entrance in compiled:
            floor = set().union(*(set(part.points(guard)) for part in group))
            walkable = floor - targets
            require(entrance in walkable, 'furniture obstructs the geometric entry point')
            visited, queue = {entrance}, deque([entrance])
            while queue:
                for point in _neighbors(queue.popleft()):
                    guard()
                    if point in walkable and point not in visited:
                        visited.add(point)
                        queue.append(point)
            require(visited == walkable, 'disconnected intended room circulation')
            access = []
            for slot in slots:
                guard()
                neighbors = sorted(set(_neighbors(tuple(slot['target']))) & visited)
                require(bool(neighbors), 'furniture has no cardinal geometric access')
                access.append({'slot': slot['name'], 'approach': list(neighbors[0])})
            witness = canonical({'area': area['name'], 'entry': entrance,
                'walkable': sorted(visited), 'approaches': access, 'policy': POLICY})
            geometry.append({'name': area['name'], 'entry_point': list(entrance), 'units': units,
                'floor_tiles': len(floor), 'walkable_tiles': len(walkable), 'approaches': access,
                'access_digest': hashlib.sha256(b'dfmcp-room-geometric-access/1\0' + witness).hexdigest()})
        project_connections = None
        if connect_areas:
            route_low, route_size, root = low, extent, None
            if entrance_access is not None:
                root = entrance_access.point
                require(root not in targets, 'furniture obstructs the requested access point')
                for cell in sorted(floors):
                    guard()
                    require(entrance_access.contains(cell), 'room floor outside access routing region')
                route_low, route_size = entrance_access.origin, entrance_access.size
                if root not in floors:
                    # Retain the entrance itself in the original goal. Without
                    # this seed, downstream terrain proof could omit its truth.
                    require(len(floors) < MAX_TILES, 'access point exceeds the tile limit')
                    parts.append(_Rect(root, (1, 1, 1), 'access_point', ''))
                    floors.add(root)
            routed = _connect_project(compiled, floors, targets, forbidden,
                                      route_low, route_size, guard, site_mask, root,
                                      minimum_fallback=_connection_fallback)
            corridors = [_Rect(origin, size, 'project_corridor', '') for origin, size in routed.parts]
            require(len(parts) + len(corridors) <= MAX_PARTS,
                    'connected room project exceeds the existing 32-part excavation limit')
            expanded = set().union(*(set(part.points(guard)) for part in corridors))
            require(expanded == set(routed.added) and not expanded & floors
                    and sum(p.volume for p in corridors) == len(expanded),
                    'corridor partition differs from the exact selected mask')
            parts.extend(corridors)
            floors.update(expanded)
            require(len(floors) <= MAX_TILES, 'connected room project exceeds the tile limit')
            project_connections = routed.summary()
            project_connections['root_area'] = compiled[0][0]['name'] if root is None else None
            project_connections['areas'] = [area['name'] for area, *_ in compiled]
            if entrance_access is not None:
                project_connections['access'] = {
                    **entrance_access.json(), 'all_areas_connected_to_point': True,
                    'point_is_existing_fortress_entrance_proven': False,
                    'point_floor_observed': False}
                # The existing excavation codec hashes a tight target hull,
                # not the larger domain that was permitted for corridor search.
                low = tuple(min(p.origin[i] for p in parts) for i in range(3))
                high = tuple(max(p.end[i] for p in parts) for i in range(3))
                extent = tuple(b - a for a, b in zip(low, high))
        parts.sort(key=lambda p: (p.origin[::-1], p.size[::-1]))
        blueprint = {'schema': 'dfmcp.excavation-blueprint/1', 'parts': [p.part() for p in parts]}
        require(len(canonical(blueprint)) <= MAX_BYTES, 'excavation artifact exceeds existing input bound')
        intent = {'schema': INTENT_SCHEMA, 'world_folder': request.folder, 'site': request.site,
            'areas': [area for area, *_ in compiled], 'excluded_items': list(request.excluded_items),
            'excluded_regions': [{'origin': list(r.origin), 'size': list(r.size)} for r in forbidden]}
        if connect_areas:
            intent['connect_areas'] = True
        if site_mask is not None:
            intent['site_mask'] = site_mask.json()
        if entrance_access is not None:
            intent['access'] = entrance_access.json()
        require(len(canonical(intent)) <= MAX_BYTES, 'normalized room request exceeds input bound')
        # Match the existing read-only blueprint's semantic mask identity. The
        # exact downstream normal-mining partition is still owned by dig_blueprint.
        indices = [(((z - low[2]) * extent[1] + y - low[1]) * extent[0] + x - low[0], 3)
                   for x, y, z in sorted(floors, key=lambda p: p[::-1])]
        mask = {'region': {'origin': list(low), 'size': list(extent)}, 'targets': indices}
        body = {'schema': PLAN_SCHEMA, 'policy': POLICY, 'intent': intent,
            'excavation_blueprint': blueprint, 'furniture_request': request.json(), 'areas': geometry,
            'excavation_mask_digest': hashlib.sha256(b'dfmcp-excavation-blueprint/1\0' + canonical(mask)).hexdigest(),
            'furniture_request_digest': request.digest, 'target_tiles': len(floors),
            'capture_region': mask['region'], 'furniture_count': len(request.slots),
            'terrain_observed': False, 'map_dimensions_observed': False,
            'native_pathfinding_proven': False, 'existing_fort_access_proven': False,
            'wall_preservation_observed': False, 'room_assignments_created': False,
            'prepared_plan_created': False, 'room_completion_proven': False,
            'mutation_authority_granted': False, 'construction_completion_proven': False}
        if project_connections is not None:
            body['project_connections'] = project_connections
        plan = cls(canonical(body))
        require(len(plan.encode()) <= MAX_PLAN_BYTES, 'complete room plan exceeds bounded artifact')
        guard()
        return plan

    @classmethod
    def from_request(cls, raw: bytes, guard: Guard = idle) -> RoomPlan:
        return cls.compile(_json(raw, MAX_BYTES, guard), guard)

    @classmethod
    def decode(cls, raw: bytes, guard: Guard = idle) -> RoomPlan:
        value = _json(raw, MAX_PLAN_BYTES, guard)
        require(type(value) is dict and 'intent' in value, 'room plan lacks original intent')
        plan = cls.compile(value['intent'], guard)
        require(plan.encode() == raw, 'room plan outputs do not match the retained intent')
        return plan

    @property
    def digest(self) -> str:
        return hashlib.sha256(b'dfmcp-room-provisioning-plan/1\0' + self._body).hexdigest()

    def json(self) -> dict:
        return {**json.loads(self._body), 'plan_digest': self.digest}

    def encode(self) -> bytes:
        return canonical(self.json())

    def request(self) -> Request:
        return Request.from_json(self.json()['furniture_request'])

    def summary(self) -> dict:
        """Complete reproducible room intent without duplicating allocation slots."""
        value = self.json()
        value.pop('furniture_request')
        return value
