"""Finite per-cell planning constraints shared by room placement and routing.

These are desired-use permissions, not terrain facts or mutation authority.
A dot permits floor/wall, F permits floor only, W wall only, X neither. Outside
this same-level mask neither use is allowed. Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Callable

from furniture_plan import integer, require

MAX_CELLS = 1024
Point = tuple[int, int, int]


def _idle() -> None:
    """Pure parsing has fixed structural bounds."""


@dataclass(frozen=True)
class SiteMask:
    origin: Point
    size: Point
    cells: str

    def __post_init__(self) -> None:
        require(type(self.origin) is tuple and type(self.size) is tuple
                and len(self.origin) == len(self.size) == 3, 'invalid room site region')
        for start, size in zip(self.origin, self.size):
            integer(start, 1, 32766)
            integer(size, 1, 128)
            require(start + size <= 32767, 'room site region exceeds coordinate bounds')
        require(self.size[2] == 1 and self.size[0] * self.size[1] <= MAX_CELLS,
                'room site mask requires one level and at most 1024 cells')
        require(type(self.cells) is str and len(self.cells) == self.size[0] * self.size[1]
                and all(c in '.FWX' for c in self.cells), 'invalid complete room site cell mask')

    @classmethod
    def from_json(cls, value: object, guard: Callable[[], None] = _idle) -> SiteMask:
        guard()
        require(type(value) is dict and set(value) == {'region', 'cells'}, 'invalid room site mask fields')
        region = value['region']
        require(type(region) is dict and set(region) == {'origin', 'size'}
                and type(region['origin']) is list and type(region['size']) is list
                and len(region['origin']) == len(region['size']) == 3, 'invalid room site region fields')
        result = cls(tuple(region['origin']), tuple(region['size']), value['cells'])
        for _ in result.cells:
            guard()
        return result

    def json(self) -> dict:
        return {'region': {'origin': list(self.origin), 'size': list(self.size)}, 'cells': self.cells}

    def at(self, point: Point) -> str:
        # Coordinates come from bounded compiler loops, not direct JSON indexing.
        x, y, z = (p - a for p, a in zip(point, self.origin))
        if z != 0 or not (0 <= x < self.size[0] and 0 <= y < self.size[1]):
            return 'X'
        return self.cells[y * self.size[0] + x]

    def permits_floor(self, point: Point) -> bool:
        return self.at(point) in '.F'

    def permits_wall(self, point: Point) -> bool:
        return self.at(point) in '.W'
