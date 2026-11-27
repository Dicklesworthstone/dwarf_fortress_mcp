"""The resource-aware Plan contracts expose the same opt-in site preparation."""
import json
from pathlib import Path
import unittest

from jsonschema import Draft202012Validator


class WorkshopPreparationSchemaTests(unittest.TestCase):
    def test_explicit_excavation_in_every_resource_request(self):
        schemas = Path(__file__).resolve().parents[1] / 'schemas'
        production = Draft202012Validator(json.loads((schemas / 'lab_resource_production.schema.json').read_bytes()))
        rooms = Draft202012Validator(json.loads((schemas / 'lab_resource_room.schema.json').read_bytes()))
        site = {'building': 'workshop:Carpenters', 'location': [2, 6, 10], 'min': [1, 5, 10], 'max': [3, 7, 10]}
        options = {'acquire_resources': {}, 'prerequisites': {'workshops': [site]}}
        quota = {'template': 'production', 'quotas': [{'item': 'CHAIR', 'minimum': 2}], **options}
        room = {'template': 'provisioned_room', 'layout': {'template': 'office_cluster',
                'origin': [8, 6, 10], 'rooms': 2, 'room_size': [3, 3]}, **options}
        access = {'entry': [0, 2, 10], 'region': {'min': [0, 0, 10], 'max': [18, 14, 10]}}
        connected = {'template': 'connected_room', 'room': room, 'access': access}
        for validator, request in ((production, quota), (production, {**quota, 'reserve_until_tick': 2000}),
                                   (rooms, room), (rooms, connected),
                                   (rooms, {**connected, 'planner': 'connected_room_stairs_resources_v1'})):
            site.pop('preparation', None)
            self.assertTrue(validator.is_valid(request))
            for policy in ('mine_rock_v1', 'mine', '', None, True, {}):
                with self.subTest(template=request['template'], policy=policy):
                    site['preparation'] = policy
                    self.assertEqual(validator.is_valid(request), policy == 'mine_rock_v1')
