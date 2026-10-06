"""Real room/journal values with independent map bytes; not game acquisition."""
from pathlib import Path

import excavation_observer as e
import room_terrain as terrain
import room_terrain_goal as r
import track_excavation as t
import furniture_allocation as allocation
from furniture_handoff import Handoff, InventorySource, Selected
from room_furniture_handoff import RoomFurnitureHandoff
import room_terrain_fixtures as f


def capture(plan, tick=120, overrides=None, **kwargs):
    selected = terrain.selection(plan)
    tiles = {point: f.visible(3) for point in selected.floors}
    tiles.update(overrides or {})
    return e.decode_capture(f.capture(selected.region, tiles, tick=tick, **kwargs), f.MANIFEST, selected.region)


def room_allocation(plan, address, tick=115):
    request = plan.request()
    candidates = tuple(allocation.Candidate(100 + i, slot.kind, slot.target,
        slot.material or (0, 2), -1 if slot.subtype is None else slot.subtype)
        for i, slot in enumerate(request.slots))
    result = allocation.allocate(request, candidates)
    assert result['status'] == 'allocated'
    by_id = {item.id: item for item in candidates}
    kinds = {'bed': 1, 'chair': 2, 'table': 3}
    chosen = tuple(Selected(row['slot'], by_id[row['item']], kinds[row['kind']])
                   for row in result['assignments'])
    source = InventorySource(address, 987, 'test-df', 'test-dfhack', 'a' * 64, 1000, tick, (200, 100, 1000))
    return RoomFurnitureHandoff(plan, Handoff(request, source, chosen))


def write_history(path, plan, address, *, ticks=(100, 110), cancel=False, overrides=None):
    goal = r.RoomTerrainGoal(plan, 1000)
    with t.open_journal(Path(path), t.Budget(60000), writable=True, create=True) as owner:
        owner.append({'kind': 'begin', 'format': t.ROOM_PROFILE.format, 'nonce': 'a' * 64,
                      'endpoint': address, 'goal': goal.json(),
                      'sample': t.sample_value(capture(plan, ticks[0], overrides))})
        for tick in ticks[1:]:
            owner.append({'kind': 'read_started'})
            owner.append({'kind': 'sample', 'sample': t.sample_value(capture(plan, tick, overrides))})
        if cancel:
            owner.append({'kind': 'cancel'})
    return goal
