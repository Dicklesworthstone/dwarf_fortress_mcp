"""Actual compiler/router/solver tests; no native or replacement compiler inputs."""
import copy
import hashlib
import itertools
import json
from pathlib import Path
import unittest

from furniture_plan import canonical
from room_layout import Layout, WorkLimit, solve
from room_provisioning import RoomPlan
from room_site_mask import SiteMask


def area(name='a', kind='dining_hall', origin=None):
    template = {'kind': kind, **({'table_count': 1} if kind == 'dining_hall' else
                               {'rooms_count': 1, 'room_size': [3, 3]})}
    return {'name': name, 'template': template, **({} if origin is None else {'origin': list(origin)})}


def request(*areas, size=(12, 12, 1)):
    return {'schema': 'dfmcp.room-layout-request/1', 'world_folder': 'region1', 'site': 2,
            'region': {'origin': [8, 8, 2], 'size': list(size)}, 'areas': list(areas) or [area()]}


def mask(region, overrides=None, default='.'):
    ox, oy, oz = region['origin']
    sx, sy, _ = region['size']
    overrides = overrides or {}
    return {'region': copy.deepcopy(region), 'cells': ''.join(
        overrides.get((x, y, oz), default) for y in range(oy, oy + sy) for x in range(ox, ox + sx))}


def floor_set(plan):
    return {(x, y, z) for part in plan.json()['excavation_blueprint']['parts']
            for ox, oy, z in [part['region']['origin']] for sx, sy, _ in [part['region']['size']]
            for y in range(oy, oy + sy) for x in range(ox, ox + sx)}


def provision(*areas):
    return {'schema': 'dfmcp.room-provisioning-request/1', 'world_folder': 'region1',
            'site': 2, 'connect_areas': True, 'areas': list(areas)}


