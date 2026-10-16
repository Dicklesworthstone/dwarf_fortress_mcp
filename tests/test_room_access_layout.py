"""Entrance-bound search and original map/1.5 evidence, without layout doubles."""
from __future__ import annotations

from copy import deepcopy
import hashlib
import io
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1] / 'scripts'
sys.path.insert(0, str(SCRIPTS))
import excavation_observer as native
import room_connections as connections
import room_layout as layout
import room_layout_observed as observed
import survey_room_layout as survey
from furniture_plan import canonical
from room_provisioning import RoomPlan
from test_room_access import flood, mask

MANIFEST = native.Manifest(7, 'df-fixture', 'dfhack-fixture')


def request(*, fixed=True):
    area = {'name': 'hall', 'template': {'kind': 'dining_hall', 'table_count': 1}}
    if fixed:
        area['origin'] = [16, 10, 2]
    return {'schema': layout.REQUEST_SCHEMA, 'world_folder': 'region1', 'site': 2,
            'region': {'origin': [10, 10, 2], 'size': [10, 6, 1]},
            'areas': [area], 'access_point': [10, 10, 2]}


def tile(*, presence=2, shape=2, depth=0, magma=0, dig=0, building=0, units=0):
    return bytes([presence]) + (struct.pack('>IBBBBBBBIHH', 1, shape, depth, magma, 0,
                                         dig, building, units, 1, 10000, 10000) if presence == 2 else b'')


def payload(value, overrides=None, *, paused=True, site=2):
    region = native.Region.from_json(value['region'])
    folder = value['world_folder'].encode()
    out = b'DFMM1500' + struct.pack('>IIBIH', 0, 100, int(paused), site, len(folder)) + folder
    out += struct.pack('>10I', 128, 128, 8, *region.origin, *region.size, region.volume)
    changes = {tuple(value.get('access_point', region.origin)): tile(shape=3)}
    changes.update(overrides or {})
    x, y, z = region.origin; width, height, _ = region.size
    for yy in range(y, y+height):
        for xx in range(x, x+width):
            out += changes.get((xx, yy, z), tile())
    return out


