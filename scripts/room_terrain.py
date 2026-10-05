"""Survey complete room intentions and propose only their remaining wall targets.

Pure map/1.5 evidence reduction. A proposal is not native mining eligibility,
mutation permission, a replacement effect key, or a completed room project.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5 (all remain open).
"""
from __future__ import annotations

from collections import Counter
from dataclasses import dataclass
import hashlib

import excavation_observer as e
from furniture_allocation import Guard, idle
from furniture_plan import canonical, require
from room_provisioning import MAX_PARTS, MAX_PLAN_BYTES, MAX_TILES, Point, RoomPlan

SCHEMA = 'dfmcp.room-terrain-survey/1'
POLICY = 'dfmcp.room-remaining-walls/1'
MAX_CAPTURE_TILES = 4096
MAX_CAPTURE_BYTES = 575 + 20 * MAX_CAPTURE_TILES
MAX_OUTPUT = 131072
MAX_BLOCKERS = 128
MAX_NATIVE_STEPS = 128
TARGET_KINDS = ('observed_floor', 'remaining_wall', 'hidden', 'missing', 'liquid',
                'active_designation', 'occupied_wall', 'unsupported_shape')
WALL_KINDS = ('wall_shape', 'hidden', 'missing', 'liquid', 'active_designation',
              'occupied_wall', 'opened_or_other_shape')
HALO_KINDS = ('visible_dry_undesignated', 'hidden', 'missing', 'liquid', 'active_designation')


def order(point: Point) -> tuple[int, int, int]:
    return point[::-1]  # Fixed native z/y/x order, not set iteration order.


def points(region: e.Region, guard: Guard):
    x, y, z = region.origin
    w, h, levels = region.size
    for pz in range(z, z + levels):
        for py in range(y, y + h):
            for px in range(x, x + w):
                guard()
                yield px, py, pz


@dataclass(frozen=True)
class Selection:
    plan: RoomPlan
    floors: frozenset[Point]
    walls: frozenset[Point]
    region: e.Region


def selection(plan: RoomPlan, guard: Guard = idle) -> Selection:
    """Regenerate original intent before trusting even a caller-built RoomPlan."""
    guard()
    require(type(plan) is RoomPlan and type(plan._body) is bytes
            and 1 <= len(plan._body) <= MAX_PLAN_BYTES, 'bounded exact room plan required')
    plan = RoomPlan.decode(plan.encode(), guard)
    body = plan.json()
    footprint = e.Region.from_json(body['capture_region'])
    # Observe one padded bounding capture, including vertical context. Never
    # clamp or split a coherent sample when the native axis/volume limit is hit.
    region = e.Region(tuple(n - 1 for n in footprint.origin), tuple(n + 2 for n in footprint.size))
    require(region.volume <= MAX_CAPTURE_TILES, 'padded room survey exceeds coherent capture bound')
    floors: set[Point] = set()
    for part in body['excavation_blueprint']['parts']:
        floors.update(points(e.Region.from_json(part['region']), guard))
    require(len(floors) == body['target_tiles'] <= MAX_TILES, 'room mask cardinality changed')
    kinds = {area['name']: area['template']['kind'] for area in body['intent']['areas']}
    walls: set[Point] = set()
    for area in body['areas']:
        if kinds[area['name']] != 'bedroom_cluster':
            continue
        for unit in area['units']:
            x, y, z = unit['origin']
            w, h, _ = unit['size']
            doorway = tuple(unit['doorway'])
            for py in range(y - 1, y + h + 1):
                for px in range(x - 1, x + w + 1):
                    guard()
                    point = px, py, z
                    if (px in (x - 1, x + w) or py in (y - 1, y + h)) and point != doorway:
                        walls.add(point)
    require(not floors & walls, 'room floors conflict with required separating walls')
    guard()
    return Selection(plan, frozenset(floors), frozenset(walls), region)


