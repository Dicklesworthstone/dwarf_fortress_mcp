"""Keep original room geometry and route-constrained furnishings inseparable.

This is bounded intent/evidence, not private custody, an observation issuer or
permission to place furniture. Existing room and route codecs remain unchanged.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5; dfmcp-wp-pln01-spatial-blueprint-layout-pwd.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import struct

from furniture_allocation import Candidate, Guard, idle
from furniture_plan import canonical, integer, require, unique
from room_provisioning import MAX_PLAN_BYTES, RoomPlan
import route_furniture_allocation as routes
import route_furniture_review as reviews

SCHEMA = 'dfmcp.room-route-handoff/1'
POLICY = 'complete-original-room-and-unchanged-route-request/1'
MAX_BYTES = 100 * 1024
MAX_RESULT_BYTES = 128 * 1024
MAX_DEFICITS = 64
CATEGORIES = ('matched', 'wrong_shape', 'liquid', 'designated', 'occupied_wall', 'hidden', 'missing')


def object_digest(raw: bytes) -> str:
    require(type(raw) is bytes and 1 <= len(raw) <= MAX_BYTES, 'invalid room-route extent')
    return hashlib.sha256(b'dfmcp-room-route-handoff/1\0' + raw).hexdigest()


def _json(raw: bytes, guard: Guard) -> dict:
    require(type(raw) is bytes and 1 <= len(raw) <= MAX_BYTES, 'room-route handoff exceeds bound')
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
            require(depth <= 14, 'room-route nesting exceeds bound')
        elif byte in (93, 125):
            depth -= 1
            require(depth >= 0, 'unbalanced room-route JSON')
    require(depth == 0 and not quoted, 'incomplete room-route JSON')

    def nonfinite(_value: str):
        raise ValueError('nonfinite room-route number')

    value = json.loads(raw, object_pairs_hook=unique, parse_constant=nonfinite)
    require(type(value) is dict and set(value) == {
        'schema', 'policy', 'room_plan', 'route_review', 'route_review_digest'},
        'invalid closed room-route envelope')
    require(value['schema'] == SCHEMA and value['policy'] == POLICY, 'unsupported room-route policy')
    require(canonical(value) == raw, 'room-route handoff is not canonical')
    guard()
    return value


def _requirements(value: dict, guard: Guard) -> dict[tuple[int, int, int], int]:
    """Only called with a regenerated RoomPlan; no caller-selected room subset.

    Expected map/1.5 shapes are floor=3 and wall=2, matching room_terrain_goal.
    Exclusions are retained by the original plan, not expanded into giant masks.
    """
    cells = {}
    for part in value['excavation_blueprint']['parts']:
        x, y, z = part['region']['origin']
        width, height, levels = part['region']['size']
        for zz in range(z, z + levels):
            for yy in range(y, y + height):
                for xx in range(x, x + width):
                    guard()
                    cells[xx, yy, zz] = 3
    kinds = {area['name']: area['template']['kind'] for area in value['intent']['areas']}
    for area in value['areas']:
        if kinds[area['name']] != 'bedroom_cluster':
            continue
        for unit in area['units']:
            x, y, z = unit['origin']
            width, height, _ = unit['size']
            doorway = tuple(unit['doorway'])
            for yy in range(y - 1, y + height + 1):
                for xx in range(x - 1, x + width + 1):
                    guard()
                    point = xx, yy, z
                    if point != doorway and (xx in (x - 1, x + width) or yy in (y - 1, y + height)):
                        require(point not in cells or cells[point] == 2, 'required wall overlaps floor')
                        cells[point] = 2
    return cells


def check_region(room: RoomPlan, origin: tuple[int, int, int], size: tuple[int, int, int],
                 guard: Guard = idle) -> RoomPlan:
    """Validate all floor/corridor/wall coverage BEFORE an acquisition is chosen."""
    guard()
    require(type(room) is RoomPlan and type(room._body) is bytes
            and 1 <= len(room._body) <= MAX_PLAN_BYTES, 'complete original room plan required')
    require(type(origin) is tuple and type(size) is tuple and len(origin) == len(size) == 3,
            'invalid complete room-route region')
    for a, n in zip(origin, size):
        integer(a, 0, 32767)
        integer(n, 1, 128)
        require(a + n <= 32768, 'room-route region exceeds map coordinate bounds')
    require(size[0] * size[1] * size[2] <= routes.MAX_TILES, 'room-route capture exceeds tile bound')
    room = RoomPlan.decode(room.encode(), guard)
    for point in _requirements(room.json(), guard):
        guard()
        require(all(a <= p < a + n for p, a, n in zip(point, origin, size)),
                'route capture omits original room floor, corridor or required wall')
    guard()
    return room


def _parts(value: dict, guard: Guard) -> tuple[RoomPlan, reviews.Review]:
    review_raw = canonical(value['route_review'])
    review = reviews.Review.decode(review_raw, value['route_review_digest'], guard)
    _, request, source, furniture = review.unpack(guard)
    room_raw = canonical(value['room_plan'])
    require(len(room_raw) <= MAX_PLAN_BYTES, 'original room plan exceeds bound')
    room = RoomPlan.decode(room_raw, guard)
    require(canonical(room.request().json()) == canonical(request.json()),
            'route review changed original room furnishing constraints or dependencies')
    room = check_region(room, source.origin, source.size, guard)
    # Source validates region-in-dimensions; complete room coverage above therefore
    # also proves dimension fit. Check native furnishing halos separately.
    furniture.check_dimensions(source.dimensions)
    guard()
    return room, review


@dataclass(frozen=True)
class RoomRouteHandoff:
    raw: bytes
    digest: str

    @classmethod
    def create(cls, room: RoomPlan, review: reviews.Review, guard: Guard = idle) -> RoomRouteHandoff:
        guard()
        require(type(room) is RoomPlan and type(room._body) is bytes
                and 1 <= len(room._body) <= MAX_PLAN_BYTES and type(review) is reviews.Review,
                'complete original room plan and route review required')
        room = RoomPlan.decode(room.encode(), guard)
        raw = canonical({'schema': SCHEMA, 'policy': POLICY, 'room_plan': room.json(),
                         'route_review': review.json(guard), 'route_review_digest': review.digest})
        return cls.decode(raw, object_digest(raw), guard)

    @classmethod
    def decode(cls, raw: bytes, expected_digest: str, guard: Guard = idle) -> RoomRouteHandoff:
        guard()
        reviews.digest_text(expected_digest)
        require(object_digest(raw) == expected_digest, 'room-route handoff differs from selected identity')
        _parts(_json(raw, guard), guard)
        guard()
        return cls(raw, expected_digest)

    def unpack(self, guard: Guard = idle) -> tuple[RoomPlan, reviews.Review]:
        guard()
        reviews.digest_text(self.digest)
        require(object_digest(self.raw) == self.digest, 'room-route bytes changed')
        return _parts(_json(self.raw, guard), guard)

    def json(self, guard: Guard = idle) -> dict:
        self.unpack(guard)
        return json.loads(self.raw)

    def encode(self, guard: Guard = idle) -> bytes:
        self.unpack(guard)
        return self.raw


def _terrain_report(room: RoomPlan, raw: bytes, guard: Guard) -> dict:
    """Re-decode raw map attributes; never reinterpret an excluded route tile as a wall."""
    terrain = routes.decode_terrain(raw, guard)
    required = _requirements(room.json(), guard)
    indices = {}
    for point, expected in required.items():
        guard()
        index = terrain.index(point)
        require(index is not None, 'current map omitted original room geometry')
        indices[index] = point, expected
    reader = routes.Reader(raw)
    # The complete decoder above already checked this exact immutable byte string.
    reader.take(21)  # magic, year, year_tick, pause, site
    reader.take(reader.number('H'))
    reader.take(40)  # dimensions, origin, size, complete cell count
    counts = {domain: dict.fromkeys(CATEGORIES, 0) for domain in ('floors', 'required_walls')}
    deficits, deficit_count, occupied_floors = [], 0, 0
    for index in range(len(terrain.shapes)):
        guard()
        presence = reader.number('B')
        attributes = struct.unpack('>IBBBBBBBIHH', reader.take(19)) if presence == 2 else None
        if index not in indices:
            continue
        point, expected = indices[index]
        domain = 'floors' if expected == 3 else 'required_walls'
        if attributes is None:
            reason = 'hidden' if presence == 1 else 'missing'
        else:
            _, shape, depth, magma, _, dig, building, units, *_ = attributes
            occupied_floors += int(expected == 3 and bool(building or units))
            reason = ('liquid' if depth or magma else 'designated' if dig else
                      'wrong_shape' if shape != expected else
                      'occupied_wall' if expected == 2 and (building or units) else 'matched')
        counts[domain][reason] += 1
        if reason != 'matched':
            deficit_count += 1
            if len(deficits) < MAX_DEFICITS:
                deficits.append({'domain': domain, 'position': list(point), 'reason': reason})
    require(reader.offset == len(raw), 'room terrain assessment missed raw bytes')
    unknown = sum(row['hidden'] + row['missing'] for row in counts.values())
    return {'policy': 'original-room-floor-wall-at-route-sample/1', 'counts': counts,
            'evaluated_tiles': len(required), 'captured_tiles': len(terrain.shapes),
            'occupied_floor_tiles': occupied_floors,
            'deficits': {'count': deficit_count, 'rows': deficits, 'omitted': deficit_count - len(deficits)},
            'status': 'matched' if not deficit_count else 'contradicted' if deficit_count > unknown else 'unknown',
            'all_required_shapes_at_sample': not deficit_count,
            'map_sha256': terrain.digest, 'observed_tick': terrain.tick,
            'continuous_wall_preservation_proven': False, 'room_completion_proven': False}


def revalidate(handoff: RoomRouteHandoff, candidates: tuple[Candidate, ...], native_types: dict[int, int],
               terrain_raw: bytes, source: reviews.Source, guard: Guard = idle,
               *, maximum_work: int = routes.MAX_WORK) -> dict:
    """One captured model checks original geometry, items AND full delivery order.

    For unstarted plans only: no receipt subset, replacement selection or success
    latching. The caller owns acquisition and current authority. This diagnostic
    is not integrated into the placement coordinator's custody or commit gate.
    """
    from route_furniture_suffix import assess_remaining

    work = routes.Work(maximum_work, guard)
    require(type(handoff) is RoomRouteHandoff, 'original room-route handoff required')
    room, review = handoff.unpack(work)
    original, _, baseline, plan = review.unpack(work)
    baseline.permits(source)
    allowance = maximum_work - work.used - 2
    require(allowance > 0, 'no complete room-route review allowance remains')
    individual = reviews.revalidate(review, candidates, native_types, terrain_raw, source,
                                    work.check, maximum_work=allowance)
    work.used += individual['work_units']
    geometry = _terrain_report(room, terrain_raw, work)
    delivery = None
    if individual['status'] == 'constraints_met':
        positions = {row['item']: tuple(row['current_position']) for row in individual['rows']}
        allowance = maximum_work - work.used - 2
        require(allowance > 0, 'no complete room delivery allowance remains')
        delivery = assess_remaining(plan, (), positions, terrain_raw, work.check,
            max_route_steps=original['max_route_steps'], maximum_work=allowance)
        work.used += delivery['work_units']
    ready = (geometry['status'] == 'matched' and individual['status'] == 'constraints_met'
             and delivery is not None and delivery['status'] == 'ready')
    blocked = (geometry['status'] == 'contradicted' or individual['status'] == 'constraints_not_met'
               or (delivery is not None and delivery['status'] == 'blocked'))
    work()
    result = {'schema': 'dfmcp.room-route-check/1', 'handoff_digest': handoff.digest,
              'room_plan_digest': room.digest, 'route_review_digest': review.digest,
              'status': 'ready_in_captured_model' if ready else 'blocked_in_captured_model' if blocked else 'unestablished',
              'room_plan': room.json(), 'route_review': individual, 'room_terrain': geometry,
              'delivery_suffix': delivery, 'plan': plan.json() if ready else None,
              'source': source.json(), 'all_original_room_intent_retained': True,
              'reallocation_performed': False, 'dependencies_changed': False,
              'room_completion_proven': False, 'placement_authorized': False,
              'placement_gate_enforced': False, 'native_pathfinding_proven': False,
              'atomic_route_placement_proven': False, 'game_effect_performed': False,
              'work_units': work.used + 1, 'maximum_work': maximum_work}
    result['evidence_digest'] = hashlib.sha256(b'dfmcp-room-route-check/1\0' + canonical(result)).hexdigest()
    require(len(canonical(result)) <= MAX_RESULT_BYTES, 'complete room-route result exceeds bound')
    work()
    work.check()
    return result
