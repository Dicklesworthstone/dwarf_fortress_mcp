"""Independent map/1.5 byte fixtures for room surveys; no native game execution."""
from __future__ import annotations

import struct

import excavation_observer as e
from room_provisioning import RoomPlan

MANIFEST = e.Manifest(7, 'test-df', 'test-dfhack')


def bedroom(count=2, size=(3, 3), *, origin=(10, 10, 2), name='sleep') -> dict:
    return {'name': name, 'origin': list(origin),
            'template': {'kind': 'bedroom_cluster', 'rooms_count': count, 'room_size': list(size)}}


def dining(count=3, columns=3, *, origin=(10, 10, 2), name='eat') -> dict:
    return {'name': name, 'origin': list(origin),
            'template': {'kind': 'dining_hall', 'table_count': count, 'columns': columns}}


def request(*areas) -> dict:
    return {'schema': 'dfmcp.room-provisioning-request/1', 'world_folder': 'region1',
            'site': 2, 'areas': list(areas or (bedroom(),))}


def plan(*areas) -> RoomPlan:
    return RoomPlan.compile(request(*areas))


def coordinates(region: e.Region) -> list[tuple[int, int, int]]:
    x, y, z = region.origin
    w, h, levels = region.size
    return [(a, b, c) for c in range(z, z + levels) for b in range(y, y + h) for a in range(x, x + w)]


def visible(shape=2, *, depth=0, magma=0, dig=0, building=0, units=0) -> bytes:
    return b'\x02' + struct.pack('>IBBBBBBBIHH', 42, shape, depth, magma, 0,
                                dig, building, units, 1, 10015, 10015)


def capture(region: e.Region, overrides=None, *, folder='region1', site=2,
            dimensions=(32768, 32768, 32768), tick=100, paused=True) -> bytes:
    folder_bytes = folder.encode('utf-8')
    header = (b'DFMM1500' + struct.pack('>IIBIH', tick // 403200, tick % 403200,
              int(paused), site, len(folder_bytes)) + folder_bytes
              + struct.pack('>10I', *dimensions, *region.origin, *region.size, region.volume))
    overrides = overrides or {}
    return header + b''.join(overrides.get(point, visible()) for point in coordinates(region))


def expand(blueprint: dict) -> set[tuple[int, int, int]]:
    result = set()
    for part in blueprint['parts']:
        assert part['shape'] == 'floor'
        group = set(coordinates(e.Region.from_json(part['region'])))
        assert not result & group, 'overlapping output rectangles'
        result.update(group)
    return result