class SiteMaskTests(unittest.TestCase):
    def test_exact_mask_roundtrip_and_outside_is_never_permitted(self):
        m = SiteMask((8, 8, 2), (4, 1, 1), '.FWX')
        self.assertEqual(SiteMask.from_json(m.json()), m)
        self.assertEqual([m.permits_floor((8 + i, 8, 2)) for i in range(4)], [True, True, False, False])
        self.assertEqual([m.permits_wall((8 + i, 8, 2)) for i in range(4)], [True, False, True, False])
        for p in ((7, 8, 2), (12, 8, 2), (8, 7, 2), (8, 8, 3)):
            self.assertFalse(m.permits_floor(p)); self.assertFalse(m.permits_wall(p))
        value = m.json(); value['region']['origin'][0] = 1
        self.assertEqual(m.origin, (8, 8, 2))

    def test_closed_shape_bounds_and_exact_scalar_types(self):
        good = mask(request()['region'])
        bad = [None, [], {**good, 'authority': True}, {**good, 'cells': '.' * 143},
               {**good, 'cells': '.' * 145}, {**good, 'cells': '.' * 143 + '\n'},
               {**good, 'cells': '.' * 143 + 'f'}, {**good, 'cells': ['.'] * 144}]
        for which in ('origin', 'size'):
            for values in ([True, 8, 2], [1, 1], [32766, 128, 1], [1, 1, 2.0]):
                row = copy.deepcopy(good); row['region'][which] = values; bad.append(row)
        bad.append({'region': {'origin': [1, 1, 1], 'size': [128, 128, 1]}, 'cells': '.' * 16384})
        for value in bad:
            with self.subTest(value=str(value)[:70]), self.assertRaises(ValueError):
                SiteMask.from_json(value)

    def test_floor_and_required_wall_constraints_are_distinct(self):
        v = provision(area(kind='bedroom_cluster', origin=(10, 10, 2)))
        region = {'origin': [8, 9, 2], 'size': [6, 6, 1]}
        for cell in 'XW':
            v['site_mask'] = mask(region, {(10, 10, 2): cell})
            self.assertRaisesRegex(ValueError, 'floor forbidden', RoomPlan.compile, v)
        for cell in 'FX':
            v['site_mask'] = mask(region, {(9, 9, 2): cell})
            self.assertRaisesRegex(ValueError, 'wall forbidden', RoomPlan.compile, v)
        v['site_mask'] = mask(region, {(9, 9, 2): 'W', (11, 13, 2): 'F'})
        p = RoomPlan.compile(v)
        self.assertEqual(RoomPlan.decode(p.encode()), p)
        self.assertEqual(p.json()['intent']['site_mask'], v['site_mask'])

    def test_corridors_reroute_instead_of_crossing_blocked_cells(self):
        v = provision(area('a', origin=(10, 10, 2)), area('b', origin=(18, 10, 2)))
        old = RoomPlan.compile(v)
        self.assertIn((15, 10, 2), floor_set(old))
        v['site_mask'] = mask({'origin': [10, 10, 2], 'size': [12, 3, 1]}, {(15, 10, 2): 'X'})
        new = RoomPlan.compile(v)
        self.assertNotIn((15, 10, 2), floor_set(new))
        self.assertEqual(new.request(), old.request())
        self.assertEqual(len(new.request().slots), 4)
        v['site_mask'] = mask(v['site_mask']['region'], {(15, y, 2): 'X' for y in range(10, 13)})
        self.assertRaisesRegex(ValueError, 'no corridor', RoomPlan.compile, v)

    def test_solver_prunes_wall_incompatible_origins_before_leaf_compilation(self):
        v = request(area(kind='bedroom_cluster'))
        v['site_mask'] = mask(v['region'], default='F')
        out = solve(canonical(v)).json()
        self.assertEqual(out['status'], 'no_layout_in_domain')
        self.assertEqual(out['search']['complete_layouts_tested'], 0)
        self.assertIsNone(out['room_plan'])
        # A single valid preserved-wall footprint is enough; no caller origin supplied.
        walls = {(x, y, 2): '.' for y in range(9, 14) for x in range(9, 14)
                 if (x in (9, 13) or y in (9, 13)) and (x, y) != (11, 13)}
        v['site_mask'] = mask(v['region'], walls, default='F')
        found = solve(canonical(v))
        self.assertEqual(found.json()['status'], 'layout_found')
        self.assertEqual(found.room_plan().json()['intent']['areas'][0]['origin'], [10, 10, 2])
        self.assertEqual(Layout.decode(found.encode()), found)

    def test_256_masks_match_independent_complete_floor_search(self):
        tested = [(x, y, 2) for y in (8, 10) for x in range(8, 12)]
        for bits in range(256):
            blocked = {p for i, p in enumerate(tested) if bits & (1 << i)}
            v = request(size=(6, 3, 1))
            v['site_mask'] = mask(v['region'], {p: 'X' for p in blocked})
            possible = [(x, 8, 2) for x in range(8, 11)
                        if not ({(xx, yy, 2) for xx in range(x, x + 4) for yy in range(8, 11)} & blocked)]
            found = solve(canonical(v)).json()
            self.assertEqual(found['status'] == 'layout_found', bool(possible), bits)
            if possible:
                self.assertEqual(found['room_plan']['intent']['areas'][0]['origin'], list(possible[0]))

    def test_solver_retains_exclusions_constraints_and_complete_request(self):
        v = request(area('a'), area('b'))
        v['areas'][1]['item_constraints'] = {'chair': {'material': [10, 2], 'max_distance': 17}}
        v['site_mask'] = mask(v['region'], {(8, 8, 2): 'X'})
        v['excluded_items'] = [42]
        v['excluded_regions'] = [{'origin': [9, 8, 2], 'size': [1, 1, 1]}]
        out = solve(canonical(v)); p = out.room_plan()
        self.assertEqual(len(p.request().slots), 4)
        self.assertTrue({(8, 8, 2), (9, 8, 2)}.isdisjoint(floor_set(p)))
        self.assertEqual(p.request().excluded_items, (42,))
        chair = next(s for s in p.request().slots if s.name == 'b.001.chair')
        self.assertEqual((chair.material, chair.max_distance), ((10, 2), 17))
        self.assertFalse(p.json()['terrain_observed'])
        reverse = copy.deepcopy(v); reverse['areas'].reverse()
        self.assertEqual(out.encode(), solve(canonical(reverse)).encode())

    def test_different_domain_and_partial_or_forged_imports_refuse(self):
        v = request(); v['site_mask'] = mask(v['region'])
        result = solve(canonical(v))
        modified = result.json(); modified['room_plan']['intent'].pop('site_mask')
        self.assertRaises(ValueError, Layout.decode, canonical(modified))
        modified = result.room_plan().json(); modified['intent']['site_mask']['cells'] = 'X' * 144
        self.assertRaises(ValueError, RoomPlan.decode, canonical(modified))
        v['site_mask']['region']['origin'][0] += 1
        self.assertRaisesRegex(ValueError, 'complete layout domain', solve, canonical(v))

    def test_exact_work_and_node_limits_never_return_partial_layout(self):
        v = request(); v['site_mask'] = mask(v['region'])
        out = solve(canonical(v)).json()
        used = out['search']['work_units']
        v['search'] = {'max_work': used}
        self.assertEqual(solve(canonical(v)).json()['status'], 'layout_found')
        v['search']['max_work'] -= 1
        self.assertRaises(WorkLimit, solve, canonical(v))
        v['search'] = {'max_nodes': 1}
        result = solve(canonical(v)).json()
        self.assertEqual(result['status'], 'search_limit_reached')
        self.assertIsNone(result['room_plan'])

    def test_guard_exceptions_are_not_swallowed_as_geometry_refusals(self):
        v = request(size=(4, 3, 1)); v['site_mask'] = mask(v['region'])
        raw = canonical(v); calls = []
        solve(raw, lambda: calls.append(1))
        for limit in range(len(calls)):
            left = [limit]
            def guard():
                if not left[0]:
                    raise ValueError('revoked')
                left[0] -= 1
            self.assertRaisesRegex(ValueError, 'revoked', solve, raw, guard)

    def test_maximum_mask_and_eight_areas_keep_all_furnishings(self):
        v = request(*[area(str(i)) for i in range(8)], size=(32, 32, 1))
        for i, a in enumerate(v['areas']):
            a['origin'] = [8 + i % 4 * 7, 8 + i // 4 * 5, 2]
            a['template']['table_count'] = 2
        v['site_mask'] = mask(v['region'])
        found = solve(canonical(v)); plan = found.room_plan()
        self.assertEqual(len(plan.request().slots), 32)
        self.assertEqual(len(plan.json()['intent']['areas']), 8)
        self.assertEqual(Layout.decode(found.encode()), found)

    def test_schema_accepts_masks_and_refuses_malformed_structures(self):
        import jsonschema
        schema = json.loads((Path(__file__).resolve().parents[1] / 'schemas/room_layout_request_v1.json').read_text())
        jsonschema.Draft202012Validator.check_schema(schema)
        validator = jsonschema.Draft202012Validator(schema)
        v = request(); v['site_mask'] = mask(v['region'])
        validator.validate(v)
        validator.validate(solve(canonical(v)).json()['request'])
        for value in (None, [], {}, {'region': v['region'], 'cells': 'x'},
                      {'region': v['region'], 'cells': '.\n'}, {'region': v['region'], 'cells': '.' * 1025}):
            v['site_mask'] = value
            self.assertFalse(validator.is_valid(v))
            self.assertRaises(ValueError, solve, canonical(v))


if __name__ == '__main__':
    unittest.main()
