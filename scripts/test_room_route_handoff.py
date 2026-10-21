"""Actual room compiler, route codec, allocator and combined-condition tests."""
from __future__ import annotations

from dataclasses import replace
import hashlib
import json
import struct
import unittest
from unittest.mock import patch

from furniture_allocation import Candidate, Request
from furniture_plan import canonical
from room_provisioning import RoomPlan
import room_route_handoff as model
import route_furniture_allocation as routes
import route_furniture_review as reviews


def room_plan(*, dining=False, count=1, rotation=0, vertical=0):
    template = ({'kind': 'dining_hall', 'table_count': count, 'columns': 4} if dining else
                {'kind': 'bedroom_cluster', 'rooms_count': count, 'room_size': [3, 3]})
    area = {'name': 'rooms', 'origin': [5, 5, 2], 'template': template}
    if rotation:
        area.update(origin=[15, 15, 2], rotation=rotation)
    if vertical:
        area['item_constraints'] = {kind: {'max_vertical_distance': vertical}
                                   for kind in ('chair', 'table') if dining or kind != 'bed'}
    return RoomPlan.compile({'schema': 'dfmcp.room-provisioning-request/1',
        'world_folder': 'fort', 'site': 7, 'areas': [area], 'excluded_items': [999]})


def raw_map(room, *, origin=(0, 0, 2), size=(32, 32, 1), changes=None, tick=10):
    """Independent test emitter: default floor plus explicitly required wall rings."""
    walls = set()
    intent = room.json()['intent']
    kinds = {area['name']: area['template']['kind'] for area in intent['areas']}
    for area in room.json()['areas']:
        if kinds[area['name']] == 'bedroom_cluster':
            for unit in area['units']:
                x, y, z = unit['origin']; w, h, _ = unit['size']
                walls.update((xx, yy, z) for yy in range(y - 1, y + h + 1)
                             for xx in range(x - 1, x + w + 1)
                             if xx == x - 1 or xx == x + w or yy == y - 1 or yy == y + h)
                walls.discard(tuple(unit['doorway']))
    folder = intent['world_folder'].encode()
    header = b'DFMM1500' + struct.pack('>IIBIH', 0, tick, 1, intent['site'], len(folder)) + folder
    header += struct.pack('>10I', 64, 64, 8, *origin, *size, size[0]*size[1]*size[2])
    out = bytearray(header)
    changes = {} if changes is None else changes
    for z in range(origin[2], origin[2] + size[2]):
        for y in range(origin[1], origin[1] + size[1]):
            for x in range(origin[0], origin[0] + size[0]):
                point = x, y, z
                v = {'presence': 2, 'shape': 2 if point in walls else 3,
                     'depth': 0, 'magma': 0, 'dig': 0, 'building': 0, 'unit': 0}
                v.update(changes.get(point, {}))
                out.append(v['presence'])
                if v['presence'] == 2:
                    out += struct.pack('>IBBBBBBBIHH', 1, v['shape'], v['depth'], v['magma'],
                        0, v['dig'], v['building'], v['unit'], 1, 10015, 10015)
    return bytes(out)


def source(raw):
    terrain = routes.decode_terrain(raw)
    return reviews.Source('127.0.0.1:5000', 'df', 'dfhack', 2, 3, terrain.digest,
        hashlib.sha256(b'fixture complete inventory').hexdigest(), terrain.tick,
        terrain.dimensions, terrain.origin, terrain.size, (1000, 1000, 1000))


def fixture(room=None, *, raw=None, request=None):
    room = room_plan() if room is None else room
    raw = raw_map(room) if raw is None else raw
    request = room.request() if request is None else request
    items = tuple(Candidate(100 + i, slot.kind, slot.target, (0, -1), -1)
                  for i, slot in enumerate(request.slots))
    types = {item.id: ('bed', 'chair', 'table').index(item.kind) for item in items}
    allocation = routes.allocate_routes(request, items, raw)
    review = reviews.create(request, allocation, source(raw), types)
    return room, review, items, types, raw


def checked(f, **kwargs):
    room, review, items, types, raw = f
    handoff = model.RoomRouteHandoff.create(room, review)
    return model.revalidate(handoff, items, types, raw, source(raw), **kwargs)


