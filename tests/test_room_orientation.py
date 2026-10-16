"""Whole-plan rotation, canonical compatibility and downstream allocation tests."""
from __future__ import annotations

from copy import deepcopy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
from furniture_allocation import Candidate, allocate
from furniture_plan import FurniturePlan, canonical
from room_provisioning import INTENT_SCHEMA, RoomPlan


def request(kind='bedrooms', angle=None):
    template = ({'kind': 'bedroom_cluster', 'rooms_count': 5, 'room_size': [3, 5]}
                if kind == 'bedrooms' else {'kind': 'dining_hall', 'table_count': 4, 'columns': 4})
    area = {'name': kind, 'origin': [100, 100, 2], 'template': template}
    if angle is not None:
        area['rotation'] = angle
    return {'schema': INTENT_SCHEMA, 'world_folder': 'rotation-fixture', 'site': 7, 'areas': [area]}


def cells(region):
    x, y, z = region['origin']
    w, h, levels = region['size']
    return {(xx, yy, zz) for zz in range(z, z + levels)
            for yy in range(y, y + h) for xx in range(x, x + w)}


def floor(body):
    return set().union(*(cells(p['region']) for p in body['excavation_blueprint']['parts']))


def turn(point, angle, origin=(100, 100, 2)):
    # Independent repeated single-turn oracle, not the implementation's branches.
    x, y, z = (a - b for a, b in zip(point, origin))
    for _ in range(angle // 90):
        x, y = -y, x
    return x + origin[0], y + origin[1], z + origin[2]


class RoomOrientationTests(unittest.TestCase):
    def test_legacy_digests_and_explicit_zero_are_unchanged(self):
        goldens = {'bedrooms': '051a414c9effb457811899ff88998ded84bef1bb59ae0775283b76971e77ec16',
                   'dining': 'bc4fd5d1eafec154a878baf38fe532684144efe9c3e96b9f5a2e94616ba96c42'}
        for kind, digest in goldens.items():
            old = RoomPlan.compile(request(kind))
            self.assertEqual(old.digest, digest)
            self.assertEqual(old.encode(), RoomPlan.compile(request(kind, 0)).encode())
            self.assertEqual(RoomPlan.decode(old.encode()), old)

    def test_complete_masks_units_doors_and_slots_rotate_together(self):
        for kind in ('bedrooms', 'dining'):
            original = RoomPlan.compile(request(kind)).json()
            for angle in (90, 180, 270):
                with self.subTest(kind=kind, angle=angle):
                    plan = RoomPlan.compile(request(kind, angle))
                    result = plan.json()
                    self.assertEqual(floor(result), {turn(p, angle) for p in floor(original)})
                    self.assertEqual(result['target_tiles'], original['target_tiles'])
                    self.assertEqual(result['furniture_count'], original['furniture_count'])
                    self.assertEqual(result['intent']['areas'][0]['rotation'], angle)
                    for before, after in zip(original['areas'][0]['units'], result['areas'][0]['units']):
                        self.assertEqual(cells(after), {turn(p, angle) for p in cells(before)})
                        expected = None if before['doorway'] is None else list(turn(before['doorway'], angle))
                        self.assertEqual(after['doorway'], expected)
                        self.assertEqual(after['slots'], before['slots'])
                    for before, after in zip(original['furniture_request']['slots'], result['furniture_request']['slots']):
                        self.assertEqual(after, {**before, 'target': list(turn(before['target'], angle))})
                    self.assertEqual(result['areas'][0]['entry_point'],
                                     list(turn(original['areas'][0]['entry_point'], angle)))
                    self.assertEqual(RoomPlan.decode(plan.encode()), plan)

    def test_rectangular_dining_hall_can_fit_vertical_site(self):
        value = request('dining', 90)
        value['site_mask'] = {'region': {'origin': [98, 100, 2], 'size': [3, 13, 1]}, 'cells': '.' * 39}
        result = RoomPlan.compile(value).json()
        self.assertEqual(result['capture_region'], value['site_mask']['region'])
        value['areas'][0].pop('rotation')
        with self.assertRaisesRegex(ValueError, 'site mask'):
            RoomPlan.compile(value)

    def test_valid_rotated_shape_at_edge_is_not_rejected_as_unrotated(self):
        value = request('dining', 90)
        value['areas'][0]['origin'] = [32766, 100, 2]
        result = RoomPlan.compile(value).json()
        self.assertTrue(all(1 <= p[0] <= 32766 for p in floor(result)))
        value['areas'][0].pop('rotation')
        with self.assertRaisesRegex(ValueError, 'coordinate bounds'):
            RoomPlan.compile(value)

    def test_rotated_out_of_bounds_never_returns_partial_plan(self):
        for angle, origin in ((90, [1, 100, 2]), (180, [1, 1, 2]), (270, [100, 1, 2])):
            value = request('dining', angle)
            value['areas'][0]['origin'] = origin
            with self.subTest(angle=angle), self.assertRaisesRegex(ValueError, 'coordinate bounds'):
                RoomPlan.compile(value)

    def test_rotation_is_exact_closed_enum(self):
        for angle in (True, False, None, 90.0, '90', -90, 360, 45, [], {}):
            value = request('dining')
            value['areas'][0]['rotation'] = angle
            with self.subTest(angle=angle), self.assertRaisesRegex(ValueError, 'quarter-turn'):
                RoomPlan.compile(value)

    def test_unknown_fields_remain_rejected(self):
        value = request('dining', 90)
        value['areas'][0]['mirror'] = True
        with self.assertRaisesRegex(ValueError, 'fields'):
            RoomPlan.compile(value)
        value['areas'][0].pop('mirror')
        value['areas'][0]['template']['rotation'] = 90
        with self.assertRaisesRegex(ValueError, 'fields'):
            RoomPlan.compile(value)

    def test_inputs_and_item_constraints_are_not_mutated(self):
        value = request('dining', 270)
        value['areas'][0]['item_constraints'] = {'chair': {'material': [0, 1], 'max_distance': 12}}
        before = deepcopy(value)
        body = RoomPlan.compile(value).json()
        self.assertEqual(value, before)
        chair = next(s for s in body['furniture_request']['slots'] if s['kind'] == 'chair')
        self.assertEqual(chair['material'], [0, 1])
        self.assertEqual(chair['max_distance'], 12)

    def test_rotated_bedroom_walls_cannot_be_excavated_by_another_area(self):
        for angle in (90, 180, 270):
            value = request('bedrooms', angle)
            value['areas'][0]['template']['rooms_count'] = 1
            unit = RoomPlan.compile(value).json()['areas'][0]['units'][0]
            x, y, z = unit['origin']
            # Put a dining floor against the non-door corner of this room's ring.
            value['areas'].append({'name': 'intruder', 'origin': [x - 3, y - 3, z],
                                  'template': {'kind': 'dining_hall', 'table_count': 1}})
            with self.subTest(angle=angle), self.assertRaises(ValueError):
                RoomPlan.compile(value)

    def test_site_mask_protects_rotated_wall_ring(self):
        value = request('bedrooms', 90)
        value['areas'][0]['template']['rooms_count'] = 1
        body = RoomPlan.compile(value).json()
        region = {'origin': [90, 90, 2], 'size': [20, 20, 1]}
        mask = ['.'] * 400
        unit = body['areas'][0]['units'][0]
        x, y, _ = unit['origin']
        mask[(y - 1 - 90) * 20 + x - 1 - 90] = 'F'
        value['site_mask'] = {'region': region, 'cells': ''.join(mask)}
        with self.assertRaisesRegex(ValueError, 'wall forbidden'):
            RoomPlan.compile(value)

    def test_oppositely_rotated_areas_connect_without_dropping_rooms(self):
        value = request('dining', 90)
        other = deepcopy(value['areas'][0])
        other.update(name='other', origin=[120, 100, 2], rotation=270)
        value.update(areas=[*value['areas'], other], connect_areas=True)
        plan = RoomPlan.compile(value)
        self.assertTrue(plan.json()['project_connections']['all_areas_connected'])
        self.assertEqual(plan.json()['furniture_count'], 16)
        self.assertEqual(RoomPlan.decode(plan.encode()), plan)

    def test_real_allocator_consumes_rotated_complete_request(self):
        plan = RoomPlan.compile(request('bedrooms', 90))
        desired = plan.request()
        candidates = tuple(Candidate(i, slot.kind, slot.target, (0, 1), -1)
                           for i, slot in enumerate(desired.slots))
        result = allocate(desired, candidates)
        self.assertEqual(result['status'], 'allocated')
        self.assertEqual(result['total_distance'], 0)
        actual = FurniturePlan.from_json(result['plan'])
        self.assertEqual({s.target for s in actual.steps}, {s.target for s in desired.slots})
        self.assertEqual(len(actual.steps), 15)
        self.assertFalse(result['game_effect_performed'])

    def test_rotation_and_geometry_tampering_fail_regeneration(self):
        plan = RoomPlan.compile(request('dining', 90))
        for field in ('rotation', 'target'):
            body = plan.json()
            if field == 'rotation':
                body['intent']['areas'][0]['rotation'] = 270
            else:
                body['furniture_request']['slots'][0]['target'][0] += 1
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'retained intent'):
                RoomPlan.decode(canonical(body))

    def test_rotations_have_distinct_plan_and_request_digests(self):
        plans = [RoomPlan.compile(request('dining', angle)) for angle in (0, 90, 180, 270)]
        self.assertEqual(len({p.digest for p in plans}), 4)
        self.assertEqual(len({p.request().digest for p in plans}), 4)

    def test_cancellation_propagates_before_any_result(self):
        value = request('dining', 90)
        for stop in (1, 2, 10, 30, 60, 90, 120, 150):
            calls = 0
            def guard():
                nonlocal calls
                calls += 1
                if calls == stop:
                    raise InterruptedError('cancelled rotation')
            with self.subTest(stop=stop), self.assertRaisesRegex(InterruptedError, 'cancelled rotation'):
                RoomPlan.compile(value, guard)
            self.assertEqual(calls, stop)

    def test_no_live_or_mutation_claim_is_added(self):
        body = RoomPlan.compile(request('dining', 90)).json()
        for field in ('terrain_observed', 'map_dimensions_observed', 'native_pathfinding_proven',
                      'room_assignments_created', 'prepared_plan_created', 'mutation_authority_granted',
                      'room_completion_proven', 'construction_completion_proven'):
            self.assertIs(body[field], False)


if __name__ == '__main__':
    unittest.main()
