"""Whole-room siting and allocation against retained operations/1.4 pages."""
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from construction_receipt import Manifest
from furniture_plan import canonical
from room_furniture_handoff import RoomFurnitureHandoff
import room_inventory_layout as siting
from room_inventory_peer import Peer, capture
from room_layout import MINIMUM_TARGETS_OBJECTIVE, WorkLimit
from test_plan_rooms import invoke
from test_room_layout import request, solve


def project():
    return request({'name': 'sleep', 'template': {'kind': 'dormitory', 'bed_count': 2, 'columns': [1, 2]},
                    'item_constraints': {'bed': {'max_distance': 0, 'material': [7, -1]}}},
                   size=(8, 4, 1))


def stock(document):
    fixed = deepcopy(document)
    fixed['areas'][0].update(origin=[14, 10, 2])
    fixed['areas'][0]['template']['columns'] = 1
    return solve(fixed).room_plan().request()


def evaluate(document, raw, guard=lambda: None):
    return siting.evaluate(siting.prepare(canonical(document), guard), raw,
                           Manifest(100, 'test-df', 'test-dfhack'), '127.0.0.1:5000')


class InventoryLayoutTests(unittest.TestCase):
    def test_native_command_selects_a_different_site_and_exports_both_exact_items(self):
        document = project()
        first = solve(document).room_plan()
        wanted = stock(document)
        self.assertNotEqual(first.request(), wanted)
        raw = capture(wanted, padding=1700)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory, 'layout.json')
            path.write_bytes(canonical(document))
            with Peer(raw) as peer:
                run = invoke('site-and-allocate', '--request-file', str(path), env=peer.environment())
                self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
                report = json.loads(run.stdout)
                self.assertEqual((peer.connections, peer.releases), (1, 1))
                self.assertEqual(peer.methods, ['Handshake', 'ReadObservation'])
                self.assertGreater(len(peer.pages), 1)
            self.assertGreater(report['layouts_rejected_for_inventory_shortage'], 0)
            self.assertEqual(report['source']['capture_sha256'], hashlib.sha256(raw).hexdigest())
            self.assertEqual(report['projection']['counts']['unsupported_kind'], 1700)
            selected = RoomFurnitureHandoff.from_json(report['room_handoff'])
            self.assertEqual(selected.room_plan.request(), wanted)
            self.assertEqual([s.item for s in selected.allocation.plan().steps], [1, 2])
            with Peer(raw) as peer:
                run = invoke('site-and-allocate', '--request-file', str(path), '--emit', 'room-handoff',
                             env=peer.environment())
                self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
                self.assertEqual(RoomFurnitureHandoff.decode(run.stdout).room_plan, selected.room_plan)

    def test_proven_domain_shortage_and_node_limit_with_sufficient_stock(self):
        document = project()
        wanted = stock(document)
        raw = capture(wanted, omit=[wanted.slots[1].name])
        report = evaluate(document, raw)
        self.assertEqual(report['status'], 'inventory_shortage_in_domain')
        self.assertIsNone(report['room_handoff'])
        self.assertEqual(report['domain_supply']['shortage'],
                         {'slots': [s.name for s in wanted.slots],
                          'candidate_items': [1], 'missing_at_least': 1})
        self.assertEqual(report['search']['nodes_visited'], 0)
        document['search'] = {'max_nodes': 2}
        limited = evaluate(document, capture(wanted))
        self.assertEqual(limited['status'], 'search_limit_reached')
        self.assertFalse(limited['search']['domain_exhausted'])
        self.assertIsNone(limited['room_handoff'])

    def test_material_competition_is_proven_even_with_enough_beds_in_total(self):
        from dataclasses import replace
        from furniture_allocation import Request
        def sleeping(name, count, material):
            return {'name': name, 'template': {'kind': 'dormitory', 'bed_count': count},
                    'item_constraints': {'bed': {'material': material}}}
        document = request(sleeping('east', 2, [7, -1]), sleeping('west', 1, [8, -1]),
                           size=(22, 16, 1), search={'max_nodes': 1})
        source = solve(request(sleeping('stock', 3, [8, -1]), size=(8, 5, 1))).room_plan().request()
        slots = list(source.slots)
        slots[0] = replace(slots[0], material=(7, -1))
        report = evaluate(document, capture(Request(source.folder, source.site, tuple(slots))))
        supply = report['domain_supply']
        self.assertEqual(report['status'], 'inventory_shortage_in_domain')
        self.assertEqual(report['search']['nodes_visited'], 0)
        self.assertEqual(supply['maximum_assignable_in_relaxed_domain'], 2)
        self.assertEqual(supply['shortage'], {'slots': ['east.001.bed', 'east.002.bed'],
                         'candidate_items': [1], 'missing_at_least': 1})
        self.assertIsNone(report['room_handoff'])

    def test_rectangle_haul_distance_is_a_lower_bound_never_a_placement(self):
        from dataclasses import replace
        from furniture_allocation import Request
        document = request({'name': 'sleep', 'template': {'kind': 'dormitory', 'bed_count': 1},
                            'item_constraints': {'bed': {'max_distance': 1}}}, size=(2, 2, 1))
        source = solve(document).room_plan().request()
        def at(point):
            return capture(Request(source.folder, source.site, (replace(source.slots[0], target=point),)))
        for point in ((13, 11, 2), (11, 11, 3)):
            report = evaluate(document, at(point))
            self.assertEqual(report['status'], 'inventory_shortage_in_domain')
            self.assertEqual(report['domain_supply']['shortage']['candidate_items'], [])
        feasible = evaluate(document, at((12, 11, 2)))
        self.assertIsNone(feasible['domain_supply']['shortage'])
        self.assertIsNotNone(feasible['room_handoff'])
        # The closest tile to this item is in the region, but cannot host this bed.
        blocked = evaluate(document, at((9, 10, 2)))
        self.assertIsNone(blocked['domain_supply']['shortage'])
        self.assertEqual(blocked['status'], 'no_layout_in_domain')
        self.assertIsNone(blocked['room_handoff'])

    def test_ranking_only_considers_complete_inventory_feasible_layouts(self):
        document = project()
        raw = capture(stock(document))
        document['objective'] = MINIMUM_TARGETS_OBJECTIVE
        report = evaluate(document, raw)
        goal = report['optimization']
        self.assertTrue(goal['minimum_in_compiler_domain_proven'])
        self.assertIn('complete_captured_inventory_assignment', goal['scope'])
        room = RoomFurnitureHandoff.from_json(report['room_handoff']).room_plan
        self.assertEqual(room.request(), stock(project()))
        self.assertEqual(goal['selected_plan_digest'], room.digest)
        self.assertEqual(evaluate(document, raw), report)
        document['excluded_items'] = [1]
        self.assertIsNone(evaluate(document, raw)['room_handoff'])

    def test_bad_source_refuses_and_preflight_rejects_route_only_requests_before_contact(self):
        document = project()
        raw = capture(stock(document), folder='another-world')
        with self.assertRaisesRegex(ValueError, 'fortress'):
            evaluate(document, raw)
        with self.assertRaises(ValueError):
            evaluate(document, raw[:-1])
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory, 'layout.json')
            document['areas'][0]['item_constraints']['bed']['max_vertical_distance'] = 1
            path.write_bytes(canonical(document))
            with Peer(raw) as peer:
                run = invoke('site-and-allocate', '--request-file', str(path), env=peer.environment())
                self.assertEqual(run.returncode, 2)
                self.assertEqual(peer.connections, 0)

    def test_budget_exhaustion_cannot_become_a_shortage_or_partial_handoff(self):
        document = project()
        raw = capture(stock(document))
        document['search'] = {'max_work': 1500}
        with self.assertRaises(WorkLimit):
            evaluate(document, raw)
