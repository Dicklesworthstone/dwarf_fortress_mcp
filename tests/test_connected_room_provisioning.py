"""Actual room compiler/allocation integration, including legacy byte witnesses."""
import copy
import hashlib
import itertools
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest
from unittest import mock

import room_provisioning as rooms
from furniture_allocation import Candidate, allocate
from furniture_plan import canonical
from test_room_connections import cells, reachable


def area(name, origin, template):
    return {'name': name, 'origin': list(origin), 'template': template}


def bedroom(count=2, size=(3, 3), origin=(10, 10, 2), name='sleep'):
    return area(name, origin, {'kind': 'bedroom_cluster', 'rooms_count': count, 'room_size': list(size)})


def dining(count=3, columns=3, origin=(10, 20, 2), name='eat'):
    return area(name, origin, {'kind': 'dining_hall', 'table_count': count, 'columns': columns})


def request(*areas, connected=True):
    out = {'schema': rooms.INTENT_SCHEMA, 'world_folder': 'region1', 'site': 2,
           'areas': list(areas) or [bedroom(), dining()]}
    if connected:
        out['connect_areas'] = True
    return out


def mask(plan):
    parts = [(tuple(p['region']['origin']), tuple(p['region']['size']))
             for p in plan.json()['excavation_blueprint']['parts']]
    expanded = cells(parts)
    assert len(expanded) == len(set(expanded))
    return set(expanded)


def required_walls(plan):
    result = set()
    kinds = {a['name']: a['template']['kind'] for a in plan.json()['intent']['areas']}
    for a in plan.json()['areas']:
        if kinds[a['name']] != 'bedroom_cluster':
            continue
        for u in a['units']:
            x, y, z = u['origin']
            w, h, _ = u['size']
            ring = {(xx, yy, z) for xx in range(x-1, x+w+1) for yy in (y-1, y+h)}
            ring |= {(xx, yy, z) for xx in (x-1, x+w) for yy in range(y-1, y+h+1)}
            result |= ring - {tuple(u['doorway'])}
    return result


