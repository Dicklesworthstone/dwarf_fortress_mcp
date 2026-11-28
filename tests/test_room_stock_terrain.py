"""Join a replayed padded map with one actual paged-inventory CLI fixture."""
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from room_layout_capture_fixtures import capture as map_capture, request, tile
from room_inventory_peer import Peer, capture as inventory_capture
from test_plan_rooms import invoke
import excavation_observer as native
from furniture_plan import canonical
from room_excavation_handoff import RoomExcavationHandoff
from room_furniture_handoff import RoomFurnitureHandoff
import room_layout_excavation as excavation
from room_provisioning import INTENT_SCHEMA, RoomPlan


MAP_MANIFEST = native.Manifest(23, 'DF-test', 'DFHack-test')


class StockTerrainTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.path = Path(directory.name, 'excavation-layout.json')
        self.document = request(access=False, fixed=False, size=(8, 3, 1))
        self.document['areas'][0]['item_constraints'] = {
            kind: {'material': [7, -1], 'max_distance': 0} for kind in ('chair', 'table')}
        area = deepcopy(self.document['areas'][0])
        area['origin'] = [14, 10, 2]
        self.slots = RoomPlan.compile({'schema': INTENT_SCHEMA, 'world_folder': 'region1',
                                       'site': 2, 'areas': [area]}).request()
        self.stock = inventory_capture(self.slots)

    def archive(self, endpoint, *, tick=200, floor=False, manifest=MAP_MANIFEST):
        raw_request = canonical(self.document)
        _, padded, _ = excavation.prepare(raw_request)
        raw_map = map_capture(padded, tick=tick, default=tile(3 if floor else 2))
        archive, _ = excavation.solve(raw_request, raw_map, manifest, endpoint)
        self.assertEqual(archive.json()['layout']['room_plan']['intent']['areas'][0]['origin'],
                         [10, 10, 2])
        self.path.write_bytes(archive.encode())
        return archive, raw_map

    def command(self, peer, emit='report'):
        return invoke('site-and-allocate', '--excavation-layout-file', str(self.path),
                      '--emit', emit, env=peer.environment())

    def test_reselects_and_exports_the_same_original_room_for_stock_and_excavation(self):
        # Padding forces multiple native pages even though only two items fit the request.
        with Peer(inventory_capture(self.slots, padding=1700)) as peer:
            archive, raw_map = self.archive(peer.endpoint)
            run = self.command(peer)
            self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
            report = json.loads(run.stdout)
            self.assertEqual((peer.connections, peer.releases), (1, 1))
            self.assertGreater(len(peer.pages), 1)
            self.assertEqual(peer.methods, ['Handshake', 'ReadObservation'])
            furniture = RoomFurnitureHandoff.from_json(report['room_handoff'])
            dig = RoomExcavationHandoff.decode(canonical(report['excavation_handoff']))
            self.assertEqual(furniture.room_plan.request(), self.slots)
            self.assertEqual([s.item for s in furniture.allocation.plan().steps], [1, 2])
            self.assertEqual(furniture.room_plan, dig.plan())
            self.assertNotEqual(dig.plan().digest, archive.json()['layout']['room_plan']['plan_digest'])
            self.assertEqual(dig.plan().json()['intent']['areas'][0]['origin'], [14, 10, 2])
            self.assertEqual(report['map_source']['capture_sha256'], hashlib.sha256(raw_map).hexdigest())
            self.assertEqual(report['terrain_survey']['survey_digest'], dig.json()['survey_digest'])
            self.assertNotIn('room_plan', report['terrain_survey'])
            for emit, expected in [('room-handoff', furniture), ('excavation-handoff', dig)]:
                exported = self.command(peer, emit)
                self.assertEqual(exported.returncode, 0, exported.stdout + exported.stderr)
                self.assertEqual(type(expected).decode(exported.stdout), expected)
            self.assertEqual((peer.connections, peer.releases), (3, 3))

    def test_matching_open_floor_still_exports_exact_furniture_without_a_dig(self):
        self.document['objective'] = 'minimum-observed-excavation/1'
        with Peer(self.stock) as peer:
            self.archive(peer.endpoint, floor=True)
            run = self.command(peer)
            self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
            report = json.loads(run.stdout)
            self.assertEqual(RoomFurnitureHandoff.from_json(report['room_handoff']).room_plan.request(),
                             self.slots)
            self.assertEqual(report['terrain_survey']['status'], 'terrain_shapes_satisfied_at_sample')
            self.assertEqual(report['terrain_survey']['target_counts']['remaining_wall'], 0)
            self.assertEqual(report['optimization']['observed_wall_targets'], 0)
            self.assertEqual(report['optimization']['observed_floor_targets'], 12)
            self.assertEqual(report['optimization']['cost_evidence'], 'replayed_complete_padded_map')
            self.assertIsNone(report['excavation_handoff'])
            self.assertEqual(self.command(peer, 'excavation-handoff').returncode, 2)

    def test_cross_source_drift_and_incomplete_inventory_never_export_either_plan(self):
        for case in ('tick', 'endpoint', 'version', 'folder', 'unpaused', 'lost_release'):
            with self.subTest(case=case):
                raw_stock = inventory_capture(self.slots, folder='another-world') if case == 'folder' else self.stock
                if case == 'unpaused':
                    # Nested jobs/1.2 header: magic, year, year tick, raw pause byte.
                    pause = raw_stock.index(b'DFMJ1200') + 8 + 4 + 4
                    self.assertEqual(raw_stock[pause], 1)
                    raw_stock = raw_stock[:pause] + bytes([0]) + raw_stock[pause+1:]
                with Peer(raw_stock, fault='lost_release' if case == 'lost_release' else None) as peer:
                    endpoint = peer.endpoint
                    if case == 'endpoint':
                        other_port = 1 if peer.address[1] != 1 else 2
                        endpoint = f'127.0.0.1:{other_port}'
                    manifest = native.Manifest(23, 'different-df', 'DFHack-test') if case == 'version' else MAP_MANIFEST
                    self.archive(endpoint, tick=201 if case == 'tick' else 200, manifest=manifest)
                    run = self.command(peer)
                    self.assertEqual(run.returncode, 2, run.stdout + run.stderr)
                    self.assertNotIn(b'"room_handoff":{', run.stdout)
                    self.assertNotIn(b'"excavation_handoff":{', run.stdout)

    def test_rehashed_archive_with_changed_terrain_evidence_refuses(self):
        with Peer(self.stock) as peer:
            archive, _ = self.archive(peer.endpoint)
            body = json.loads(archive._body)
            body['terrain_survey']['target_counts']['remaining_wall'] += 1
            # A valid outer digest cannot vouch for a forged native-derived survey.
            self.path.write_bytes(excavation.ExcavationLayout(canonical(body)).encode())
            run = self.command(peer, 'room-handoff')
            self.assertEqual(run.returncode, 2, run.stdout + run.stderr)
            self.assertNotIn(b'dfmcp.room-furniture-handoff/1', run.stdout)
