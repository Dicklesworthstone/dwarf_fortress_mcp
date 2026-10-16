"""Global orientation search through real compiler, allocation and CLI exports."""
from __future__ import annotations

from copy import deepcopy
import itertools
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parents[1] / 'scripts'
sys.path.insert(0, str(SCRIPTS))
from furniture_allocation import Candidate, Request, allocate
from furniture_plan import canonical
from room_provisioning import RoomPlan
import room_layout as layout


def dining(name='dining', **extra):
    return {'name': name, 'template': {'kind': 'dining_hall', 'table_count': 4, 'columns': 4}, **extra}


def request(*areas, size=(3, 13, 1), **extra):
    return {'schema': layout.REQUEST_SCHEMA, 'world_folder': 'layout-rotation-fixture', 'site': 7,
            'region': {'origin': [100, 100, 2], 'size': list(size)},
            'areas': list(areas) or [dining(rotations=[0, 90, 180, 270])], **extra}


def solve(value):
    return layout.solve(canonical(value))


def mixed_request():
    # The left strip fits only 0 degrees at its pivot; the right only 90.
    value = request(dining('horizontal', origin=[100, 100, 2], rotations=[0, 90, 180, 270]),
                    dining('vertical', origin=[115, 100, 2], rotations=[0, 90, 180, 270]),
                    size=(16, 13, 1))
    value['site_mask'] = {'region': deepcopy(value['region']), 'cells': ''.join(
        'F' if (x <= 12 and y <= 2) or x >= 13 else 'X'
        for y in range(13) for x in range(16))}
    return value