class ConnectedRoomsTests(unittest.TestCase):
    def check_project(self, plan):
        floors = mask(plan)
        targets = {s.target for s in plan.request().slots}
        walkable = floors - targets
        visited = reachable(walkable, tuple(plan.json()['areas'][0]['entry_point']))
        self.assertEqual(visited, walkable)
        self.assertFalse(floors & required_walls(plan))
        for a in plan.json()['areas']:
            self.assertIn(tuple(a['entry_point']), visited)
            for approach in a['approaches']:
                self.assertIn(tuple(approach['approach']), visited)
        self.assertEqual(plan.json()['project_connections']['connected_walkable_tiles'], len(walkable))
        self.assertEqual(plan.json()['target_tiles'], len(floors))
        self.assertEqual(rooms.RoomPlan.decode(plan.encode()), plan)

    def test_checked_in_example_and_contract_match_the_executable_compiler(self):
        root = Path(__file__).resolve().parents[1]
        contract = json.loads((root / 'architecture/room_connections_v1.json').read_text())
        example = (root / contract['example']).read_bytes()
        plan = rooms.RoomPlan.from_request(example)
        self.check_project(plan)
        self.assertEqual(contract['policy'], plan.json()['project_connections']['policy'])
        self.assertEqual(contract['limits']['capture_cells'], rooms.MAX_CAPTURE_VOLUME)
        self.assertEqual(contract['limits']['whole_plan_floor_tiles_including_furniture'], rooms.MAX_TILES)
        self.assertEqual(contract['limits']['whole_plan_excavation_parts'], rooms.MAX_PARTS)
        self.assertEqual(plan.json()['furniture_count'], 12)
        self.assertEqual(plan.json()['target_tiles'], 64)

    def test_mixed_project_adds_corridors_without_changing_any_furnishing(self):
        original = rooms.RoomPlan.compile(request(connected=False))
        connected = rooms.RoomPlan.compile(request())
        self.check_project(connected)
        self.assertEqual(original.request(), connected.request())
        self.assertEqual(original.json()['areas'], connected.json()['areas'])
        self.assertEqual(len(mask(connected) - mask(original)), 5)
        self.assertTrue(mask(original) < mask(connected))
        self.assertEqual(connected.json()['project_connections']['areas'], ['eat', 'sleep'])
        self.assertNotEqual(original.digest, connected.digest)
        self.assertNotEqual(original.json()['excavation_mask_digest'], connected.json()['excavation_mask_digest'])

    def test_excluded_barrier_is_detoured_but_complete_barrier_refuses(self):
        doc = request()
        doc['excluded_regions'] = [{'origin': [9, 17, 2], 'size': [11, 1, 1]}]
        plan = rooms.RoomPlan.compile(doc)
        self.check_project(plan)
        self.assertIn((8, 17, 2), mask(plan))
        self.assertFalse(mask(plan) & {(x, 17, 2) for x in range(9, 20)})
        doc['excluded_regions'][0] = {'origin': [8, 17, 2], 'size': [12, 1, 1]}
        before = copy.deepcopy(doc)
        with self.assertRaisesRegex(ValueError, 'no corridor'):
            rooms.RoomPlan.compile(doc)
        self.assertEqual(doc, before)

    def test_connection_from_north_preserves_every_separating_bedroom_wall(self):
        plan = rooms.RoomPlan.compile(request(bedroom(), dining(origin=(10, 5, 2))))
        self.check_project(plan)
        self.assertFalse(mask(plan) & {(x, 9, 2) for x in range(9, 18)})
        self.assertGreater(plan.json()['project_connections']['new_corridor_tiles'], 0)

    def test_large_other_level_exclusion_does_not_expand_or_block_current_level(self):
        doc = request()
        doc['excluded_regions'] = [{'origin': [0, 0, 3], 'size': [32768, 32768, 32765]}]
        self.check_project(rooms.RoomPlan.compile(doc))

    def test_connections_never_invent_stairs_between_levels(self):
        doc = request(bedroom(), dining(origin=(10, 20, 3)))
        with self.assertRaisesRegex(ValueError, 'one level'):
            rooms.RoomPlan.compile(doc)
        doc.pop('connect_areas')
        self.assertEqual(len(rooms.RoomPlan.compile(doc).json()['areas']), 2)

    def test_area_and_exclusion_order_preserve_complete_canonical_bytes(self):
        left = request()
        left['excluded_regions'] = [{'origin': [9, 17, 2], 'size': [11, 1, 1]},
                                    {'origin': [30, 30, 2], 'size': [1, 1, 1]}]
        right = copy.deepcopy(left)
        right['areas'].reverse()
        right['excluded_regions'].reverse()
        self.assertEqual(rooms.RoomPlan.compile(left).encode(), rooms.RoomPlan.compile(right).encode())

    def test_regeneration_rejects_dropped_corridor_and_forged_connectivity(self):
        plan = rooms.RoomPlan.compile(request())
        variants = []
        v = plan.json(); v['excavation_blueprint']['parts'].pop(); variants.append(v)
        v = plan.json(); v['project_connections']['connection_digest'] = '0'*64; variants.append(v)
        v = plan.json(); v['project_connections']['new_corridor_tiles'] = 0; variants.append(v)
        v = plan.json(); v['project_connections']['all_areas_connected'] = False; variants.append(v)
        v = plan.json(); v['intent'].pop('connect_areas'); variants.append(v)
        v = plan.json(); v['intent']['areas'].pop(); variants.append(v)
        for value in variants:
            with self.assertRaises(ValueError):
                rooms.RoomPlan.decode(canonical(value))

    def test_connectivity_and_corridors_are_never_native_success_or_authority(self):
        plan = rooms.RoomPlan.from_request(canonical(request()))
        for name in ('terrain_observed', 'native_pathfinding_proven', 'existing_fort_access_proven',
                     'wall_preservation_observed', 'room_completion_proven',
                     'mutation_authority_granted', 'prepared_plan_created'):
            self.assertIs(plan.json()[name], False)
        self.assertFalse(plan.json()['project_connections']['global_minimum_excavation_proven'])
        self.assertEqual(plan.summary()['project_connections'], plan.json()['project_connections'])

    def test_final_corridor_parts_and_tiles_share_existing_whole_plan_limits(self):
        original = rooms.RoomPlan.compile(request(connected=False)).json()
        for name, value in [('MAX_PARTS', len(original['excavation_blueprint']['parts'])),
                            ('MAX_TILES', original['target_tiles'])]:
            with mock.patch.object(rooms, name, value), self.assertRaises(ValueError):
                rooms.RoomPlan.compile(request())

    def test_exact_boolean_and_duplicate_fields_refuse(self):
        for value in (None, 1, 0, 'true', [], {}):
            doc = request(); doc['connect_areas'] = value
            with self.assertRaises(ValueError):
                rooms.RoomPlan.compile(doc)
        raw = canonical(request()).replace(b'"connect_areas":true', b'"connect_areas":true,"connect_areas":false')
        with self.assertRaises(ValueError):
            rooms.RoomPlan.from_request(raw)

    def test_all_eight_areas_and_all_thirty_two_slots_are_preserved(self):
        many = [dining(1, 1, (3 + i % 4 * 6, 3 + i // 4 * 6, 2), f'area{i}') for i in range(8)]
        plan = rooms.RoomPlan.compile(request(*many))
        self.check_project(plan)
        self.assertEqual(len(plan.json()['areas']), 8)
        maximum = rooms.RoomPlan.compile(request(bedroom(8), dining(4, 4, (10, 24, 2))))
        self.check_project(maximum)
        self.assertEqual(len(maximum.request().slots), 32)
        self.assertLessEqual(len(maximum.json()['excavation_blueprint']['parts']), 32)

    def test_global_allocator_retains_constraints_and_complete_shortage(self):
        doc = request()
        doc['areas'][0]['item_constraints'] = {'chair': {'material': [7, -1]}}
        plan = rooms.RoomPlan.compile(doc)
        candidates = tuple(Candidate(i, s.kind, s.target, s.material or (9, -1), -1)
                           for i, s in enumerate(plan.request().slots))
        complete = allocate(plan.request(), candidates)
        self.assertEqual(complete['status'], 'allocated')
        self.assertEqual(len(complete['plan']['steps']), 12)
        shortage = allocate(plan.request(), ())
        self.assertEqual(shortage['status'], 'shortage')
        self.assertIsNone(shortage['plan'])
        self.assertEqual(shortage['assignments'], [])

    def test_repeatable_across_process_hash_seeds(self):
        code = ('import sys;from room_provisioning import RoomPlan;'
                'sys.stdout.buffer.write(RoomPlan.from_request(sys.stdin.buffer.read()).encode())')
        outputs = []
        for seed in ('1', '97', 'random'):
            env = {**os.environ, 'PYTHONHASHSEED': seed,
                   'PYTHONPATH': str(Path(__file__).resolve().parents[1] / 'scripts')}
            proc = subprocess.run([sys.executable, '-c', code], input=canonical(request()),
                                  capture_output=True, env=env, timeout=10, check=True)
            self.assertEqual(proc.stderr, b'')
            outputs.append(proc.stdout)
        self.assertEqual(outputs, [rooms.RoomPlan.compile(request()).encode()]*3)

    def test_interruption_never_returns_an_unconnected_or_partial_room_plan(self):
        calls = 0
        def count():
            nonlocal calls
            calls += 1
        doc = request()
        rooms.RoomPlan.compile(doc, count)
        for fail_at in range(1, calls+1):
            left = fail_at
            def guard():
                nonlocal left
                left -= 1
                if not left:
                    raise InterruptedError('revoked')
            with self.assertRaises(InterruptedError):
                rooms.RoomPlan.compile(doc, guard)
        self.assertEqual(doc, request())

    def test_314_legacy_requests_retain_exact_prechange_bytes_and_refusals(self):
        # Generated by the unmodified, Git-blob-verified 1a486bd993bc19c6d6f19d9999eac5d1682ac1b9.
        # Hash covers SHA-256 of every full successful artifact, and None for each refusal.
        outcomes = {}
        templates = [(f'bedrooms:{n}:{w}:{h}', {'kind': 'bedroom_cluster', 'rooms_count': n, 'room_size': [w, h]})
                     for n, w, h in itertools.product(range(1, 11), range(3, 8), range(3, 8))]
        templates += [(f'dining:{n}:{c}', {'kind': 'dining_hall', 'table_count': n, 'columns': c})
                      for n, c in itertools.product(range(1, 17), range(1, 5))]
        for key, template in templates:
            doc = request(area('area', (10, 10, 2), template), connected=False)
            try:
                original = rooms.RoomPlan.compile(doc).encode()
                outcomes[key] = hashlib.sha256(original).hexdigest()
                doc['connect_areas'] = False
                self.assertEqual(rooms.RoomPlan.compile(doc).encode(), original)
            except ValueError:
                outcomes[key] = None
        self.assertEqual(len(outcomes), 314)
        self.assertEqual(sum(v is not None for v in outcomes.values()), 310)
        self.assertEqual(hashlib.sha256(canonical(outcomes)).hexdigest(), '9714b6a05aa0a84145dd5de70cad05947a42a92af80d8916c147a54ac5a71432')


if __name__ == '__main__':
    unittest.main()