def partition(mask: frozenset[Point], span: int, guard: Guard = idle) -> list[dict]:
    """Bounded exact z/y/x width-first cover, not a minimum rectangle claim.

    A 128-wide part is an existing blueprint cuboid; the independent 8-wide
    partition counts the normal-mining client's bounded native rectangles.
    Neither representation contains any unselected coordinate.
    """
    require(type(mask) is frozenset and len(mask) <= MAX_TILES and type(span) is int
            and span in (8, 128), 'invalid bounded residual mask')
    for point in mask:
        guard()
        require(type(point) is tuple and len(point) == 3, 'invalid residual coordinate')
        for value in point:
            e.integer(value, 1, 32766)
    remaining, parts = set(mask), []
    while remaining:
        guard()
        x, y, z = min(remaining, key=order)
        width = 1
        while width < span and (x + width, y, z) in remaining:
            guard()
            width += 1
        height = 1
        while height < span:
            guard()
            if not all((px, y + height, z) in remaining for px in range(x, x + width)):
                break
            height += 1
        region = e.Region((x, y, z), (width, height, 1))
        remaining.difference_update(points(region, guard))
        parts.append({'region': region.json(), 'shape': 'floor'})
    return parts


def mask_digest(mask: frozenset[Point]) -> str | None:
    """The existing excavation-blueprint semantic identity, not a dig authority."""
    if not mask:
        return None
    low = tuple(min(p[i] for p in mask) for i in range(3))
    size = tuple(max(p[i] for p in mask) + 1 - low[i] for i in range(3))
    targets = [(((z - low[2]) * size[1] + y - low[1]) * size[0] + x - low[0], 3)
               for x, y, z in sorted(mask, key=order)]
    body = {'region': {'origin': list(low), 'size': list(size)}, 'targets': targets}
    return hashlib.sha256(b'dfmcp-excavation-blueprint/1\0' + canonical(body)).hexdigest()


def _problem(tile: e.Tile) -> str | None:
    if tile.presence != 2:
        return 'hidden' if tile.presence == 1 else 'missing'
    _, _, depth, magma, _, dig, *_ = tile.attributes
    if depth or magma:
        return 'liquid'
    if dig:
        return 'active_designation'
    return None


