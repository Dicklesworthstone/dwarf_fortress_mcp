"""Whole-room compilation, allocation, ranked layout and raw-map evidence paths."""
from copy import deepcopy
import hashlib
import itertools
from pathlib import Path
import struct
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
from furniture_plan import canonical
from furniture_allocation import Candidate, allocate
import excavation_observer as native
import room_connections as connections
import room_cost_connections as routing
import room_layout as layout
import room_layout_observed as observed
from room_provisioning import RoomPlan, INTENT_SCHEMA

MANIFEST = native.Manifest(7, 'fixture-df', 'fixture-dfhack')
REGION = {'origin': [10, 10, 2], 'size': [13, 5, 1]}


def room_request():
    return {'schema': INTENT_SCHEMA, 'world_folder': 'fixture', 'site': 1, 'connect_areas': True,
            'areas': [{'name': name, 'origin': [x, 12, 2],
                       'template': {'kind': 'dining_hall', 'table_count': 1}}
                      for name, x in (('a', 10), ('b', 19))]}


def targets(body):
    result = set()
    for part in body['excavation_blueprint']['parts']:
        region = part['region']
        points = set(itertools.product(*(range(a, a+n) for a, n in zip(region['origin'], region['size']))))
        if points & result:
            raise AssertionError('overlapping excavation targets')
        result.update(points)
    return result


def free_cells():
    original = room_request(); original['connect_areas'] = False
    return targets(RoomPlan.compile(original).json()) | {
        (x, y, 2) for y in range(10, 15) for x in range(10, 23)
        if y == 10 or x in (10, 22)}


def costs():
    free = free_cells()
    return {'region': deepcopy(REGION), 'cells': ''.join(
        '0' if (x, y, 2) in free else '1' for y in range(10, 15) for x in range(10, 23))}


def layout_request(*, reroute=True, observed_costs=False):
    value = {key: item for key, item in room_request().items() if key != 'connect_areas'}
    value.update(schema=layout.REQUEST_SCHEMA, region=deepcopy(REGION),
                 objective=observed.EXCAVATION_OBJECTIVE if observed_costs else layout.COST_OBJECTIVE)
    if reroute:
        value['connection_strategy'] = layout.COST_CONNECTION_STRATEGY
    if not observed_costs:
        value['excavation_costs'] = costs()['cells']
    return value


def capture(value=None, *, changes=None, free=None, folder='fixture', site=1, paused=True):
    region = REGION if value is None else value['region']
    changes = changes or {}
    free = free_cells() if free is None else free
    folder = folder.encode()
    origin, size = region['origin'], region['size']
    raw = bytearray(b'DFMM1500' + struct.pack('>IIBIH', 0, 100, int(paused), site, len(folder)) + folder)
    raw += struct.pack('>10I', 128, 128, 8, *origin, *size, size[0]*size[1]*size[2])
    for z in range(origin[2], origin[2]+size[2]):
        for y in range(origin[1], origin[1]+size[1]):
            for x in range(origin[0], origin[0]+size[0]):
                row = {'presence': 2, 'shape': 3 if (x,y,z) in free else 2,
                       'liquid': 0, 'magma': 0, 'dig': 0, 'building': 0, 'units': 0}
                row.update(changes.get((x,y,z), {}))
                raw.append(row['presence'])
                if row['presence'] == 2:
                    raw.extend(struct.pack('>IBBBBBBBIHH', 1, row['shape'], row['liquid'], row['magma'],
                        0, row['dig'], row['building'], row['units'], 1, 10000, 10000))
    return bytes(raw)