class LayoutAccessTests(unittest.TestCase):
    def check(self, value):
        result = layout.solve(canonical(value))
        self.assertEqual(result.json()['status'], 'layout_found')
        self.assertEqual(layout.Layout.decode(result.encode()), result)
        plan = result.room_plan()
        root = tuple(value['access_point'])
        walkable = mask(plan) - {s.target for s in plan.request().slots}
        self.assertEqual(flood(walkable, root), walkable)
        self.assertEqual(plan.json()['intent']['access'], {'point': value['access_point'], 'region': value['region']})
        self.assertEqual(result.json()['policy'], layout.ACCESS_POLICY)
        return result

    def test_search_routes_to_fixed_entrance_not_just_between_rooms(self):
        self.check(request())

    def test_entrance_under_first_candidate_furniture_causes_backtracking(self):
        value = request(fixed=False); value['access_point'] = [11, 11, 2]
        result = self.check(value)
        self.assertGreater(result.json()['search']['compiler_refusals'], 0)
        self.assertNotEqual(result.room_plan().json()['intent']['areas'][0]['origin'], [10, 10, 2])

    def test_detour_uses_declared_domain_and_keeps_exclusions(self):
        value = request(); value['excluded_regions'] = [{'origin': [14, 10, 2], 'size': [1, 4, 1]}]
        self.assertTrue(any(y == 14 for _, y, _ in mask(self.check(value).room_plan())))
        value['excluded_regions'][0]['size'][1] = 6
        out = layout.solve(canonical(value)).json()
        self.assertEqual(out['status'], 'no_layout_in_domain')
        self.assertIsNone(out['room_plan'])
        self.assertTrue(out['search']['domain_exhausted'])

    def test_orientation_search_and_entrance_constraints_are_joint(self):
        value = request(fixed=False)
        value['region']['size'] = [4, 13, 1]
        value['areas'][0]['template'].update(table_count=4, columns=4)
        value['areas'][0]['rotations'] = [0, 90, 180, 270]
        plan = self.check(value).room_plan()
        self.assertIn(plan.json()['intent']['areas'][0]['rotation'], (90, 270))
        self.assertEqual(len(plan.request().slots), 8)

    def test_malformed_or_excluded_access_rejected_during_preflight(self):
        cases = []
        for point in (None, [True,10,2], [9,10,2], [10,10,3], [20,10,2], [10.0,10,2]):
            value = request(); value['access_point'] = point; cases.append(value)
        value = request(); value['excluded_regions'] = [{'origin':[10,10,2], 'size':[1,1,1]}]; cases.append(value)
        value = request(); value['site_mask'] = {'region':value['region'], 'cells':'X'+'.'*59}; cases.append(value)
        for value in cases:
            with self.subTest(value=value), self.assertRaises(ValueError):
                observed.prepare(canonical(value))

    def test_budget_exhaustion_is_inconclusive_and_never_a_partial_plan(self):
        value = request(); value['search'] = {'max_nodes': 1}
        out = layout.solve(canonical(value)).json()
        self.assertEqual(out['status'], 'search_limit_reached')
        self.assertFalse(out['search']['domain_exhausted']); self.assertIsNone(out['room_plan'])
        value['search'] = {'max_work': 10}
        with self.assertRaises(layout.WorkLimit): layout.solve(canonical(value))

    def test_router_work_exhaustion_cannot_be_reported_as_no_layout(self):
        # Fault the existing budget object, not the solver/compiler result.
        original = connections._Work.__init__
        def small(self, guard, maximum): original(self, guard, 5)
        with patch.object(connections._Work, '__init__', small):
            with self.assertRaises(connections.WorkLimit):
                connections.connect({(10,10,2)}, set(), (('a',(10,10,2)),),
                                    ((10,10,2),(1,1,1)), max_new_tiles=0)
            with self.assertRaises(layout.WorkLimit): layout.solve(canonical(request()))

    def test_replay_binds_entrance_even_when_no_route_changes(self):
        result = self.check(request())
        for field in ('request', 'lowered', 'witness'):
            value = result.json()
            if field == 'request': value['request']['access_point'][0] += 1
            elif field == 'lowered': value['room_plan']['intent']['access']['point'][0] += 1
            else: value['room_plan']['project_connections']['access']['point'][0] += 1
            with self.subTest(field=field), self.assertRaises(ValueError):
                layout.Layout.decode(canonical(value))

    def test_offline_cli_exports_exact_entrance_bound_artifacts(self):
        value = request(); result = self.check(value)
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)/'input.json'; source.write_bytes(canonical(value))
            for kind, expected in (('layout', result.encode()), ('room-plan', result.room_plan().encode())):
                run = subprocess.run([sys.executable, str(SCRIPTS/'layout_rooms.py'), '--request-file',
                                      str(source), '--emit', kind], capture_output=True, check=True, timeout=10)
                self.assertEqual(run.stdout, expected); self.assertEqual(run.stderr, b'')


