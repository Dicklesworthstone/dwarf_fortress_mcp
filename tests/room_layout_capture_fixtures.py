"""Synthetic native map payloads; no game acquisition or producer attestation."""
from __future__ import annotations
import struct
from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import excavation_observer as native
import room_layout_excavation as compiler

MANIFEST = native.Manifest(23, 'test-df', 'test-dfhack')
ADDRESS = '127.0.0.1:5000'


def tile(shape=2, *, presence=2, depth=0, magma=0, dig=0, building=0, units=0):
    if presence != 2:
        return bytes([presence])
    return b'\x02' + struct.pack('>IBBBBBBBIHH', 97, shape, depth, magma, 0, dig,
                                  building, units, 19, 10015, 10016)


def capture(region, overrides=None, *, default=None, folder='region1', site=2,
            tick=101, paused=True, dimensions=(256, 256, 8)):
    folder = folder.encode('utf-8')
    raw = bytearray(b'DFMM1500' + struct.pack('>IIBIH', tick // 403200, tick % 403200,
                                            int(paused), site, len(folder)) + folder)
    raw.extend(struct.pack('>10I', *dimensions, *region.origin, *region.size, region.volume))
    overrides = overrides or {}
    default = tile() if default is None else default
    x, y, z = region.origin
    w, h, levels = region.size
    for zz in range(z, z + levels):
        for yy in range(y, y + h):
            for xx in range(x, x + w):
                raw.extend(overrides.get((xx, yy, zz), default))
    return bytes(raw)


def request(*, access=True, size=(10, 6, 1), origin=(10, 10, 2), fixed=True):
    area = {'name': 'hall', 'template': {'kind': 'dining_hall', 'table_count': 1}}
    if fixed:
        area['origin'] = [origin[0] + 6, origin[1], origin[2]]
    value = {'schema': 'dfmcp.room-layout-request/1', 'world_folder': 'region1', 'site': 2,
             'region': {'origin': list(origin), 'size': list(size)}, 'areas': [area]}
    if access:
        value['access_point'] = list(origin)
    return value


def points(region):
    x, y, z = region['origin']
    w, h, levels = region['size']
    return {(xx, yy, zz) for zz in range(z, z + levels)
            for yy in range(y, y + h) for xx in range(x, x + w)}


def mask(blueprint):
    return set().union(*(points(p['region']) for p in blueprint['parts']))
