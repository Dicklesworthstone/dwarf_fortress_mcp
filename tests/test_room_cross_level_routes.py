"""Whole-room requests enter the real observed stair-route allocator unchanged."""
from copy import deepcopy
from pathlib import Path
import struct
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
from furniture_allocation import Candidate, Request, allocate
from furniture_plan import canonical
from room_provisioning import INTENT_SCHEMA, RoomPlan
from route_furniture_allocation import allocate_routes


def request(kind='dining_hall', count=2, vertical=None, angle=0):
    template = ({'kind': kind, 'rooms_count': count, 'room_size': [3, 3]}
                if kind == 'bedroom_cluster' else {'kind': kind, 'table_count': count, 'columns': 4})
    kinds = ('bed', 'chair', 'table') if kind == 'bedroom_cluster' else ('chair', 'table')
    constraints = {} if vertical is None else {k: {'max_vertical_distance': vertical} for k in kinds}
    return {'schema': INTENT_SCHEMA, 'world_folder': 'room-route-fixture', 'site': 7,
            'areas': [{'name': 'rooms', 'origin': [100, 100, 2], 'rotation': angle,
                       'template': template, 'item_constraints': constraints}]}


def map_bytes(plan, *, stairs=True, hidden=False, blocked=False, upper=7, lower=6):
    body = plan.json()
    region = body['capture_region']
    x, y, _ = region['origin']
    w, h, _ = region['size']
    origin, size, dimensions = (x - 1, y - 1, 1), (w + 2, h + 2, 2), (256, 256, 8)
    folder = plan.request().folder.encode()
    raw = bytearray(b'DFMM1500' + struct.pack('>IIBIH', 0, 100, 1, 7, len(folder)) + folder)
    raw += struct.pack('>10I', *dimensions, *origin, *size, size[0] * size[1] * size[2])
    for z in range(origin[2], origin[2] + size[2]):
        for yy in range(origin[1], origin[1] + size[1]):
            for xx in range(origin[0], origin[0] + size[0]):
                stair = (xx, yy) == origin[:2]
                if stair and hidden:
                    raw += b'\1'
                else:
                    shape = (lower if z == 1 else upper) if stair and stairs else 3
                    raw += b'\2' + struct.pack('>IBBBBBBBIHH', 1, shape, 0, 0, 0, 0,
                                               int(stair and blocked), 0, 1, 10000, 10000)
    return bytes(raw), origin, size, dimensions


def candidates(plan, *, level=1):
    return tuple(Candidate(i + 10, s.kind, (*s.target[:2], level), (1, 2), -1)
                 for i, s in enumerate(plan.request().slots))