class ObservedAccessTests(unittest.TestCase):
    def test_entrance_is_observed_in_the_same_original_map_capture(self):
        value = request(); raw = payload(value)
        out = observed.solve(canonical(value), raw, MANIFEST)
        self.assertEqual(observed.ObservedLayout.decode(out.encode()), out)
        body = out.json(); evidence = body['access_evidence']
        self.assertEqual(body['status'], 'layout_found')
        self.assertEqual(evidence['point'], value['access_point'])
        self.assertTrue(evidence['point_floor_observed']); self.assertTrue(evidence['same_capture_as_layout'])
        self.assertEqual(evidence['map_witness'], body['source']['map_witness'])
        self.assertEqual(evidence['game_tick'], 100)
        self.assertFalse(evidence['point_is_existing_fortress_entrance_proven'])
        self.assertFalse(evidence['current_access_proven'])
        # The separate regenerated geometry artifact is not relabelled observed.
        self.assertFalse(body['layout']['room_plan']['project_connections']['access']['point_floor_observed'])

    def test_caller_floor_permission_never_promotes_wall_to_observed_entrance(self):
        value = request(); value['site_mask'] = {'region':deepcopy(value['region']), 'cells':'F'+'.'*59}
        raw = payload(value, {(10,10,2):tile(shape=2)})
        with self.assertRaisesRegex(ValueError, 'must be observed'):
            observed.solve(canonical(value), raw, MANIFEST)

    def test_unknown_wet_designated_occupied_and_nonfloor_entrances_refuse(self):
        bad = [tile(presence=0), tile(presence=1), tile(shape=2), tile(shape=1), tile(shape=8),
               tile(shape=3, depth=1), tile(shape=3, magma=1), tile(shape=3, dig=1),
               tile(shape=3, building=1), tile(shape=3, units=1)]
        for row in bad:
            value = request()
            with self.subTest(tile=row.hex()), self.assertRaisesRegex(ValueError, 'must be observed'):
                observed.solve(canonical(value), payload(value, {(10,10,2):row}), MANIFEST)

    def test_access_evidence_survives_infeasibility_without_claiming_room_connection(self):
        value = request()
        overrides = {(14,y,2):tile(presence=1) for y in range(10,16)}
        out = observed.solve(canonical(value), payload(value, overrides), MANIFEST).json()
        self.assertTrue(out['access_evidence']['point_floor_observed'])
        self.assertEqual(out['status'], 'no_layout_in_domain')
        self.assertIsNone(out['layout']['room_plan'])

    def test_substituted_source_partial_payload_and_running_world_refuse(self):
        value = request(); raw = payload(value)
        for bad in (raw[:-1], raw+b'\0', payload(value, paused=False), payload(value, site=3), b'bad'):
            with self.subTest(raw=bad[:20]), self.assertRaises(ValueError):
                observed.solve(canonical(value), bad, MANIFEST)

    def test_changed_entrance_fact_or_role_claim_fails_regeneration(self):
        value = request(); out = observed.solve(canonical(value), payload(value), MANIFEST)
        for key in ('point', 'map_witness', 'game_tick', 'point_is_existing_fortress_entrance_proven'):
            body = out.json()
            body['access_evidence'][key] = {'point':[11,10,2], 'map_witness':'0'*64,
                'game_tick':101, 'point_is_existing_fortress_entrance_proven':True}[key]
            with self.subTest(key=key), self.assertRaises(ValueError):
                observed.ObservedLayout.decode(canonical(body))

    def test_interruption_across_preflight_decode_search_and_publish_propagates(self):
        value = request(); raw = payload(value); calls = []
        observed.solve(canonical(value), raw, MANIFEST, lambda:calls.append(1))
        for stop in sorted(set(range(1,len(calls)+1,43)) | {len(calls)}):
            seen = 0
            def guard():
                nonlocal seen
                seen += 1
                if seen == stop: raise InterruptedError('cancelled observed access')
            with self.subTest(stop=stop), self.assertRaisesRegex(InterruptedError, 'cancelled observed access'):
                observed.solve(canonical(value), raw, MANIFEST, guard)

    def test_invalid_point_refuses_before_authority_or_native_contact(self):
        value = request(); value['access_point'] = [0,10,2]
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)/'invalid.json'; source.write_bytes(canonical(value))
            stream = io.TextIOWrapper(io.BytesIO(), encoding='utf-8')
            with patch.object(survey.Authority, 'load', side_effect=AssertionError('preflight was skipped')):
                with patch.object(sys, 'stdout', stream):
                    code = survey.main(['--request-file', str(source)])
            self.assertEqual(code, 2)
            stream.flush(); reply = json.loads(stream.buffer.getvalue())
            self.assertFalse(reply['agent_turn']['coverage']['native_connection_attempted'])
            self.assertIsNone(reply['result'])


if __name__ == '__main__': unittest.main()