def survey(plan: RoomPlan, raw: bytes, manifest: e.Manifest, guard: Guard = idle) -> dict:
    """Reduce one complete raw capture; never trust derived caller tile objects.

    Evidence is historical and supplied by the caller. Only the separate live
    command can establish that it acquired these bytes through the native reader.
    """
    selected = selection(plan, guard)
    require(type(raw) is bytes and 1 <= len(raw) <= MAX_CAPTURE_BYTES,
            'room survey capture byte bound exceeded')
    require(type(manifest) is e.Manifest, 'exact map source manifest required')
    manifest = e.Manifest.from_json(manifest.json())
    capture = e.decode_capture(raw, manifest, selected.region)
    guard()
    body = selected.plan.json()
    require((capture.folder, capture.site) == (body['intent']['world_folder'], body['intent']['site']),
            'room survey is not from the requested fortress')
    tiles = dict(zip(points(selected.region, guard), capture.tiles))
    target_counts, wall_counts, halo_counts = (dict.fromkeys(kinds, 0) for kinds in
                                               (TARGET_KINDS, WALL_KINDS, HALO_KINDS))
    deficits, reasons = [], Counter()

    def deficit(domain: str, reason: str, point: Point | None = None) -> None:
        reasons[domain + '.' + reason] += 1
        if len(deficits) < MAX_BLOCKERS:
            deficits.append({'domain': domain, 'reason': reason,
                             'coordinate': None if point is None else list(point)})

    remaining: set[Point] = set()
    occupied_floors = 0
    for point in sorted(selected.floors, key=order):
        guard()
        tile = tiles[point]
        category = _problem(tile)
        if category is None:
            shape, building, units = tile.attributes[1], tile.attributes[6], tile.attributes[7]
            if shape == 3:
                category = 'observed_floor'
                occupied_floors += int(bool(building or units))
            elif shape == 2:
                category = 'occupied_wall' if building or units else 'remaining_wall'
            else:
                category = 'unsupported_shape'
        target_counts[category] += 1
        if category == 'remaining_wall':
            remaining.add(point)
        elif category != 'observed_floor':
            deficit('target', category, point)
    for point in sorted(selected.walls, key=order):
        guard()
        tile = tiles[point]
        category = _problem(tile)
        if category is None:
            if tile.attributes[1] != 2:
                category = 'opened_or_other_shape'
            elif tile.attributes[6] or tile.attributes[7]:
                category = 'occupied_wall'
            else:
                category = 'wall_shape'
        wall_counts[category] += 1
        if category != 'wall_shape':
            deficit('required_wall', category, point)
    # Only the exact remaining-mask halo constrains an excavation proposal.
    # Padded capture holes are not extra targets, and unrelated hidden gaps do
    # not become false floor deficits. Halo checks do not prove support/pressure.
    halo: set[Point] = set()
    for x, y, z in sorted(remaining, key=order):
        for dz in (-1, 0, 1):
            for dy in (-1, 0, 1):
                for dx in (-1, 0, 1):
                    guard()
                    halo.add((x + dx, y + dy, z + dz))
    halo.difference_update(remaining)
    for point in sorted(halo, key=order):
        guard()
        reason = _problem(tiles[point])
        halo_counts[reason or 'visible_dry_undesignated'] += 1
        if reason:
            deficit('remaining_halo', reason, point)
    remaining_mask = frozenset(remaining)
    parts = partition(remaining_mask, 128, guard)
    # A global greedy cover can need more rectangles than the original room
    # partition (notably ten bedrooms). Also clip each ORIGINAL disjoint part
    # to this same residual mask; choose the smaller exact cover deterministically.
    local_parts = []
    for part in body['excavation_blueprint']['parts']:
        clipped = frozenset(points(e.Region.from_json(part['region']), guard)) & remaining_mask
        local_parts.extend(partition(clipped, 128, guard))
    local_parts.sort(key=lambda p: (p['region']['origin'][::-1], p['region']['size'][::-1]))
    parts = min((parts, local_parts), key=lambda group: (len(group), canonical(group)))
    native_parts = partition(remaining_mask, 8, guard)
    if len(parts) > MAX_PARTS:
        deficit('capacity', 'residual_exceeds_32_blueprint_parts')
    if len(native_parts) > MAX_NATIVE_STEPS:
        deficit('capacity', 'residual_exceeds_128_native_steps')
    blocked = bool(reasons)
    blueprint = None
    if remaining and not blocked:
        blueprint = {'schema': 'dfmcp.excavation-blueprint/1', 'parts': parts}
        require(len(canonical(blueprint)) <= 16384, 'residual blueprint exceeds existing byte allowance')
    status = ('blocked' if blocked else 'excavation_proposed' if remaining
              else 'terrain_shapes_satisfied_at_sample')
    count = sum(reasons.values())
    result = {
        'schema': SCHEMA, 'policy': POLICY, 'status': status, 'room_plan': body,
        'source': {'profile': 'map/1.5', **capture.binding(), 'game_tick': capture.tick,
                   'paused_at_capture': capture.paused, 'observation_witness': capture.witness,
                   'capture_sha256': hashlib.sha256(raw).hexdigest(), 'capture_bytes': len(raw)},
        'coverage': {'captured_tiles': selected.region.volume, 'target_tiles': len(selected.floors),
                     'required_wall_tiles': len(selected.walls), 'remaining_halo_tiles': len(halo),
                     'unused_capture_tiles': len(tiles.keys() - (selected.floors | selected.walls | halo))},
        'target_counts': target_counts, 'required_wall_counts': wall_counts,
        'remaining_halo_counts': halo_counts, 'occupied_floor_tiles': occupied_floors,
        'remaining_wall_targets': [list(p) for p in sorted(remaining, key=order)],
        'remaining_mask_digest': mask_digest(remaining_mask), 'remaining_blueprint': blueprint,
        'partition': {'blueprint_parts': len(parts), 'normal_mining_rectangles': len(native_parts),
                      'policy': 'smaller_global_or_original_part_width_first_cover', 'minimum_parts_proven': False},
        'blockers': {'count': count, 'counts': dict(sorted(reasons.items())),
                     'shown': len(deficits), 'omitted': count - len(deficits), 'rows': deficits},
        'terrain_shapes_satisfied_at_sample': status == 'terrain_shapes_satisfied_at_sample',
        'completion_goal_location': 'room_plan.excavation_blueprint',
        'residual_completion_is_room_completion': False,
        'mutation_authority_granted': False, 'native_excavation_eligibility_proven': False,
        'existing_effects_reconciled': False, 'replacement_effect_key_authorized': False,
        'current_terrain_proven': False, 'continuous_wall_preservation_proven': False,
        'structural_safety_proven': False, 'native_pathfinding_proven': False,
        'room_assignments_created': False, 'room_completion_proven': False,
        'construction_completion_proven': False, 'production_admitted': False,
    }
    result['survey_digest'] = hashlib.sha256(b'dfmcp-room-terrain-survey/1\0' + canonical(result)).hexdigest()
    require(len(canonical(result)) <= MAX_OUTPUT - 4096, 'room survey exceeds complete result allowance')
    guard()
    return result
