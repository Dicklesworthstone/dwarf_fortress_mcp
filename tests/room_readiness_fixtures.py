"""Synthetic map/operations bytes; placement receipts use the production codec.

These fixtures are NOT native DFHack execution or acquisition attestations.
"""
from dataclasses import replace
import struct

import build_placement_wire as w
import construction_plan as c
from construction_receipt import Manifest
from furniture_allocation import Candidate
from furniture_handoff import Handoff, InventorySource, Selected
from room_furniture_handoff import RoomFurnitureHandoff
from room_provisioning import RoomPlan
import room_readiness as r
import room_terrain

SOFTWARE = ('test-df', 'test-dfhack')
BUILD = Manifest(41, *SOFTWARE)
OPERATIONS = Manifest(17, *SOFTWARE)
MAP = Manifest(29, *SOFTWARE)
DIMENSIONS = (128, 128, 16)


def room(large=False):
    area = {'name': 'rooms', 'origin': [10, 10, 2], 'template': (
        {'kind': 'dining_hall', 'table_count': 16, 'columns': 4} if large else
        {'kind': 'bedroom_cluster', 'rooms_count': 2, 'room_size': [3, 3]})}
    return RoomPlan.compile({'schema': 'dfmcp.room-provisioning-request/1', 'world_folder': 'region1',
        'site': 2, 'areas': [area], 'excluded_items': list(range(30000, 30646)) if large else [30000]})


def handoff(plan=None, address='127.0.0.1:5000'):
    plan = room() if plan is None else plan
    selected = tuple(Selected(s.name, Candidate(100 + i, s.kind, s.target, (0, 2), -1),
                              100 + w.KINDS.index(s.kind)) for i, s in enumerate(plan.request().slots))
    allocation = Handoff(plan.request(), InventorySource(address, OPERATIONS.generation, *SOFTWARE,
        'a' * 64, 1024, 100, (1000, 2000, 10000)), selected)
    return RoomFurnitureHandoff(plan, allocation)


def receipts(h):
    records = []
    for i, step in enumerate(h.allocation.plan().ordered):
        kind = w.KINDS.index(step.kind)
        selected = w.Selection(kind, step.item, *step.target)
        before = w.Capture(BUILD.generation, i, 200, 2, DIMENSIONS, 2000 + i, 1000 + i, i,
            'region1', True, True, True, selected, (w.Tile(2, 42, 3),) * 9,
            w.Item(2, step.target, kind, 100 + kind, -1, 0, 2, on_ground=True, ground=w.Tile(2, 42, 3)))
        native = w.Plan('readiness-fixture-' + step.name, before)
        insertion = w.Insertion(before.next_building, before.next_job, step.item, kind, step.target,
                               0, 2, 0, 3, True, True, True, False)
        records.append(w.Record(native, 'placed', 'none', before.expected_after(), insertion).raw)
    return tuple(records)


def goal(large=False, **policy):
    h = handoff(room(large))
    return r.Goal(h, c.Goal(receipts(h), 10000, stable_span=policy.pop('stable_span', 10), **policy))


def text(s):
    raw = s.encode()
    return struct.pack('>H', len(raw)) + raw


def operations(goal, tick, *, pending_item=None, missing_building=None, removal=False,
               extra_items=0, folder='region1', site=2, paused=True, horizons=None):
    records = [w.Record.decode(raw) for raw in goal.condition.receipts]
    count = len(records)
    horizons = horizons or (1000 + count, 2000 + count, 10000 + extra_items)
    jobs = []
    if removal:
        p = records[0].insertion
        jobs.append(struct.pack('>Ii', p.job, 9) + text('DestroyBuilding') + text('') + b'\0\0'
            + struct.pack('>3i', *p.pos) + b'\0\x01' + struct.pack('>I', p.building)
            + struct.pack('>iII', -1, 0, 0))
    j = (b'DFMJ1200' + struct.pack('>IIBiI', tick // 403200, tick % 403200, int(paused), site, horizons[0])
         + text(folder) + struct.pack('>I', len(jobs)) + b''.join(jobs))
    buildings, items = [], []
    for rec in records:
        p = rec.insertion
        x, y, z = p.pos
        if p.building != missing_building:
            buildings.append(struct.pack('>Ii', p.building, 50 + p.kind)
                + text(('', 'Bed', 'Chair', 'Table')[p.kind])
                + struct.pack('>7i', x, y, x, y, z, 3, 3))
        holder = b'\0' if p.item == pending_item or p.building == missing_building else b'\x01' + struct.pack('>I', p.building)
        items.append((p.item, struct.pack('>Ii', p.item, 100 + p.kind)
            + text(('', 'BED', 'CHAIR', 'TABLE')[p.kind]) + struct.pack('>iiiI3iI', -1, 0, 2, 1, x, y, z,
                64 if p.item == pending_item else 256) + b'\0' + holder))
    for i in range(extra_items):
        identity = 10000 + i
        items.append((identity, struct.pack('>Ii', identity, 300) + text('BOULDER')
            + struct.pack('>iiiI3iI', -1, 0, 2, 1, 1, 1, 2, 64) + b'\0\0'))
    return (b'DFMO1400' + struct.pack('>I', len(j)) + j + struct.pack('>II', horizons[1], horizons[2])
            + struct.pack('>I', len(buildings)) + b''.join(buildings)
            + struct.pack('>I', len(items)) + b''.join(raw for _, raw in sorted(items)) + struct.pack('>I', 0))


def tile(shape=2, *, depth=0, magma=0, dig=0, building=0, units=0):
    return b'\x02' + struct.pack('>IBBBBBBBIHH', 42, shape, depth, magma, 0, dig, building, units, 1, 10015, 10015)


def map_capture(goal, tick, *, overrides=None, paused=True, dimensions=DIMENSIONS, folder='region1', site=2):
    selected = room_terrain.selection(goal.room.room_plan)
    region = selected.region
    header = (b'DFMM1500' + struct.pack('>IIBIH', tick // 403200, tick % 403200, int(paused), site, len(folder.encode()))
        + folder.encode() + struct.pack('>10I', *dimensions, *region.origin, *region.size, region.volume))
    tiles = {p: tile(3) for p in selected.floors}
    # Installed furniture occupies floors, but is not a reason to call the room unfinished.
    tiles.update({s.target: tile(3, building=1) for s in goal.room.allocation.plan().steps})
    tiles.update(overrides or {})
    return header + b''.join(tiles.get(p, tile()) for p in room_terrain.points(region, lambda: None))


def sample(goal, tick=300, *, overrides=None, map_manifest=MAP, **ops):
    raw = map_capture(goal, tick, overrides=overrides)
    linked = c.LinkedSample(BUILD, goal.condition.receipts, OPERATIONS, operations(goal, tick, **ops),
                            BUILD, goal.condition.receipts)
    return r.Sample(map_manifest, raw, linked, map_manifest, raw)