class RoomRouteTests(unittest.TestCase):
    def test_roundtrip_keeps_exact_complete_parent_and_child(self):
        room, review, *_ = fixture()
        h = model.RoomRouteHandoff.create(room, review)
        restored = model.RoomRouteHandoff.decode(h.encode(), h.digest)
        a, b = restored.unpack()
        self.assertEqual(a.encode(), room.encode())
        self.assertEqual(b.raw, review.raw)
        self.assertEqual(a.request().json(), b.unpack()[1].json())
        copy = h.json(); copy['room_plan']['furniture_count'] = 0
        self.assertEqual(h, restored)
        self.assertEqual(h.json()['room_plan']['furniture_count'], 3)

    def test_complete_terrain_items_and_sequence_are_all_required(self):
        result = checked(fixture())
        self.assertEqual(result['status'], 'ready_in_captured_model')
        self.assertEqual(result['room_terrain']['evaluated_tiles'], 30)
        self.assertEqual(result['room_terrain']['counts']['required_walls']['matched'], 15)
        self.assertEqual(result['room_terrain']['counts']['floors']['matched'], 15)
        self.assertEqual(result['delivery_suffix']['status'], 'ready')
        self.assertIsNotNone(result['plan'])
        for key in ('room_completion_proven', 'placement_authorized', 'placement_gate_enforced',
                    'game_effect_performed', 'atomic_route_placement_proven'):
            self.assertFalse(result[key])

    def test_wall_or_corridor_failure_blocks_despite_all_item_routes(self):
        f = fixture(); room, review, items, types, raw = f
        h = model.RoomRouteHandoff.create(room, review)
        for point, replacement in [((4, 4, 2), {'shape': 3}), ((3, 9, 2), {'shape': 2}),
                                   ((4, 4, 2), {'dig': 1}), ((4, 4, 2), {'building': 1}),
                                   ((4, 4, 2), {'depth': 1}), ((4, 4, 2), {'magma': 1})]:
            with self.subTest(point=point, replacement=replacement):
                changed = raw_map(room, changes={point: replacement})
                result = model.revalidate(h, items, types, changed, source(changed))
                self.assertEqual(result['route_review']['status'], 'constraints_met')
                self.assertEqual(result['status'], 'blocked_in_captured_model')
                self.assertIsNone(result['plan'])
                self.assertEqual(result['room_terrain']['deficits']['count'], 1)

    def test_unknown_required_wall_never_becomes_satisfaction(self):
        room, review, items, types, _ = fixture()
        h = model.RoomRouteHandoff.create(room, review)
        for presence in (0, 1):
            raw = raw_map(room, changes={(4, 4, 2): {'presence': presence}})
            result = model.revalidate(h, items, types, raw, source(raw))
            self.assertEqual(result['status'], 'unestablished')
            self.assertEqual(result['room_terrain']['status'], 'unknown')
            self.assertIsNone(result['plan'])

    def test_known_contradiction_is_not_hidden_by_other_unknown_tiles(self):
        room, review, items, types, _ = fixture()
        h = model.RoomRouteHandoff.create(room, review)
        raw = raw_map(room, changes={(4, 4, 2): {'presence': 1}, (5, 4, 2): {'shape': 3}})
        result = model.revalidate(h, items, types, raw, source(raw))
        self.assertEqual(result['status'], 'blocked_in_captured_model')
        self.assertEqual(result['room_terrain']['deficits']['count'], 2)

    def test_current_individual_routes_do_not_prove_original_sequence(self):
        room, review, items, types, raw = fixture()
        h = model.RoomRouteHandoff.create(room, review)
        bed = next(item for item in items if item.kind == 'bed')
        changed = tuple(replace(item, position=bed.position) if item.kind == 'table' else item for item in items)
        result = model.revalidate(h, changed, types, raw, source(raw))
        self.assertEqual(result['route_review']['status'], 'constraints_met')
        self.assertEqual(result['room_terrain']['status'], 'matched')
        self.assertEqual(result['delivery_suffix']['status'], 'blocked')
        self.assertEqual(result['status'], 'blocked_in_captured_model')
        self.assertIsNone(result['plan'])
        self.assertEqual(result['delivery_suffix']['routes'], [])

    def test_missing_original_item_is_unknown_and_no_replacement_is_selected(self):
        room, review, items, types, raw = fixture()
        h = model.RoomRouteHandoff.create(room, review)
        replacement = replace(items[0], id=900)
        supplied = (replacement, *items[1:]); kinds = {i.id: ('bed','chair','table').index(i.kind) for i in supplied}
        result = model.revalidate(h, supplied, kinds, raw, source(raw))
        self.assertEqual(result['status'], 'unestablished')
        self.assertIsNone(result['plan']); self.assertIsNone(result['delivery_suffix'])
        self.assertFalse(result['reallocation_performed'])
        self.assertEqual(h.unpack()[1].raw, review.raw)

    def test_changed_original_constraints_or_dependencies_refuse_binding(self):
        room = room_plan(dining=True, count=2)
        req = room.request()
        for change in range(5):
            slots = list(req.slots)
            if change == 0:
                slots[0] = replace(slots[0], max_distance=100)
            elif change == 1:
                slots[2] = replace(slots[2], after=(slots[0].name,))
            elif change == 2:
                slots = slots[:2]
            elif change == 3:
                slots[0] = replace(slots[0], target=(20, 20, 2))
            other = Request(req.folder if change != 4 else 'other', req.site, tuple(slots), req.excluded_items)
            raw = raw_map(room)
            if change == 4:
                altered = room.json()['intent']; altered['world_folder'] = 'other'
                raw = raw_map(RoomPlan.compile(altered))
            _, review, *_ = fixture(room, raw=raw, request=other)
            with self.assertRaises(ValueError):
                model.RoomRouteHandoff.create(room, review)

    def test_region_must_cover_required_walls_and_unfurnished_floors(self):
        room = room_plan()
        for origin, size in [((5, 5, 2), (3, 3, 1)), ((3, 5, 2), (6, 5, 1))]:
            f = fixture(room, raw=raw_map(room, origin=origin, size=size))
            with self.assertRaises(ValueError):
                model.RoomRouteHandoff.create(f[0], f[1])
        self.assertEqual(model.check_region(room, (3,4,2), (6,6,1)).digest, room.digest)

    def test_all_four_rotations_and_maximum_furnishing_count(self):
        for rotation in (0, 90, 180, 270):
            result = checked(fixture(room_plan(rotation=rotation)))
            self.assertEqual(result['status'], 'ready_in_captured_model')
            self.assertEqual(result['room_terrain']['evaluated_tiles'], 30)
        f = fixture(room_plan(dining=True, count=16))
        result = checked(f)
        self.assertEqual(len(result['plan']['steps']), 32)
        self.assertEqual(len(result['delivery_suffix']['routes']), 32)
        self.assertLessEqual(len(canonical(result)), model.MAX_RESULT_BYTES)

    def test_source_scope_or_digest_change_cannot_rebind_original_review(self):
        room, review, items, types, raw = fixture()
        h = model.RoomRouteHandoff.create(room, review)
        original = source(raw)
        for field, value in [('map_generation', 5), ('inventory_generation', 5), ('tick', 9),
                             ('map_sha256', '0'*64), ('df_version', 'other'), ('size', (31,32,1))]:
            with self.subTest(field=field), self.assertRaises(ValueError):
                model.revalidate(h, items, types, raw, replace(original, **{field:value}))

    def test_every_prefix_and_single_byte_mutation_rejected_with_original_seal(self):
        room, review, *_ = fixture()
        h = model.RoomRouteHandoff.create(room, review)
        for end in range(len(h.raw)):
            with self.assertRaises(ValueError):
                model.RoomRouteHandoff.decode(h.raw[:end], h.digest)
        for offset in range(len(h.raw)):
            changed = bytearray(h.raw); changed[offset] ^= 1
            with self.assertRaises(ValueError):
                model.RoomRouteHandoff.decode(bytes(changed), h.digest)

    def test_rehashed_malformed_or_false_derived_claims_refused(self):
        room, review, *_ = fixture()
        h = model.RoomRouteHandoff.create(room, review)
        for change in range(6):
            value = h.json()
            if change == 0: value['room_plan']['terrain_observed'] = True
            elif change == 1: value['room_plan']['areas'][0]['units'][0]['size'][0] = 5
            elif change == 2: value['route_review']['selections'].pop()
            elif change == 3: value['route_review_digest'] = '0'*64
            elif change == 4: value['extra'] = None
            else: value['policy'] = 'future/2'
            raw = canonical(value)
            with self.assertRaises(ValueError):
                model.RoomRouteHandoff.decode(raw, model.object_digest(raw))
        for raw in [h.raw+b'\n', b'['*15+b']'*15, b'{"schema":1,"schema":2}', b'{"schema":NaN}']:
            with self.assertRaises(ValueError):
                model.RoomRouteHandoff.decode(raw, model.object_digest(raw))

    def test_budget_boundary_and_callback_abort_publish_nothing(self):
        f = fixture()
        result = checked(f)
        self.assertEqual(checked(f, maximum_work=result['work_units'])['status'], result['status'])
        with self.assertRaises(ValueError): checked(f, maximum_work=result['work_units']-1)
        calls = 0
        def count():
            nonlocal calls
            calls += 1
        checked(f, guard=count)
        for stop in sorted(set([1,2,calls,calls-1,*range(1,calls+1, max(1,calls//25))])):
            seen = 0
            def abort():
                nonlocal seen
                seen += 1
                if seen == stop: raise InterruptedError('cancelled')
            with self.assertRaises(InterruptedError): checked(f, guard=abort)

    def test_deficit_presentation_bound_does_not_limit_evaluation(self):
        room = room_plan(dining=True, count=16)
        room, review, items, types, _ = fixture(room)
        h = model.RoomRouteHandoff.create(room, review)
        changes = {(x,y,2): {'depth':1} for y in range(5,14) for x in range(5,18)}
        raw = raw_map(room, changes=changes)
        result = model.revalidate(h, items, types, raw, source(raw))
        deficits = result['room_terrain']['deficits']
        self.assertEqual(deficits['count'], 117)
        self.assertEqual(len(deficits['rows']), 64)
        self.assertEqual(deficits['omitted'], 53)
        self.assertIsNone(result['plan'])


if __name__ == '__main__':
    unittest.main()
