"""Search complete room templates over their declared column counts."""
from copy import deepcopy
import json
from pathlib import Path
import unittest

from jsonschema import Draft202012Validator
from room_layout import COST_OBJECTIVE, MINIMUM_TARGETS_OBJECTIVE, Layout, WorkLimit
from test_room_layout import dining, request, solve


class RoomLayoutColumnTests(unittest.TestCase):
    schema = Draft202012Validator(json.loads(
        (Path(__file__).resolve().parents[1] / 'schemas/room_layout_request_v1.json').read_bytes()))

    def test_narrow_site_retains_all_seven_bedrooms_and_material_constraints(self):
        area = {'name': 'sleep', 'template': {'kind': 'bedroom_cluster', 'rooms_count': 7,
                'room_size': [1, 2], 'furnishings': 'bed_only', 'columns': [4, 2, 1]},
                'item_constraints': {'bed': {'material': [7, -1]}}}
        document = request(area, size=(5, 36, 1))
        self.schema.validate(document)
        result = solve(document)
        self.assertEqual(Layout.decode(result.encode()), result)
        self.assertEqual(result.json()['request']['areas'][0]['template']['columns'], [1, 2, 4])
        room = result.room_plan()
        self.assertEqual(room.json()['intent']['areas'][0]['template']['columns'], 1)
        self.assertEqual(len(room.request().slots), 7)
        self.assertTrue(all(slot.material == (7, -1) for slot in room.request().slots))
        area['template']['columns'] = [2, 4]
        self.assertEqual(solve(document).json()['status'], 'no_layout_in_domain')

    def test_choice_and_rotation_match_exhaustive_fixed_intents_for_both_objectives(self):
        area = dining(count=3, columns=[3, 2, 1], origin=[17, 16, 2], rotations=[0, 180])
        for objective in (MINIMUM_TARGETS_OBJECTIVE, COST_OBJECTIVE):
            document = request(area, size=(20, 18, 1), objective=objective)
            if objective == COST_OBJECTIVE:
                document['excavation_costs'] = ''.join('1' if i % 7 <= 2 else '0' for i in range(360))
            def score(result):
                score = result.json()['optimization']['score']
                return tuple(score[k] for k in result.json()['optimization']['comparison_order'])
            fixed = []
            for width in (1, 2, 3):
                for angle in (0, 180):
                    concrete = deepcopy(document)
                    candidate = concrete['areas'][0]
                    candidate['template']['columns'] = width
                    del candidate['rotations']
                    candidate['rotation'] = angle
                    result = solve(concrete)
                    if result.json()['status'] == 'layout_found':
                        fixed.append(score(result))
            selected = solve(document)
            self.assertEqual(score(selected), min(fixed))
            self.assertTrue(selected.json()['optimization']['minimum_in_compiler_domain_proven'])
            self.assertEqual(len(selected.room_plan().request().slots), 6)
            self.assertEqual(Layout.decode(selected.encode()), selected)

    def test_existing_floor_strip_changes_the_selected_dormitory_shape(self):
        from room_layout_observed import ObservedLayout
        from test_room_layout_excavation_observed import request as observed_request, solve as observed_solve
        area = {'name': 'sleep', 'origin': [10, 10, 2],
                'template': {'kind': 'dormitory', 'bed_count': 4, 'columns': [1, 4]}}
        document = observed_request(area, size=(5, 8, 1))
        result = observed_solve(document, 'FFWWW' * 8)
        body = result.json()
        self.assertEqual(body['layout']['room_plan']['intent']['areas'][0]['template']['columns'], 1)
        self.assertEqual(body['excavation_evidence']['observed_wall_targets'], 0)
        self.assertEqual(body['excavation_evidence']['observed_floor_targets'], 16)
        self.assertEqual(ObservedLayout.decode(result.encode()), result)

    def test_compiler_capacity_excludes_only_unrepresentable_shapes(self):
        area = {'name': 'offices', 'template': {'kind': 'office_cluster', 'rooms_count': 14,
                'room_size': [1, 2], 'columns': [1, 7]}}
        document = request(area, size=(17, 12, 1))
        self.schema.validate(document)
        result = solve(document)
        self.assertEqual(result.json()['request']['areas'][0]['template']['columns'], [1, 7])
        room = result.room_plan()
        self.assertEqual(room.json()['intent']['areas'][0]['template']['columns'], 7)
        self.assertEqual(len(room.request().slots), 14)
        area['template']['columns'] = [1, 2]
        self.assertFalse(self.schema.is_valid(document))
        with self.assertRaisesRegex(ValueError, 'capacity'):
            solve(document)

    def test_mixed_project_requires_compatible_widths_for_both_areas(self):
        def area(name, y):
            return {'name': name, 'origin': [10, y, 2],
                    'template': {'kind': 'dormitory', 'bed_count': 4, 'columns': [1, 4]}}
        document = request(area('south', 12), area('north', 10), size=(5, 4, 1))
        result = solve(document)
        plan = result.room_plan()
        self.assertEqual([a['template']['columns'] for a in plan.json()['intent']['areas']], [4, 4])
        self.assertTrue(plan.json()['project_connections']['all_areas_connected'])
        self.assertEqual(len(plan.request().slots), 8)
        document['search'] = {'max_nodes': 2}
        limited = solve(document).json()
        self.assertEqual(limited['status'], 'search_limit_reached')
        self.assertIsNone(limited['room_plan'])
        document['search'] = {'max_work': 5}
        with self.assertRaises(WorkLimit):
            solve(document)

    def test_reordering_choices_replays_and_single_choice_matches_fixed_bytes(self):
        area = dining(count=2, columns=[1, 2])
        document = request(area, size=(8, 6, 1))
        result = solve(document)
        area['template']['columns'].reverse()
        self.assertEqual(solve(document), result)
        area['template']['columns'] = [2]
        single = solve(document)
        area['template']['columns'] = 2
        self.assertEqual(solve(document), single)
        for invalid in ([], [2, 2], [0, 2], [1, 5], [True, 2], [1, None]):
            area['template']['columns'] = invalid
            self.assertFalse(self.schema.is_valid(document))
            with self.assertRaises(ValueError):
                solve(document)
