"""Private bedrooms through layout, terrain survey and inventory allocation."""
from copy import deepcopy
from itertools import product
from pathlib import Path
from tempfile import TemporaryDirectory
import json
import unittest

from test_plan_rooms import invoke
from test_room_provisioning import bedroom, check_access, intent, mask, reference_cluster
from test_room_layout import request, solve
from room_inventory_peer import Peer, capture as inventory_capture
from room_terrain_fixtures import MANIFEST, capture as map_capture, visible
from furniture_plan import canonical
from room_furniture_handoff import RoomFurnitureHandoff
from room_provisioning import RoomPlan
import room_terrain as terrain


def private(count, size):
    area = bedroom(count, size, origin=(40, 40, 2))
    area['template']['furnishings'] = 'bed_only'
    return area


class PrivateBedroomTests(unittest.TestCase):
    def test_cardinal_interior_access_for_every_supported_shape(self):
        for width, height in product(range(1, 8), repeat=2):
            if (width, height) == (1, 1):
                continue
            with self.subTest(size=(width, height)):
                room = RoomPlan.compile(intent(private(5, (width, height))))
                self.assertEqual(mask(room), reference_cluster(5, width, height, (40, 40, 2)))
                check_access(self, room)
                for unit, slot in zip(room.json()['areas'][0]['units'], room.request().slots, strict=True):
                    self.assertEqual(unit['slots'], [slot.name])
                    self.assertEqual(slot.kind, 'bed')
                    _, y, _ = unit['origin']
                    approach = (unit['doorway'][0], y + height - 1, 2)
                    self.assertNotEqual(slot.target, approach)
                    self.assertIn(approach, mask(room))

    def test_private_walls_are_checked_in_the_observed_terrain(self):
        for angle in (0, 90, 180, 270):
            with self.subTest(angle=angle):
                area = private(3, (1, 2))
                area['rotation'] = angle
                room = RoomPlan.compile(intent(area))
                selection = terrain.selection(room)
                self.assertTrue(selection.walls)
                result = terrain.survey(room, map_capture(selection.region), MANIFEST)
                self.assertEqual(result['status'], 'excavation_proposed')
                self.assertEqual(set(map(tuple, result['remaining_wall_targets'])), mask(room))
                altered = map_capture(selection.region, {min(selection.walls): visible(3)})
                blocked = terrain.survey(room, altered, MANIFEST)
                self.assertEqual(blocked['status'], 'blocked')
                self.assertIsNone(blocked['remaining_blueprint'])

    def test_automatic_layout_retains_thirteen_private_rooms_and_bed_constraints(self):
        from jsonschema import Draft202012Validator
        schema = Path(__file__).resolve().parents[1] / 'schemas/room_layout_request_v1.json'
        validator = Draft202012Validator(json.loads(schema.read_bytes()))
        area = private(13, (1, 2))
        del area['origin']
        area['item_constraints'] = {'bed': {'material': [7, -1]}}
        document = request(area, size=(12, 23, 1))
        validator.validate(document)
        layout = solve(document)
        room = layout.room_plan()
        self.assertEqual(len(room.json()['areas'][0]['units']), 13)
        self.assertEqual(len(room.json()['excavation_blueprint']['parts']), 31)
        self.assertEqual(len(room.request().slots), 13)
        self.assertTrue(all(s.material == (7, -1) for s in room.request().slots))
        self.assertEqual(RoomPlan.decode(room.encode()), room)
        validator.validate(layout.json()['request'])

        for key, value in (('room_size', [1, 1]), ('rooms_count', 14),
                           ('room_size', [True, 2]), ('furnishings', 'empty'),
                           ('furnishings', 'bed_chair_table')):
            bad = deepcopy(document)
            bad['areas'][0]['template'][key] = value
            self.assertFalse(validator.is_valid(bad))
            with self.assertRaises(ValueError):
                solve(bad)
        bad = deepcopy(document)
        bad['areas'][0]['item_constraints']['chair'] = {}
        self.assertFalse(validator.is_valid(bad))
        with self.assertRaises(ValueError):
            solve(bad)

    def test_default_furniture_and_canonical_identity_survive(self):
        area = bedroom(7)
        original = RoomPlan.compile(intent(area))
        area['template']['furnishings'] = 'bed_chair_table'
        self.assertEqual(RoomPlan.compile(intent(area)).encode(), original.encode())
        area['template']['furnishings'] = 'bed_only'
        basic = RoomPlan.compile(intent(area))
        self.assertNotEqual(basic.digest, original.digest)
        self.assertEqual(mask(basic), mask(original))
        self.assertEqual((len(basic.request().slots), len(original.request().slots)), (7, 21))

    def test_original_room_roundtrips_through_inventory_cli(self):
        area = private(7, (1, 2))
        area['item_constraints'] = {'bed': {'material': [7, -1]}}
        with TemporaryDirectory() as directory:
            source = Path(directory) / 'rooms.json'
            source.write_bytes(canonical(intent(area)))
            compiled = invoke('compile', '--request-file', str(source))
            self.assertEqual(compiled.returncode, 0, compiled.stdout + compiled.stderr)
            room = RoomPlan.decode(compiled.stdout)
            source.write_bytes(compiled.stdout)
            with Peer(inventory_capture(room.request())) as peer:
                result = invoke('allocate', '--plan-file', str(source), '--emit', 'room-handoff',
                                env=peer.environment())
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(peer.methods, ['Handshake', 'ReadObservation'])
            handoff = RoomFurnitureHandoff.decode(result.stdout)
            self.assertEqual(handoff.room_plan, room)
            self.assertEqual([s.kind for s in handoff.allocation.plan().steps], ['bed'] * 7)