class RoomCrossLevelRoutesTests(unittest.TestCase):
    def test_zero_and_omitted_limits_preserve_legacy_bytes(self):
        for kind in ('dining_hall', 'bedroom_cluster'):
            for angle in (0, 90, 180, 270):
                a = RoomPlan.compile(request(kind, vertical=None, angle=angle))
                b = RoomPlan.compile(request(kind, vertical=0, angle=angle))
                self.assertEqual(a.encode(), b.encode())
                self.assertNotIn(b'max_vertical_distance', b.encode())

    def test_every_room_kind_and_rotation_preserves_the_original_vertical_limit(self):
        for kind in ('dining_hall', 'bedroom_cluster'):
            for angle in (0, 90, 180, 270):
                p = RoomPlan.compile(request(kind, vertical=3, angle=angle))
                self.assertTrue(all(s.max_vertical_distance == 3 for s in p.request().slots))
                self.assertEqual(RoomPlan.decode(p.encode()), p)
                self.assertEqual(Request.decode(canonical(p.json()['furniture_request'])), p.request())

    def test_complete_room_can_source_furniture_on_another_level(self):
        for kind in ('dining_hall', 'bedroom_cluster'):
            p = RoomPlan.compile(request(kind, vertical=1))
            terrain = map_bytes(p)[0]
            result = allocate_routes(p.request(), candidates(p), terrain)
            self.assertEqual(result['status'], 'allocated')
            self.assertEqual(len(result['assignments']), len(p.request().slots))
            self.assertEqual({tuple(s['target']) for s in result['plan']['steps']},
                             {s.target for s in p.request().slots})
            self.assertTrue(all(s['position'][2] == 1 for s in result['assignments']))
            self.assertFalse(result['pathfinding_proven'])

    def test_all_32_furnishings_are_retained_across_levels(self):
        p = RoomPlan.compile(request(count=16, vertical=1))
        result = allocate_routes(p.request(), candidates(p), map_bytes(p)[0])
        self.assertEqual(result['status'], 'allocated')
        self.assertEqual(len(result['plan']['steps']), 32)
        self.assertEqual(len({s['item'] for s in result['plan']['steps']}), 32)

    def test_absent_hidden_blocked_and_wrong_stairs_never_become_a_route(self):
        p = RoomPlan.compile(request(vertical=1))
        for options in ({'stairs': False}, {'hidden': True}, {'blocked': True},
                        {'lower': 7}, {'upper': 6}):
            result = allocate_routes(p.request(), candidates(p), map_bytes(p, **options)[0])
            self.assertEqual(result['status'], 'shortage', options)
            self.assertIsNone(result['plan'])
            self.assertEqual(result['assignments'], [])

    def test_same_level_default_does_not_silently_enable_stairs(self):
        p = RoomPlan.compile(request())
        result = allocate_routes(p.request(), candidates(p), map_bytes(p)[0])
        self.assertEqual(result['status'], 'shortage')
        self.assertTrue(result['same_level_endpoints_only'])

    def test_nonroute_allocator_refuses_cross_level_contract_even_for_same_level_items(self):
        p = RoomPlan.compile(request(vertical=1))
        with self.assertRaisesRegex(ValueError, 'observed route allocator'):
            allocate(p.request(), candidates(p, level=2))

    def test_limits_are_exact_bounded_integers(self):
        for value in (True, False, 1.0, '1', None, [], {}, -1, 128):
            r = request(vertical=1)
            r['areas'][0]['item_constraints']['chair']['max_vertical_distance'] = value
            with self.subTest(value=value), self.assertRaises(ValueError):
                RoomPlan.compile(r)
        p = RoomPlan.compile(request(vertical=127))
        self.assertEqual(p.request().slots[0].max_vertical_distance, 127)

    def test_vertical_limit_does_not_relax_material_subtype_or_manhattan(self):
        for selector in ({'material': [9, 9]}, {'subtype': 1}, {'max_distance': 0}):
            r = request(vertical=1)
            r['areas'][0]['item_constraints']['chair'].update(selector)
            p = RoomPlan.compile(r)
            result = allocate_routes(p.request(), candidates(p), map_bytes(p)[0])
            self.assertEqual(result['status'], 'shortage')
            self.assertIsNone(result['plan'])

    def test_per_kind_limits_do_not_spill_to_other_furnishings(self):
        r = request()
        r['areas'][0]['item_constraints']['chair'] = {'max_vertical_distance': 1}
        p = RoomPlan.compile(r)
        self.assertEqual({s.kind: s.max_vertical_distance for s in p.request().slots}, {'chair': 1, 'table': 0})
        result = allocate_routes(p.request(), candidates(p), map_bytes(p)[0])
        self.assertEqual(result['status'], 'shortage')
        self.assertIsNone(result['plan'])

    def test_route_steps_and_work_budget_are_still_required(self):
        p = RoomPlan.compile(request(vertical=1))
        raw = map_bytes(p)[0]
        result = allocate_routes(p.request(), candidates(p), raw, max_route_steps=0)
        self.assertEqual(result['status'], 'shortage')
        with self.assertRaisesRegex(ValueError, 'work allowance'):
            allocate_routes(p.request(), candidates(p), raw, maximum_work=1)

    def test_inputs_remain_immutable_and_tampered_limits_fail_regeneration(self):
        r = request(vertical=1)
        original = deepcopy(r)
        p = RoomPlan.compile(r)
        self.assertEqual(r, original)
        body = p.json()
        body['intent']['areas'][0]['item_constraints']['chair']['max_vertical_distance'] = 2
        with self.assertRaisesRegex(ValueError, 'retained intent'):
            RoomPlan.decode(canonical(body))


if __name__ == '__main__':
    unittest.main()
