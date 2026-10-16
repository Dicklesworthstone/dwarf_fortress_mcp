"""Whole-project access corridors through the real compiler and item allocator.

Beads: dfmcp-wp-pln01-spatial-blueprint-layout-pwd; df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

from copy import deepcopy
import hashlib
import itertools
import os
from pathlib import Path
import subprocess
import sys
import unittest

SCRIPTS = Path(__file__).resolve().parents[1] / 'scripts'
sys.path.insert(0, str(SCRIPTS))
from furniture_allocation import Candidate, allocate
from furniture_plan import canonical
from room_access import Access
from room_connections import connect
from room_provisioning import RoomPlan


def dining(name='hall', origin=(16, 10, 2), **extra):
    return {'name': name, 'origin': list(origin),
            'template': {'kind': 'dining_hall', 'table_count': 1}, **extra}


def request(*areas, point=(10, 10, 2), size=(10, 6, 1)):
    return {'schema': 'dfmcp.room-provisioning-request/1', 'world_folder': 'region1', 'site': 2,
            'connect_areas': True, 'areas': list(areas) or [dining()],
            'access': {'point': list(point), 'region': {'origin': [10, 10, 2], 'size': list(size)}}}


def mask(plan):
    cells = set()
    for part in plan.json()['excavation_blueprint']['parts']:
        low, size = part['region']['origin'], part['region']['size']
        selected = set(itertools.product(*(range(a, a + n) for a, n in zip(low, size))))
        if selected & cells:
            raise AssertionError('output rectangles overlap')
        cells.update(selected)
    return cells


def flood(cells, start):
    seen, pending = {start}, [start]
    for x, y, z in pending:
        for point in ((x-1, y, z), (x+1, y, z), (x, y-1, z), (x, y+1, z)):
            if point in cells and point not in seen:
                seen.add(point)
                pending.append(point)
    return seen


class AccessTests(unittest.TestCase):
    def check(self, value):
        original = deepcopy(value)
        plan = RoomPlan.compile(value)
        self.assertEqual(value, original)
        self.assertEqual(RoomPlan.decode(plan.encode()), plan)
        result, cells = plan.json(), mask(plan)
        root = tuple(value['access']['point'])
        walkable = cells - {s.target for s in plan.request().slots}
        self.assertIn(root, walkable)
        self.assertEqual(flood(walkable, root), walkable)
        self.assertTrue(all(tuple(a['entry_point']) in walkable for a in result['areas']))
        access = Access.from_json(value['access'])
        self.assertTrue(all(access.contains(p) for p in cells))
        self.assertLessEqual(len(cells), 512)
        self.assertLessEqual(len(result['excavation_blueprint']['parts']), 32)
        # Independently reconstruct the semantic mask identity and tight hull.
        low = tuple(min(p[i] for p in cells) for i in range(3))
        size = tuple(max(p[i] for p in cells) - low[i] + 1 for i in range(3))
        self.assertEqual(result['capture_region'], {'origin': list(low), 'size': list(size)})
        indices = [(((z-low[2])*size[1]+y-low[1])*size[0]+x-low[0], 3)
                   for x, y, z in sorted(cells, key=lambda p: p[::-1])]
        identity = hashlib.sha256(b'dfmcp-excavation-blueprint/1\0' + canonical(
            {'region': result['capture_region'], 'targets': indices})).hexdigest()
        self.assertEqual(identity, result['excavation_mask_digest'])
        self.assertEqual(len(cells), result['target_tiles'])
        self.assertEqual(result['intent']['access'], value['access'])
        self.assertIs(result['existing_fort_access_proven'], False)
        self.assertIs(result['mutation_authority_granted'], False)
        self.assertTrue(result['project_connections']['access']['all_areas_connected_to_point'])
        return plan

    def test_connect_external_point_and_retain_it_in_the_original_goal(self):
        plan = self.check(request())
        result = plan.json()
        self.assertEqual(mask(plan), {(x, 10, 2) for x in range(10, 16)} |
                         {(x, y, 2) for x in range(16, 20) for y in range(10, 13)})
        self.assertEqual(result['project_connections']['new_corridor_tiles'], 5)
        self.assertIsNone(result['project_connections']['root_area'])
        self.assertFalse(result['project_connections']['access']['point_floor_observed'])

    def test_detour_can_leave_the_room_hull_but_not_the_explicit_domain(self):
        value = request()
        value['excluded_regions'] = [{'origin': [14, 10, 2], 'size': [1, 4, 1]}]
        cells = mask(self.check(value))
        self.assertTrue(any(y == 14 for _, y, _ in cells))
        self.assertFalse(cells & {(14, y, 2) for y in range(10, 14)})
        self.assertTrue(all(10 <= x < 20 and 10 <= y < 16 for x, y, _ in cells))
        # Closing the only in-domain detour cannot manufacture an outside path.
        value['excluded_regions'][0]['size'][1] = 6
        with self.assertRaisesRegex(ValueError, 'no corridor'):
            RoomPlan.compile(value)

    def test_root_already_in_room_floor_is_not_duplicated(self):
        value = request(point=(16, 10, 2))
        plan = self.check(value)
        old = deepcopy(value); old.pop('access')
        self.assertEqual(mask(plan), mask(RoomPlan.compile(old)))
        self.assertEqual(plan.json()['project_connections']['new_corridor_tiles'], 0)

    def test_access_point_cannot_be_furniture_or_excluded(self):
        with self.assertRaisesRegex(ValueError, 'furniture obstructs'):
            RoomPlan.compile(request(point=(17, 11, 2)))
        value = request()
        value['excluded_regions'] = [{'origin': [10, 10, 2], 'size': [1, 1, 1]}]
        with self.assertRaisesRegex(ValueError, 'protected'):
            RoomPlan.compile(value)

    def test_rotated_bedroom_walls_and_doors_are_preserved(self):
        for angle in (0, 90, 180, 270):
            area = {'name': 'sleep', 'origin': [22, 22, 2], 'rotation': angle,
                    'template': {'kind': 'bedroom_cluster', 'rooms_count': 1, 'room_size': [3, 5]}}
            value = request(area, size=(24, 24, 1))
            plan = self.check(value)
            unit = plan.json()['areas'][0]['units'][0]
            x, y, z = unit['origin']; w, h, _ = unit['size']
            wall = {(xx, yy, z) for xx in range(x-1, x+w+1) for yy in range(y-1, y+h+1)
                    if xx in (x-1, x+w) or yy in (y-1, y+h)} - {tuple(unit['doorway'])}
            self.assertFalse(mask(plan) & wall)
            value['access']['point'] = list(min(wall))
            with self.subTest(angle=angle), self.assertRaisesRegex(ValueError, 'protected'):
                RoomPlan.compile(value)

    def test_eight_areas_plus_access_are_not_reduced_to_seven_areas(self):
        areas = [dining(str(i), (16+(i%4)*6, 12+(i//4)*7, 2)) for i in range(8)]
        plan = self.check(request(*areas, size=(32, 16, 1)))
        self.assertEqual(len(plan.json()['areas']), 8)
        self.assertEqual(len(plan.request().slots), 16)

    def test_site_mask_applies_to_corridors_and_access(self):
        value = request()
        cells = ['.'] * 60
        for y in range(4): cells[y*10+4] = 'W'
        value['site_mask'] = {'region': deepcopy(value['access']['region']), 'cells': ''.join(cells)}
        self.assertTrue(any(y == 14 for _, y, _ in mask(self.check(value))))
        cells[0] = 'X'
        value['site_mask']['cells'] = ''.join(cells)
        with self.assertRaisesRegex(ValueError, 'protected'):
            RoomPlan.compile(value)

    def test_routing_region_never_relaxes_the_original_site_mask(self):
        value = request()
        value['site_mask'] = {'region': {'origin': [16, 10, 2], 'size': [4, 3, 1]}, 'cells': '.'*12}
        with self.assertRaisesRegex(ValueError, 'protected'):
            RoomPlan.compile(value)

    def test_access_requires_complete_connection_and_in_domain_rooms(self):
        value = request(); value['connect_areas'] = False
        with self.assertRaisesRegex(ValueError, 'complete project'):
            RoomPlan.compile(value)
        value = request(size=(9, 6, 1))
        with self.assertRaisesRegex(ValueError, 'outside access routing'):
            RoomPlan.compile(value)

    def test_closed_types_bounds_and_same_level(self):
        for point in ([True, 10, 2], [10.0, 10, 2], [0, 10, 2], [10, 10, 3], [20, 10, 2], None, [1, 2]):
            value = request(); value['access']['point'] = point
            with self.subTest(point=point), self.assertRaises(ValueError):
                RoomPlan.compile(value)
        for size in ([10, 6, 2], [33, 32, 1], [129, 1, 1], [10, False, 1], [10, 0, 1]):
            with self.subTest(size=size), self.assertRaises(ValueError):
                RoomPlan.compile(request(size=size))
        for access in (None, {}, [], {**request()['access'], 'execute': True}):
            value = request(); value['access'] = access
            with self.subTest(access=access), self.assertRaises(ValueError):
                RoomPlan.compile(value)

    def test_router_obeys_corridor_and_total_tile_allowances(self):
        with self.assertRaisesRegex(ValueError, 'tile allowance'):
            connect({(10, 10, 2), (19, 10, 2)}, set(), (('a', (19, 10, 2)),),
                    ((10, 10, 2), (10, 1, 1)), max_new_tiles=7, root=(10, 10, 2))
        full = {(x, y, 2) for x in range(10, 42) for y in range(10, 26)}
        with self.assertRaisesRegex(ValueError, 'tile allowance'):
            connect(full, set(), (('a', (10, 10, 2)),),
                    ((10, 10, 2), (32, 32, 1)), max_new_tiles=1, root=(10, 10, 2))

    def test_whole_item_allocation_keeps_the_access_corridor_clear(self):
        plan = self.check(request())
        wanted = plan.request()
        candidates = tuple(Candidate(i, slot.kind, slot.target, (0, 1), -1)
                           for i, slot in enumerate(wanted.slots))
        result = allocate(wanted, candidates)
        self.assertEqual(result['status'], 'allocated')
        self.assertEqual(len(result['plan']['steps']), 2)
        self.assertNotIn([10, 10, 2], [step['target'] for step in result['plan']['steps']])
        self.assertEqual(allocate(wanted, candidates[:1])['status'], 'shortage')

    def test_replay_rejects_changed_root_domain_or_corridor(self):
        plan = self.check(request())
        for field in ('root', 'domain', 'part', 'claim'):
            value = plan.json()
            if field == 'root': value['intent']['access']['point'][0] += 1
            elif field == 'domain': value['intent']['access']['region']['size'][1] += 1
            elif field == 'part': value['excavation_blueprint']['parts'].pop()
            else: value['project_connections']['access']['point_floor_observed'] = True
            with self.subTest(field=field), self.assertRaises(ValueError):
                RoomPlan.decode(canonical(value))

    def test_cancellation_at_every_guard_has_no_partial_result(self):
        value = request(); calls = []
        RoomPlan.compile(value, lambda: calls.append(1))
        for stop in range(1, len(calls)+1):
            seen = 0
            def guard():
                nonlocal seen
                seen += 1
                if seen == stop: raise InterruptedError('cancelled access')
            with self.subTest(stop=stop), self.assertRaisesRegex(InterruptedError, 'cancelled access'):
                RoomPlan.compile(value, guard)
            self.assertEqual(seen, stop)

    def test_root_participates_in_witness_even_when_connected(self):
        floors = {(x, 10, 2) for x in range(10, 14)}
        kwargs = dict(floors=floors, blocked=set(), entries=(('a', (10, 10, 2)),),
                      region=((10, 10, 2), (4, 1, 1)), max_new_tiles=0)
        a = connect(**kwargs, root=(10, 10, 2)); b = connect(**kwargs, root=(13, 10, 2))
        self.assertEqual(a.added, b.added)
        self.assertNotEqual(a.digest, b.digest)
        for root in ((14, 10, 2), (True, 10, 2), [10, 10, 2]):
            with self.assertRaises(ValueError): connect(**kwargs, root=root)

    def test_hash_seed_replay(self):
        raw = canonical(request())
        expected = RoomPlan.from_request(raw).encode()
        code = 'import sys,room_provisioning as r;sys.stdout.buffer.write(r.RoomPlan.from_request(sys.stdin.buffer.read()).encode())'
        for seed in ('1', '17', 'random'):
            run = subprocess.run([sys.executable, '-c', code], input=raw, capture_output=True,
                env={**os.environ, 'PYTHONPATH': str(SCRIPTS), 'PYTHONHASHSEED': seed}, timeout=10, check=True)
            self.assertEqual(run.stdout, expected)


if __name__ == '__main__':
    unittest.main()