class RoomCostWorkflowTests(unittest.TestCase):
    def test_complete_room_reuses_open_passage_instead_of_five_wall_targets(self):
        value = room_request()
        baseline = RoomPlan.compile(value)
        value['corridor_costs'] = costs()
        original_input = deepcopy(value)
        plan = RoomPlan.compile(value)
        self.assertEqual(value, original_input)
        self.assertEqual(plan.request(), baseline.request())
        self.assertEqual(plan.json()['areas'], baseline.json()['areas'])
        self.assertEqual(plan.json()['project_connections']['baseline_corridor_cost'], 5)
        self.assertEqual(plan.json()['project_connections']['selected_corridor_cost'], 0)
        self.assertEqual(plan.json()['target_tiles'], 39)
        self.assertTrue(targets(plan.json()) <= free_cells())
        self.assertEqual(RoomPlan.decode(plan.encode()), plan)
        self.assertEqual(plan.json()['intent']['corridor_costs'], costs())

    def test_capture_mask_uses_tight_final_hull_after_outside_room_detour(self):
        plan = RoomPlan.compile({**room_request(), 'corridor_costs': costs()}).json()
        self.assertEqual(plan['capture_region'], REGION)
        low, extent = REGION['origin'], REGION['size']
        mask = {'region': REGION, 'targets': [
            (((z-low[2])*extent[1]+y-low[1])*extent[0]+x-low[0], 3)
            for x,y,z in sorted(targets(plan), key=lambda p: p[::-1])]}
        digest = hashlib.sha256(b'dfmcp-excavation-blueprint/1\0' + canonical(mask)).hexdigest()
        self.assertEqual(digest, plan['excavation_mask_digest'])

    def test_existing_allocator_retains_all_original_items_and_shortage_is_whole(self):
        desired = RoomPlan.compile({**room_request(), 'corridor_costs': costs()}).request()
        candidates = tuple(Candidate(i, slot.kind, slot.target, (0,1), -1) for i,slot in enumerate(desired.slots))
        result = allocate(desired, candidates)
        self.assertEqual(result['status'], 'allocated')
        self.assertEqual(len(result['plan']['steps']), 4)
        self.assertEqual(result['total_distance'], 0)
        shortage = allocate(desired, candidates[:-1])
        self.assertEqual(shortage['status'], 'shortage')
        self.assertIsNone(shortage['plan'])
        self.assertEqual(shortage['assignments'], [])

    def test_fixed_entrance_and_cost_domain_must_agree(self):
        value = {**room_request(), 'corridor_costs': costs(),
                 'access': {'point': [10,10,2], 'region': deepcopy(REGION)}}
        plan = RoomPlan.compile(value)
        self.assertEqual(plan.json()['project_connections']['access']['point'], [10,10,2])
        self.assertIn((10,10,2), targets(plan.json()))
        self.assertEqual(RoomPlan.decode(plan.encode()), plan)
        value['access']['region']['size'][0] -= 1
        with self.assertRaisesRegex(ValueError, 'same|share'):
            RoomPlan.compile(value)

    def test_exclusions_and_forbidden_sites_cannot_become_free_passages(self):
        for use_site in (False, True):
            value = {**room_request(), 'corridor_costs': costs()}
            blocked = (16,10,2)
            if use_site:
                cells = ['.'] * 65; cells[6] = 'X'
                value['site_mask'] = {'region': deepcopy(REGION), 'cells': ''.join(cells)}
            else:
                value['excluded_regions'] = [{'origin': list(blocked), 'size': [1,1,1]}]
            plan = RoomPlan.compile(value).json()
            self.assertNotIn(blocked, targets(plan))
            self.assertGreater(plan['project_connections']['selected_corridor_cost'], 0)

    def test_bedroom_wall_ring_and_cross_level_selectors_remain_original(self):
        for angle in (0,90,180,270):
            value = {'schema': INTENT_SCHEMA, 'world_folder': 'fixture', 'site': 1,
                'connect_areas': True, 'areas': [{'name': 'bedrooms', 'origin': [50,50,2], 'rotation': angle,
                    'template': {'kind': 'bedroom_cluster', 'rooms_count': 2, 'room_size': [3,3]},
                    'item_constraints': {'bed': {'max_vertical_distance': 2, 'material': [1,2]}}}],
                'corridor_costs': {'region': {'origin':[35,35,2], 'size':[32,32,1]}, 'cells':'0'*1024}}
            plan = RoomPlan.compile(value)
            self.assertEqual(RoomPlan.decode(plan.encode()), plan)
            for slot in plan.request().slots:
                if slot.kind == 'bed':
                    self.assertEqual((slot.max_vertical_distance,slot.material),(2,(1,2)))
            floors = targets(plan.json())
            for unit in plan.json()['areas'][0]['units']:
                x,y,z = unit['origin']; w,h,_ = unit['size']
                walls = {(xx,yy,z) for xx in range(x-1,x+w+1) for yy in range(y-1,y+h+1)
                    if xx in (x-1,x+w) or yy in (y-1,y+h)} - {tuple(unit['doorway'])}
                self.assertFalse(floors & walls)

    def test_all_eight_areas_and_32_furnishings_fit_without_dropping_any(self):
        value = {'schema': INTENT_SCHEMA, 'world_folder':'fixture','site':1,'connect_areas':True,
            'areas':[{'name':str(i),'origin':[10+(i%4)*8,10+(i//4)*4,2],
                      'template':{'kind':'dining_hall','table_count':2}} for i in range(8)],
            'corridor_costs':{'region':{'origin':[10,10,2],'size':[32,10,1]},'cells':'0'*320}}
        plan = RoomPlan.compile(value)
        self.assertEqual(len(plan.request().slots), 32)
        self.assertEqual(len(plan.json()['areas']), 8)
        self.assertEqual(RoomPlan.decode(plan.encode()), plan)

    def test_invalid_cost_scope_and_unconnected_cost_requests_refuse(self):
        value = {**room_request(), 'corridor_costs':costs()}
        for modification in ({'connect_areas':False}, {'corridor_costs':None},
                {'corridor_costs':{'region':{'origin':[10,10,2],'size':[4,4,1]},'cells':'0'*16}}):
            with self.assertRaises(ValueError): RoomPlan.compile({**value, **modification})
        with self.assertRaises(ValueError): RoomPlan.compile(value, _connection_fallback=False)

    def test_rehashed_changed_costs_counts_or_geometry_do_not_pass_replay(self):
        plan = RoomPlan.compile({**room_request(), 'corridor_costs':costs()})
        for field in ('costs','count','geometry'):
            value = plan.json(); value.pop('plan_digest')
            if field == 'costs': value['intent']['corridor_costs']['cells'] = '1'*65
            elif field == 'count': value['project_connections']['selected_corridor_cost'] = 99
            else: value['excavation_blueprint']['parts'][0]['region']['origin'][0] += 1
            forged = RoomPlan(canonical(value))
            with self.assertRaises(ValueError): RoomPlan.decode(forged.encode())

    def test_layout_strategy_is_retained_in_the_complete_selected_plan(self):
        value = layout_request()
        baseline = layout.solve(canonical(layout_request(reroute=False)))
        selected = layout.solve(canonical(value))
        self.assertEqual(selected.json()['optimization']['score']['excavation_cost'], 0)
        self.assertEqual(baseline.json()['optimization']['score']['excavation_cost'], 5)
        self.assertEqual(selected.room_plan().request(), baseline.room_plan().request())
        self.assertEqual(layout.Layout.decode(selected.encode()), selected)
        self.assertEqual(selected.json()['request']['connection_strategy'], routing.STRATEGY)
        self.assertIn('corridor_costs', selected.room_plan().json()['intent'])

    def test_new_strategy_requires_explicit_cost_objective_before_any_search(self):
        value = layout_request()
        for objective in (None, layout.MINIMUM_TARGETS_OBJECTIVE, 'unknown', True):
            bad = deepcopy(value)
            if objective is None: bad.pop('objective'); bad.pop('excavation_costs')
            else: bad['objective'] = objective
            with self.assertRaises(ValueError): layout.solve(canonical(bad))
        bad = layout_request(observed_costs=True)
        bad['objective'] = layout.MINIMUM_TARGETS_OBJECTIVE
        with self.assertRaises(ValueError): observed.prepare(canonical(bad))

    def test_raw_map_costs_reroute_around_walls_and_are_recounted(self):
        value = layout_request(observed_costs=True)
        payload = capture()
        result = observed.solve(canonical(value), payload, MANIFEST)
        evidence = result.json()['excavation_evidence']
        self.assertEqual(evidence['observed_wall_targets'], 0)
        self.assertEqual(evidence['observed_floor_targets'], 39)
        self.assertTrue(evidence['corridors_rerouted_by_cost'])
        self.assertTrue(evidence['corridor_cost_routing_enabled'])
        self.assertEqual(evidence['map_witness'], native.decode_capture(payload, MANIFEST,
                         native.Region.from_json(REGION)).witness)
        self.assertEqual(observed.ObservedLayout.decode(result.encode()), result)
        self.assertEqual(layout.Layout.decode(canonical(result.json()['layout'])).room_plan().json(),
                         result.json()['layout']['room_plan'])

    def test_caller_floor_permissions_cannot_turn_observed_walls_into_free_cells(self):
        value = layout_request(observed_costs=True)
        value['site_mask'] = {'region':deepcopy(REGION), 'cells':'F'*65}
        result = observed.solve(canonical(value), capture(free=set()), MANIFEST)
        evidence = result.json()['excavation_evidence']
        self.assertEqual(evidence['observed_wall_targets'], evidence['selected_target_tiles'])
        self.assertEqual(evidence['observed_floor_targets'], 0)
        self.assertGreater(evidence['observed_wall_targets'], 0)
        for field in ('excavation_costs','corridor_costs'):
            with self.assertRaises(ValueError):
                observed.prepare(canonical({**value,field:'0'*65}))

    def test_hidden_wet_occupied_or_designated_free_route_cells_are_excluded(self):
        value = layout_request(observed_costs=True)
        point = (16,10,2)
        for row in ({'presence':1},{'presence':0},{'liquid':1},{'magma':1},
                    {'building':1},{'units':1},{'dig':1},{'shape':4}):
            with self.subTest(row=row):
                result = observed.solve(canonical(value), capture(changes={point:row}), MANIFEST).json()
                self.assertNotIn(point, targets(result['layout']['room_plan']))
                self.assertGreater(result['excavation_evidence']['observed_wall_targets'], 0)

    def test_budgets_and_cancellation_propagate_without_an_incumbent(self):
        value = layout_request()
        result = layout.solve(canonical(value))
        used = result.json()['search']['work_units']
        with self.assertRaises(layout.WorkLimit):
            layout.solve(canonical({**value,'search':{'max_work':used-1}}))
        exact = layout.solve(canonical({**value,'search':{'max_work':used}}))
        self.assertEqual(exact.json()['status'],'layout_found')
        original_connect = routing.connect
        def reduced(*args, **kwargs):
            return original_connect(*args, **kwargs, max_work=500)
        with patch.object(routing, 'connect', side_effect=reduced):
            with self.assertRaises(layout.WorkLimit): layout.solve(canonical(value))
        def cancel(): raise InterruptedError('cancelled')
        with self.assertRaises(InterruptedError): layout.solve(canonical(value), cancel)
        with self.assertRaises(InterruptedError): observed.solve(canonical(layout_request(observed_costs=True)),capture(),MANIFEST,cancel)

    def test_node_budget_returns_no_partial_rooms_and_no_false_optimality(self):
        value = layout_request()
        for maximum in (1,2):
            result = layout.solve(canonical({**value,'search':{'max_nodes':maximum}})).json()
            self.assertEqual(result['status'],'search_limit_reached')
            self.assertIsNone(result['room_plan'])
            self.assertFalse(result['optimization']['minimum_in_compiler_domain_proven'])
        value['areas'][1].pop('origin')
        result = layout.solve(canonical({**value,'search':{'max_nodes':3}})).json()
        if result['room_plan'] is not None:
            self.assertEqual(len(result['room_plan']['areas']), 2)
            self.assertFalse(result['optimization']['minimum_in_compiler_domain_proven'])

    def test_observed_source_and_saved_evidence_tampering_refuse(self):
        value = layout_request(observed_costs=True)
        for options in ({'folder':'other'},{'site':2},{'paused':False}):
            with self.assertRaises(ValueError): observed.solve(canonical(value),capture(**options),MANIFEST)
        result = observed.solve(canonical(value),capture(),MANIFEST)
        body = result.json();body.pop('survey_digest');body['excavation_evidence']['observed_wall_targets']=7
        with self.assertRaises(ValueError): observed.ObservedLayout.decode(observed.ObservedLayout(canonical(body)).encode())


if __name__ == '__main__':
    unittest.main()
