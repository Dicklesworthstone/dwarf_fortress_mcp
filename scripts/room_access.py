"""Caller-selected access point and bounded routing domain; no terrain authority.

The point is an intended floor requirement, not evidence of an existing fortress
entrance. Observed layout additionally checks its original raw map observation.
Beads: dfmcp-wp-pln01-spatial-blueprint-layout-pwd; df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

from dataclasses import dataclass
from furniture_plan import integer, require

Point = tuple[int, int, int]


def coordinate(value: object) -> Point:
    require(type(value) is list and len(value) == 3, 'access point needs three exact coordinates')
    return tuple(integer(n, 1, 32766) for n in value)


@dataclass(frozen=True)
class Access:
    point: Point
    origin: Point
    size: Point

    @classmethod
    def from_json(cls, value: object) -> Access:
        require(type(value) is dict and set(value) == {'point', 'region'},
                'access requires one point and one exact routing region')
        region = value['region']
        require(type(region) is dict and set(region) == {'origin', 'size'},
                'invalid access routing region')
        point, origin = coordinate(value['point']), coordinate(region['origin'])
        dimensions = region['size']
        require(type(dimensions) is list and len(dimensions) == 3, 'invalid access routing dimensions')
        size = tuple(integer(n, 1, 128) for n in dimensions)
        require(size[2] == 1 and size[0] * size[1] <= 1024
                and all(a + n <= 32767 for a, n in zip(origin, size)),
                'access routing requires one bounded same-level region with native halo')
        result = cls(point, origin, size)
        require(result.contains(point), 'access point is outside the routing region')
        return result

    def contains(self, point: Point) -> bool:
        return all(a <= p < a + n for p, a, n in zip(point, self.origin, self.size))

    def json(self) -> dict:
        return {'point': list(self.point),
                'region': {'origin': list(self.origin), 'size': list(self.size)}}
