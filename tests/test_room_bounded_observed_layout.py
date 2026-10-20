"""Bounded routing from original map bytes through retained excavation handoffs."""
from __future__ import annotations

from copy import deepcopy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from test_room_bounded_layout import ROOT, large_request, targets, original_plan, protected
from test_room_cost_workflow import MANIFEST, capture, layout_request
from test_room_cost_routing_cli import Peer, environment, TOKEN
from furniture_plan import canonical
from room_excavation_handoff import RoomExcavationHandoff
from room_provisioning import RoomPlan
import room_layout as layout
import room_layout_observed as observed
import room_layout_excavation as excavation
import room_bounded_cost_paths as paths
import survey_room_layout as command
import layout_rooms as offline


def observed_large():
    value = large_request()
    costs = value.pop('excavation_costs')
    value['objective'] = observed.EXCAVATION_OBJECTIVE
    value['residual_partition'] = 'bounded-original-dig/1'
    low, size = value['region']['origin'], value['region']['size']
    free = {(low[0] + i % size[0], low[1] + i // size[0], low[2])
            for i, cell in enumerate(costs) if cell == '0'}
    return value, free


def source(value, free, *, padded=False, changes=None):
    region = (excavation.prepare(canonical(value))[1].json() if padded else value['region'])
    return capture({'region': region}, free=free, folder=value['world_folder'],
                   site=value['site'], changes=changes)


def small_observed():
    value = layout_request(observed_costs=True)
    value['connection_strategy'] = layout.BOUNDED_COST_CONNECTION_STRATEGY
    return value


class ObservedBoundedTests(unittest.TestCase):
    def test_original_wall_counts_improve_without_losing_rooms_or_floor_targets(self):
        value, free = observed_large(); original = deepcopy(value)
        raw = source(value, free)
        old = observed.solve(canonical({**value, 'connection_strategy': layout.COST_CONNECTION_STRATEGY}), raw, MANIFEST)
        result = observed.solve(canonical(value), raw, MANIFEST); body = result.json()
        self.assertEqual(value, original)
        self.assertEqual(old.json()['excavation_evidence']['observed_wall_targets'], 5)
        self.assertEqual(body['excavation_evidence']['observed_wall_targets'], 1)
        self.assertEqual(body['excavation_evidence']['observed_floor_targets'], 511)
        self.assertEqual(body['layout']['room_plan']['furniture_count'], 28)
        self.assertTrue(body['excavation_evidence']['corridor_cost_routing_enabled'])
        self.assertTrue(body['excavation_evidence']['corridors_rerouted_by_cost'])
        self.assertEqual(body['request']['connection_strategy'], layout.BOUNDED_COST_CONNECTION_STRATEGY)
        self.assertEqual(body['layout']['request']['connection_strategy'], layout.BOUNDED_COST_CONNECTION_STRATEGY)
        self.assertEqual(observed.ObservedLayout.decode(result.encode()), result)

    def test_preflight_keeps_strategy_but_cannot_accept_caller_costs(self):
        value = small_observed(); normalized, _ = observed.prepare(canonical(value))
        self.assertEqual(normalized['connection_strategy'], value['connection_strategy'])
        self.assertNotIn('excavation_costs', normalized)
        for update in ({'excavation_costs': '0'*65}, {'objective': layout.COST_OBJECTIVE},
                       {'objective': layout.MINIMUM_TARGETS_OBJECTIVE}, {'objective': None}):
            with self.subTest(update=update), self.assertRaises(ValueError):
                observed.prepare(canonical({**value, **update}))

    def test_floor_permission_on_a_raw_wall_cannot_make_the_corridor_free(self):
        value, free = observed_large(); raw = source(value, free)
        initial = observed.solve(canonical(value), raw, MANIFEST).json()
        walls = targets(initial['layout']['room_plan']) - free
        self.assertEqual(len(walls), 1)
        x, y, _ = next(iter(walls)); low = value['region']['origin']; width = value['region']['size'][0]
        index = (y-low[1])*width+x-low[0]
        cells = ['.']*(width*value['region']['size'][1]); cells[index] = 'F'
        value['site_mask'] = {'region': value['region'], 'cells': ''.join(cells)}
        body = observed.solve(canonical(value), raw, MANIFEST).json()
        self.assertEqual(body['layout']['request']['excavation_costs'][index], '1')
        self.assertEqual(body['excavation_evidence']['observed_wall_targets'], 1)

    def test_hidden_liquid_occupied_or_opened_required_wall_cannot_be_used(self):
        value, free = observed_large()
        point = tuple(value['areas'][0]['origin'])
        for change in ({'presence': 1}, {'liquid': 1}, {'building': 1}, {'dig': 1}):
            with self.subTest(change=change):
                body = observed.solve(canonical(value), source(value, free, changes={point: change}), MANIFEST).json()
                self.assertEqual(body['status'], 'no_layout_in_domain')
                self.assertIsNone(body['layout']['room_plan'])
                self.assertIsNone(body['excavation_evidence']['observed_wall_targets'])
        body = original_plan(large_request())
        wall = min(protected(body) - targets(body))
        result = observed.solve(canonical(value), source(value, free, changes={wall: {'shape': 3}}), MANIFEST)
        self.assertEqual(result.json()['status'], 'no_layout_in_domain')

    def test_padded_capture_produces_one_wall_handoff_with_the_complete_original_plan(self):
        value, free = observed_large(); raw = source(value, free, padded=True)
        result, handoff = excavation.solve(canonical(value), raw, MANIFEST, '127.0.0.1:5000')
        body = result.json()
        self.assertEqual(body['status'], 'excavation_proposed')
        self.assertEqual(body['terrain_survey']['target_counts']['remaining_wall'], 1)
        self.assertEqual(body['terrain_survey']['coverage']['target_tiles'], 512)
        self.assertEqual(body['layout']['room_plan']['furniture_count'], 28)
        self.assertTrue(body['single_source_capture'])
        self.assertFalse(body['additional_native_read'])
        self.assertEqual(handoff.plan().json(), body['layout']['room_plan'])
        self.assertEqual(handoff.plan().json()['intent']['corridor_costs']['strategy'], layout.BOUNDED_COST_CONNECTION_STRATEGY)
        self.assertEqual(RoomExcavationHandoff.decode(handoff.encode()), handoff)
        self.assertEqual(excavation.replay(result.encode()), (result, handoff))
        self.assertEqual(body['layout_evidence']['residual_capacity_evidence']['remaining_mask_digest'],
                         handoff.json()['remaining_mask_digest'])

    def test_original_entrance_and_vertical_item_constraints_survive_observed_lowering(self):
        value = small_observed()
        value['access_point'] = [10, 10, 2]
        value['areas'][0]['item_constraints'] = {'chair': {'max_vertical_distance': 2, 'max_distance': 15}}
        result = observed.solve(canonical(value), capture(), MANIFEST).json()
        plan = RoomPlan.decode(canonical(result['layout']['room_plan']))
        self.assertEqual(plan.json()['intent']['access']['point'], value['access_point'])
        self.assertTrue(result['access_evidence']['point_floor_observed'])
        self.assertEqual(next(s for s in plan.request().slots if s.name == 'a.001.chair').max_vertical_distance, 2)
        self.assertEqual(next(s for s in plan.request().slots if s.name == 'a.001.chair').max_distance, 15)

    def test_label_exhaustion_after_baseline_propagates_through_observation(self):
        with patch.object(paths, 'MAX_LABELS', 1), self.assertRaises(layout.WorkLimit):
            observed.solve(canonical(small_observed()), capture(), MANIFEST)

    def test_rehashed_capture_cost_and_strategy_claims_still_require_regeneration(self):
        result = observed.solve(canonical(small_observed()), capture(), MANIFEST)
        for target in ('request', 'intent', 'evidence'):
            body = result.json(); body.pop('survey_digest')
            if target == 'request':
                body['request']['connection_strategy'] = layout.COST_CONNECTION_STRATEGY
            elif target == 'intent':
                body['layout']['room_plan']['intent']['corridor_costs'].pop('strategy')
            else:
                body['excavation_evidence']['observed_wall_targets'] = 99
            with self.subTest(target=target), self.assertRaises(ValueError):
                observed.ObservedLayout.decode(observed.ObservedLayout(canonical(body)).encode())

    def test_zero_cost_and_unavailable_results_do_not_grant_native_authority(self):
        body = observed.solve(canonical(small_observed()), capture(), MANIFEST).json()
        self.assertEqual(body['excavation_evidence']['observed_wall_targets'], 0)
        self.assertFalse(body['native_mining_eligibility_proven'])
        self.assertFalse(body['mutation_authority_granted'])
        self.assertFalse(body['production_admitted'])
        self.assertFalse(body['layout']['room_plan']['project_connections']['global_minimum_excavation_proven'])


class BoundedObservedCLITests(unittest.TestCase):
    def process(self, raw, source_option, kind, address=None, strict=True):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'input.json'; path.write_bytes(raw)
            argv = [sys.executable, '-W', 'error', str(ROOT/'scripts/survey_room_layout.py'),
                    source_option, str(path), '--emit', kind]
            if strict: argv.append('--require-optimal-layout')
            run = subprocess.run(argv, capture_output=True, timeout=15, env=environment(address))
            self.assertEqual(run.stderr, b'')
            return run

    def local(self, raw, address=None, *, kind='survey', publisher_status=0):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'input.json'; path.write_bytes(raw); output = []
            with patch.dict(os.environ, environment(address), clear=True), \
                    patch.object(offline, 'publish', side_effect=lambda data: output.append(data) or publisher_status):
                code = command.main(['--request-file', str(path), '--emit', kind])
            self.assertEqual(len(output), 1)
            return code, output[0]

    def test_all_seven_live_exports_use_the_actual_client_and_bounded_connector(self):
        request = canonical(small_observed()); expected = observed.solve(request, capture(), MANIFEST)
        for kind in command.EXPORTS:
            with self.subTest(kind=kind), Peer() as peer:
                run = self.process(request, '--request-file', kind, peer.address)
                self.assertEqual(run.returncode, 0, run.stdout[:300])
                self.assertEqual(run.stdout, command.export(expected, kind, True, require_optimal=True))
                self.assertEqual(peer.calls, ['Handshake', 'ReadObservation'])
                self.assertEqual(peer.connections, 1)
                self.assertTrue(peer.closed.wait(1))

    def test_all_seven_saved_exports_are_offline_and_preserve_exact_artifacts(self):
        result = observed.solve(canonical(small_observed()), capture(), MANIFEST)
        for kind in command.EXPORTS:
            run = self.process(result.encode(), '--survey-file', kind)
            self.assertEqual(run.returncode, 0, run.stdout[:300])
            self.assertEqual(run.stdout, command.export(result, kind, False, require_optimal=True))
        report = json.loads(command.export(result, 'report', False))
        self.assertFalse(report['agent_turn']['coverage']['native_connection_attempted'])
        self.assertEqual(report['agent_turn']['coverage']['native_observations'], 0)

    def test_malformed_strategy_and_caller_costs_refuse_before_contact(self):
        for update in ({'excavation_costs': '0'*65}, {'objective': layout.MINIMUM_TARGETS_OBJECTIVE},
                       {'connection_strategy': 'bounded-cost-with-original-candidate/2'}):
            with self.subTest(update=update), patch.object(command, '_MapClient', side_effect=AssertionError('unexpected contact')):
                code, raw = self.local(canonical({**small_observed(), **update}), '127.0.0.1:5000')
                self.assertEqual(code, 2)
                self.assertIsNone(json.loads(raw)['result'])

    def test_source_failures_do_not_retry_or_publish_cached_candidates(self):
        for mode in ('lost', 'nonce', 'source', 'profile', 'malformed'):
            with self.subTest(mode=mode), Peer(mode) as peer:
                run = self.process(canonical(small_observed()), '--request-file', 'room-plan', peer.address)
                self.assertEqual(run.returncode, 2)
                self.assertIsNone(json.loads(run.stdout)['result'])
                self.assertEqual(peer.connections, 1)
                self.assertNotIn(TOKEN.encode(), run.stdout)

    def test_stream_closes_before_search_and_label_refusal_discards_baseline(self):
        real = observed.solve
        with Peer() as peer:
            def checked(*args):
                self.assertTrue(peer.closed.wait(1))
                return real(*args)
            with patch.object(observed, 'solve', side_effect=checked), patch.object(paths, 'MAX_LABELS', 1):
                code, raw = self.local(canonical(small_observed()), peer.address)
            self.assertEqual(code, 2)
            self.assertIsNone(json.loads(raw)['result'])
            self.assertEqual(peer.connections, 1)

    def test_revocation_after_serialization_discards_the_candidate(self):
        real = command.export
        with Peer() as peer:
            def revoked(*args, **kwargs):
                raw = real(*args, **kwargs)
                os.environ[command.TOKEN] = 'different-synthetic-read-credential-000'
                return raw
            with patch.object(command, 'export', side_effect=revoked):
                code, raw = self.local(canonical(small_observed()), peer.address)
            self.assertEqual(code, 2)
            self.assertIsNone(json.loads(raw)['result'])
            self.assertEqual(peer.connections, 1)

    def test_single_output_attempt_does_not_repeat_acquisition(self):
        with Peer() as peer:
            code, _ = self.local(canonical(small_observed()), peer.address, publisher_status=2)
            self.assertEqual(code, 2)
            self.assertEqual(peer.calls, ['Handshake', 'ReadObservation'])
            self.assertEqual(peer.connections, 1)


if __name__ == '__main__':
    unittest.main()
