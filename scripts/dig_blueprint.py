#!/usr/bin/env python3
"""Compile the floor-only subset of excavation-blueprint/1 for native dig/1.16.

This is executable geometry admission, not a terrain observation or permission to
mine. The complete sparse mask is partitioned; its bounding box is NEVER a write.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import sys

import dig_designation_client as d

SCHEMA = 'dfmcp.excavation-blueprint/1'
MAX_INPUT = 16384
MAX_PARTS = 32
MAX_TARGETS = 512
MAX_VOLUME = 1024
MAX_STEPS = 128  # Fits the existing, non-evicting dig directory registry.


def bounded_json(raw: bytes, maximum: int = MAX_INPUT) -> object:
    d.require(type(raw) is bytes and 1 <= len(raw) <= maximum, 'blueprint input exceeds byte bound')
    depth, quoted, escaped = 0, False, False
    for byte in raw:
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
            d.require(depth <= 8, 'blueprint nesting exceeds bound')
        elif byte in (93, 125):
            depth -= 1
    return json.loads(raw.decode('utf-8'), object_pairs_hook=d.unique_object)


def cells(region: dict) -> set[tuple[int, int, int]]:
    x, y, z, w, h = d.region(region)
    return {(px, py, z) for py in range(y, y + h) for px in range(x, x + w)}


@dataclass(frozen=True)
class Layout:
    """Immutable exact mask plus deterministic z/y/x, width-first partition."""
    targets: tuple[tuple[int, int, int], ...]
    rectangles: tuple[tuple[int, int, int, int, int], ...]
    blueprint_bytes: bytes
    blueprint_digest: str

    def regions(self) -> list[dict]:
        return [dict(zip(d.REGION_KEYS, value)) for value in self.rectangles]

    def blueprint(self) -> dict:
        return json.loads(self.blueprint_bytes)

    @property
    def digest(self) -> str:
        # Separate execution domain; no read-only monitor grants mutation authority.
        return d.digest(b'dfmcp-dig-blueprint-layout/1', d.canonical({
            'blueprint_digest': self.blueprint_digest, 'rectangles': self.rectangles,
            'native_profile': 'dig/1.16', 'dig_mode': 'normal'})).hex()

    def json(self) -> dict:
        return {'schema': 'dfmcp.dig-blueprint-layout/1', 'layout_digest': self.digest,
                'blueprint_digest': self.blueprint_digest, 'target_tiles': len(self.targets),
                'steps': self.regions(), 'step_count': len(self.rectangles),
                'native_profile': 'dig/1.16', 'dig_mode': 'normal',
                'unselected_tiles_designated': False, 'terrain_observed': False,
                'mutation_authority_granted': False, 'excavation_completion_proven': False}

    @classmethod
    def decode(cls, raw: bytes) -> Layout:
        return cls.from_json(bounded_json(raw))

    @classmethod
    def from_json(cls, value: object) -> Layout:
        d.require(type(value) is dict and set(value) == {'schema', 'parts'}
                  and value['schema'] == SCHEMA and type(value['parts']) is list
                  and 1 <= len(value['parts']) <= MAX_PARTS, 'invalid blueprint fields')
        parts, count = [], 0
        for part in value['parts']:
            d.require(type(part) is dict and set(part) == {'region', 'shape'}
                      and part['shape'] == 'floor', 'only floor targets admit normal mining')
            region = part['region']
            d.require(type(region) is dict and set(region) == {'origin', 'size'}
                      and type(region['origin']) is list and type(region['size']) is list
                      and len(region['origin']) == len(region['size']) == 3, 'invalid blueprint cuboid')
            origin = tuple(d.integer(n, 1, 32766) for n in region['origin'])
            size = tuple(d.integer(n, 1, 128) for n in region['size'])
            d.require(all(a + n <= 32767 for a, n in zip(origin, size)), 'full dig halo exceeds tile bounds')
            count += size[0] * size[1] * size[2]
            d.require(count <= MAX_TARGETS, 'blueprint exceeds 512 target cells')
            parts.append((origin, size))
        low = tuple(min(p[0][i] for p in parts) for i in range(3))
        high = tuple(max(p[0][i] + p[1][i] for p in parts) for i in range(3))
        size = tuple(b - a for a, b in zip(low, high))
        d.require(max(size) <= 128 and size[0] * size[1] * size[2] <= MAX_VOLUME,
                  'blueprint exceeds coherent monitor capture bounds')
        selected = set()
        for (x, y, z), (w, h, levels) in parts:
            for pz in range(z, z + levels):
                for py in range(y, y + h):
                    for px in range(x, x + w):
                        point = (px, py, pz)
                        d.require(point not in selected, 'overlapping blueprint parts')
                        selected.add(point)
        targets = tuple(sorted(selected, key=lambda p: p[::-1]))
        remaining, rectangles = set(targets), []
        while remaining:
            x, y, z = min(remaining, key=lambda p: p[::-1])
            w = 1
            while w < 8 and (x + w, y, z) in remaining:
                w += 1
            h = 1
            while h < 8 and all((px, y + h, z) in remaining for px in range(x, x + w)):
                h += 1
            rectangle = (x, y, z, w, h)
            remaining.difference_update(cells(dict(zip(d.REGION_KEYS, rectangle))))
            rectangles.append(rectangle)
            d.require(len(rectangles) <= MAX_STEPS, 'blueprint needs more than 128 native intents')
        ordered = sorted(parts, key=lambda p: (p[0][::-1], p[1][::-1]))
        blueprint = {'schema': SCHEMA, 'parts': [
            {'region': {'origin': list(o), 'size': list(s)}, 'shape': 'floor'} for o, s in ordered]}
        # Same semantic-mask digest as the existing read-only Blueprint model.
        sx, sy, _ = size
        indices = [(((z - low[2]) * sy + y - low[1]) * sx + x - low[0], 3) for x, y, z in targets]
        mask = {'region': {'origin': list(low), 'size': list(size)}, 'targets': indices}
        identity = hashlib.sha256(b'dfmcp-excavation-blueprint/1\0' + d.canonical(mask)).hexdigest()
        return cls(targets, tuple(rectangles), d.canonical(blueprint), identity)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('blueprint', type=Path)
    args = parser.parse_args(argv)
    try:
        with args.blueprint.open('rb') as source:
            layout = Layout.decode(source.read(MAX_INPUT + 1))
        print(d.canonical(layout.json()).decode('ascii'))
        return 0
    except (ValueError, OSError, TypeError, KeyError, RecursionError):
        print(d.canonical({'ok': False, 'error': 'Invalid or unsupported bounded floor blueprint.',
                           'game_mutation_dispatched': False}).decode('ascii'))
        return 2


if __name__ == '__main__':
    sys.exit(main())
