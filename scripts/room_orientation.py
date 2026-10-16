"""Quarter-turn room geometry before plan sealing, never a native mutation.

The existing template compiler owns every shape and constraint. Rotate its
complete result about the requested area's origin, including unit rectangles,
doorways, furniture and entries; downstream circulation and wall checks still
run on that result. Bead: dfmcp-wp-pln01-spatial-blueprint-layout-pwd.
"""
from __future__ import annotations

from dataclasses import replace
from typing import TYPE_CHECKING, Callable

from furniture_plan import integer, require

if TYPE_CHECKING:
    from room_provisioning import _Rect

Point = tuple[int, int, int]
ROTATIONS = (0, 90, 180, 270)


def rotation(value: object) -> int:
    require(type(value) is int and value in ROTATIONS,
            'room rotation must be an exact quarter-turn: 0, 90, 180 or 270')
    return value


def compile_rotated_area(value: dict, guard: Callable[[], None], compile_base: Callable
                         ) -> tuple[dict, list[_Rect], list[dict], list[dict], Point]:
    """Rotate clockwise in screen coordinates (+x right, +y down).

    Compile at a neutral interior point first, so an unrotated shape crossing
    the map edge cannot incorrectly reject a valid rotated shape. Zero uses the
    original origin and emits no new field, preserving legacy canonical bytes.
    """
    guard()
    angle = rotation(value['rotation'])
    base = {key: item for key, item in value.items() if key != 'rotation'}
    if angle == 0:
        return compile_base(base, guard)
    origin = value['origin']
    require(type(origin) is list and len(origin) == 3, 'three exact coordinates required')
    origin = tuple(integer(n, 1, 32766) for n in origin)
    neutral = (128, 128, origin[2])
    base['origin'] = list(neutral)
    area, parts, units, slots, entry = compile_base(base, guard)

    def turn(point: Point) -> Point:
        guard()
        x, y = point[0] - neutral[0], point[1] - neutral[1]
        if angle == 90:
            x, y = -y, x
        elif angle == 180:
            x, y = -x, -y
        else:
            x, y = y, -x
        result = (origin[0] + x, origin[1] + y, point[2])
        require(all(1 <= n <= 32766 for n in result),
                'rotated room geometry or full native halo exceeds coordinate bounds')
        return result

    def bounds(start: Point, size: Point) -> tuple[Point, Point]:
        # Inclusive tile corners, NOT the exclusive rectangle end. Quarter-turns
        # preserve axis alignment and volume; no bounding-hull excavation grows.
        first = turn(start)
        last = turn(tuple(a + n - 1 for a, n in zip(start, size)))
        low = tuple(min(a, b) for a, b in zip(first, last))
        extent = tuple(abs(a - b) + 1 for a, b in zip(first, last))
        return low, extent

    rotated_parts = []
    for part in parts:
        low, size = bounds(part.origin, part.size)
        rotated_parts.append(replace(part, origin=low, size=size))
    rotated_units = []
    for unit in units:
        low, size = bounds(tuple(unit['origin']), tuple(unit['size']))
        door = unit['doorway']
        rotated_units.append({**unit, 'origin': list(low), 'size': list(size),
                              'doorway': None if door is None else list(turn(tuple(door)))})
    rotated_slots = [{**slot, 'target': list(turn(tuple(slot['target'])))} for slot in slots]
    result = ({**area, 'origin': list(origin), 'rotation': angle}, rotated_parts,
              rotated_units, rotated_slots, turn(entry))
    guard()
    return result
