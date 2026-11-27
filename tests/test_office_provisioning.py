"""Physical private offices from site layout through exact furniture allocation."""
import json
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from jsonschema import Draft202012Validator
from furniture_plan import canonical
from room_furniture_handoff import RoomFurnitureHandoff
from room_inventory_peer import Peer, capture as inventory
from room_provisioning import RoomPlan
from room_terrain import selection, survey
from room_terrain_fixtures import MANIFEST, capture, visible
from test_plan_rooms import invoke
from test_room_layout import request, solve
from test_room_provisioning import check_access, intent


def offices(count=1, size=(2, 1)):
    return {'name': 'offices', 'origin': [40, 40, 2],
            'template': {'kind': 'office_cluster', 'rooms_count': count, 'room_size': list(size)},
            'item_constraints': {'chair': {'material': [7, -1]}}}


class OfficeProvisioningTests(unittest.TestCase):
    def test_private_walls_and_accessible_chairs_at_all_orientations(self):
        for angle in (0, 90, 180, 270):
            for size in ((1, 2), (2, 1), (3, 4)):
                with self.subTest(rotation=angle, size=size):
                    area = {**offices(5, size), 'rotation': angle}
                    plan = RoomPlan.compile(intent(area))
                    check_access(self, plan)
                    room = plan.json()['areas'][0]
                    for unit, chair in zip(room['units'], plan.request().slots, strict=True):
                        self.assertEqual((chair.kind, chair.material), ('chair', (7, -1)))
                        self.assertEqual(unit['slots'], [chair.name])
                        self.assertNotEqual(tuple(unit['doorway']), chair.target)
                    original = selection(plan)
                    self.assertTrue(original.walls)
                    blocked = survey(plan, capture(original.region, {min(original.walls): visible(3)}), MANIFEST)
                    self.assertEqual(blocked['status'], 'blocked')
                    self.assertIsNone(blocked['remaining_blueprint'])

    def test_automatic_layout_schema_and_replay_require_the_original_offices(self):
        area = offices(14, (1, 2))
        del area['origin']
        area['template']['columns'] = 7
        document = request(area, size=(17, 12, 1))
        schema = Path(__file__).resolve().parents[1] / 'schemas/room_layout_request_v1.json'
        validator = Draft202012Validator(json.loads(schema.read_bytes()))
        validator.validate(document)
        result = solve(document)
        plan = result.room_plan()
        self.assertEqual(len(plan.request().slots), 14)
        self.assertEqual(len(plan.json()['excavation_blueprint']['parts']), 31)
        self.assertEqual(RoomPlan.decode(plan.encode()), plan)
        for bad in ({'bed': {}}, {'table': {}}):
            area['item_constraints'] = bad
            self.assertFalse(validator.is_valid(document))
            with self.assertRaises(ValueError):
                solve(document)

    def test_cli_allocates_complete_chair_set_and_refuses_shortage(self):
        with TemporaryDirectory() as directory:
            source = Path(directory) / 'office.json'
            source.write_bytes(canonical(intent(offices(2))))
            result = invoke('compile', '--request-file', str(source))
            self.assertEqual(result.returncode, 0, result.stdout)
            plan = RoomPlan.decode(result.stdout)
            source.write_bytes(result.stdout)
            for omitted in [(), (plan.request().slots[0].name,)]:
                with Peer(inventory(plan.request(), omit=omitted)) as peer:
                    output = invoke('allocate', '--plan-file', str(source), '--emit', 'room-handoff',
                                    env=peer.environment())
                if omitted:
                    self.assertEqual(output.returncode, 2)
                else:
                    self.assertEqual(output.returncode, 0, output.stdout)
                    handoff = RoomFurnitureHandoff.decode(output.stdout)
                    self.assertEqual(handoff.room_plan, plan)
                    self.assertEqual([step.kind for step in handoff.allocation.plan().steps],
                                     ['chair', 'chair'])