class OrientedLayoutTests(unittest.TestCase):
    def check_plan(self, result):
        value = result.json()
        self.assertEqual(value['status'], 'layout_found')
        self.assertEqual(layout.Layout.decode(result.encode()), result)
        plan = result.room_plan()
        self.assertEqual(RoomPlan.decode(plan.encode()), plan)
        self.assertEqual(len(plan.json()['areas']), len(value['request']['areas']))
        self.assertTrue(plan.json()['project_connections']['all_areas_connected'])
        return plan

    def test_legacy_result_digests_and_zero_forms_are_unchanged(self):
        templates = [
            (dining(), '399023112ff53624c4fea193568d7f83d1ab0ec44d787619f69313cecfbacf65'),
            ({'name': 'bedrooms', 'template': {'kind': 'bedroom_cluster', 'rooms_count': 2,
              'room_size': [3, 5]}}, '59adadc8c64ab2b67c7c4294ea0dfe3622ca9e98f2ccdd6ac3271d8077b5e8d8')]
        for area, digest in templates:
            original = solve(request(area, size=(20, 20, 1)))
            self.assertEqual(original.digest, digest)
            for extra in ({'rotation': 0}, {'rotations': [0]}):
                self.assertEqual(solve(request({**area, **extra}, size=(20, 20, 1))).encode(), original.encode())
            self.assertEqual(original.json()['policy'], layout.POLICY)

    def test_narrow_site_requires_orientation_not_translation(self):
        old = solve(request(dining())).json()
        self.assertEqual(old['status'], 'no_layout_in_domain')
        result = solve(request())
        plan = self.check_plan(result)
        area = plan.json()['intent']['areas'][0]
        self.assertEqual((area['origin'], area['rotation']), ([102, 100, 2], 90))
        self.assertEqual(plan.json()['capture_region'], result.json()['request']['region'])
        self.assertEqual(result.json()['policy'], layout.ORIENTED_POLICY)

    def test_complete_project_chooses_different_orientations_together(self):
        result = solve(mixed_request())
        plan = self.check_plan(result)
        areas = plan.json()['intent']['areas']
        self.assertEqual([(a['name'], a.get('rotation', 0)) for a in areas],
                         [('horizontal', 0), ('vertical', 90)])
        self.assertEqual(plan.json()['furniture_count'], 16)
        self.assertEqual(plan.json()['target_tiles'], 78)
        self.assertEqual(plan.json()['project_connections']['new_corridor_tiles'], 0)
        self.assertTrue(all('rotations' not in a for a in areas))
        self.assertEqual([a['rotations'] for a in result.json()['request']['areas']],
                         [[0, 90, 180, 270], [0, 90, 180, 270]])

    def test_allowed_order_and_area_order_do_not_change_canonical_result(self):
        value = mixed_request()
        original = solve(value).encode()
        value['areas'].reverse()
        for area in value['areas']:
            area['rotations'].reverse()
        self.assertEqual(solve(value).encode(), original)
        for choices in itertools.permutations((0, 90, 180, 270)):
            value = request(dining(rotations=list(choices)))
            self.assertEqual(solve(value).encode(), solve(request()).encode())

    def test_single_choice_equals_fixed_rotation_and_all_fixed_angles_work(self):
        for angle in (0, 90, 180, 270):
            fixed = request(dining(rotation=angle), size=(20, 20, 1))
            offered = request(dining(rotations=[angle]), size=(20, 20, 1))
            result = solve(fixed)
            self.assertEqual(result.encode(), solve(offered).encode())
            self.assertEqual(self.check_plan(result).json()['intent']['areas'][0].get('rotation', 0), angle)

    def test_origin_constraints_are_not_relaxed_to_obtain_a_fit(self):
        good = request(dining(rotations=[0, 90, 180, 270], origin=[102, 100, 2]))
        self.check_plan(solve(good))
        for origin in ([100, 100, 2], [101, 100, 2]):
            bad = request(dining(rotations=[0, 90, 180, 270], origin=origin))
            value = solve(bad).json()
            self.assertEqual(value['status'], 'no_layout_in_domain')
            self.assertIsNone(value['room_plan'])
        domain = {'origin': [102, 100, 2], 'size': [1, 1, 1]}
        self.check_plan(solve(request(dining(rotations=[0, 90, 180, 270], origin_region=domain))))

    def test_shared_site_constraints_do_not_become_rotatable(self):
        value = request()
        value['site_mask'] = {'region': deepcopy(value['region']), 'cells': 'W' + '.' * 38}
        out = solve(value).json()
        self.assertEqual(out['status'], 'no_layout_in_domain')
        self.assertTrue(out['search']['domain_exhausted'])
        self.assertIsNone(out['room_plan'])

    def test_rotation_choices_are_bounded_exact_and_unambiguous(self):
        cases = ([], [0] * 5, [0, 0], [True], [90.0], ['90'], [None], [45], [-90], [360],
                 '0,90', {}, None)
        for choices in cases:
            with self.subTest(choices=choices), self.assertRaises(ValueError):
                solve(request(dining(rotations=choices)))
        with self.assertRaisesRegex(ValueError, 'not both'):
            solve(request(dining(rotation=90, rotations=[90])))
        for angle in (True, 90.0, '90', 45, None):
            with self.subTest(angle=angle), self.assertRaises(ValueError):
                solve(request(dining(rotation=angle)))

    def test_global_node_limit_is_inconclusive_never_a_partial_or_relaxed_plan(self):
        value = mixed_request()
        value['search'] = {'max_nodes': 2}
        out = solve(value).json()
        self.assertEqual(out['status'], 'search_limit_reached')
        self.assertEqual(out['search']['nodes_visited'], 2)
        self.assertFalse(out['search']['domain_exhausted'])
        self.assertFalse(out['all_areas_placed'])
        self.assertIsNone(out['room_plan'])
        value['search']['max_nodes'] = 3
        self.check_plan(solve(value))

    def test_one_work_budget_covers_all_shapes_search_and_lowering(self):
        value = request()
        used = solve(value).json()['search']['work_units']
        for limit in (1, 100, used // 2, used - 1):
            with self.subTest(limit=limit), self.assertRaises(layout.WorkLimit):
                solve({**value, 'search': {'max_work': limit}})
        result = solve({**value, 'search': {'max_work': used}})
        self.assertEqual(result.json()['search']['work_units'], used)
        self.check_plan(result)

    def test_cancellation_in_variants_sort_search_and_final_compile_propagates(self):
        raw = canonical(request())
        calls = []
        layout.solve(raw, lambda: calls.append(1))
        # Sample the entire guarded path, including the final guarded return.
        for stop in sorted(set([1, 2, len(calls)] + list(range(1, len(calls) + 1, 29)))):
            seen = 0
            def guard():
                nonlocal seen
                seen += 1
                if seen == stop:
                    raise ValueError('caller cancelled oriented search')
            with self.subTest(stop=stop), self.assertRaisesRegex(ValueError, 'caller cancelled'):
                layout.solve(raw, guard)
            self.assertEqual(seen, stop)

    def test_candidate_inventory_is_bounded_across_four_orientations(self):
        area = dining(rotations=[0, 90, 180, 270])
        area['template']['table_count'] = area['template']['columns'] = 1
        result = solve(request(area, size=(32, 32, 1)))
        count = result.json()['search']['candidate_counts'][0]['count']
        self.assertEqual(count, 4 * 29 * 30)
        self.assertLessEqual(count, 4 * layout.MAX_CAPTURE_VOLUME)
        self.assertLessEqual(count, layout.MAX_CANDIDATES)
        self.check_plan(result)

    def test_exhaustive_obstacle_masks_match_independent_rotated_rectangle_oracle(self):
        # 64 obstacle sets; both 4x3 and 3x4 translations, without solver helpers.
        blockers = [(100, 100, 2), (104, 100, 2), (100, 103, 2),
                    (104, 103, 2), (102, 101, 2), (101, 102, 2)]
        shapes = []
        for width, height in ((4, 3), (3, 4)):
            for x in range(100, 106 - width):
                for y in range(100, 105 - height):
                    shapes.append({(xx, yy, 2) for xx in range(x, x + width) for yy in range(y, y + height)})
        seen = set()
        for bits in range(64):
            blocked = {p for i, p in enumerate(blockers) if bits & (1 << i)}
            area = dining(rotations=[0, 90, 180, 270])
            area['template']['table_count'] = area['template']['columns'] = 1
            value = request(area, size=(5, 4, 1), excluded_regions=[
                {'origin': list(p), 'size': [1, 1, 1]} for p in sorted(blocked)])
            result = solve(value).json()
            expected = any(not shape & blocked for shape in shapes)
            self.assertEqual(result['status'] == 'layout_found', expected, bits)
            seen.add(expected)
            if not expected:
                self.assertTrue(result['search']['domain_exhausted'])
                self.assertIsNone(result['room_plan'])
        self.assertEqual(seen, {True, False})

    def test_tampered_selected_angle_or_allowed_set_is_rejected(self):
        result = solve(request())
        for field in ('allowed', 'selected', 'policy', 'trace'):
            value = result.json()
            if field == 'allowed':
                value['request']['areas'][0]['rotations'] = [90]
            elif field == 'selected':
                value['room_plan']['intent']['areas'][0]['rotation'] = 270
            elif field == 'policy':
                value['policy'] = layout.POLICY
            else:
                value['search']['trace_digest'] = '0' * 64
            with self.subTest(field=field), self.assertRaises(ValueError):
                layout.Layout.decode(canonical(value))

    def test_actual_allocation_preserves_complete_rotated_project(self):
        value = mixed_request()
        before = deepcopy(value)
        plan = self.check_plan(solve(value))
        self.assertEqual(value, before)
        wanted = plan.request()
        candidates = tuple(Candidate(i, slot.kind, slot.target, (0, 1), -1)
                           for i, slot in enumerate(wanted.slots))
        result = allocate(wanted, candidates)
        self.assertEqual(result['status'], 'allocated')
        self.assertEqual(len(result['plan']['steps']), 16)
        self.assertEqual(result['total_distance'], 0)
        shortage = allocate(wanted, candidates[:-1])
        self.assertEqual(shortage['status'], 'shortage')
        self.assertIsNone(shortage['plan'])

    def test_hash_seed_does_not_change_oriented_result(self):
        value = mixed_request()
        expected = solve(value).encode()
        code = 'import sys,room_layout;sys.stdout.buffer.write(room_layout.solve(sys.stdin.buffer.read()).encode())'
        for seed in ('1', '19', 'random'):
            process = subprocess.run([sys.executable, '-c', code], input=canonical(value), capture_output=True,
                env={**os.environ, 'PYTHONPATH': str(SCRIPTS), 'PYTHONHASHSEED': seed}, check=True, timeout=15)
            self.assertEqual(process.stdout, expected)

    def test_cli_solve_replay_and_exact_downstream_exports(self):
        result = solve(request())
        plan = result.room_plan()
        expected = {'layout': result.encode(), 'room-plan': plan.encode(),
                    'room-request': canonical(plan.json()['intent']),
                    'excavation': canonical(plan.json()['excavation_blueprint']),
                    'furniture-request': canonical(plan.json()['furniture_request'])}
        with tempfile.TemporaryDirectory() as directory:
            source, saved = Path(directory) / 'request.json', Path(directory) / 'layout.json'
            source.write_bytes(canonical(request()))
            saved.write_bytes(result.encode())
            for argument, path in (('--request-file', source), ('--layout-file', saved)):
                for kind, raw in expected.items():
                    process = subprocess.run([sys.executable, str(SCRIPTS / 'layout_rooms.py'),
                        argument, str(path), '--emit', kind], capture_output=True, check=True, timeout=15)
                    self.assertEqual(process.stdout, raw, (argument, kind))
                    self.assertEqual(process.stderr, b'')
                    if kind == 'furniture-request':
                        self.assertEqual(Request.decode(process.stdout), plan.request())

    def test_cli_inconclusive_search_refuses_to_export_partial_rooms(self):
        value = mixed_request()
        value['search'] = {'max_nodes': 1}
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'request.json'
            source.write_bytes(canonical(value))
            process = subprocess.run([sys.executable, str(SCRIPTS / 'layout_rooms.py'),
                '--request-file', str(source), '--emit', 'room-plan'], capture_output=True, timeout=15)
            self.assertEqual(process.returncode, 2)
            reply = json.loads(process.stdout)
            self.assertEqual(reply['error']['code'], 'ROOM_LAYOUT_SEARCH_LIMIT')
            self.assertIsNone(reply['result'])
            self.assertFalse(reply['agent_turn']['briefing']['mutation_admissible'])


if __name__ == '__main__':
    unittest.main()
