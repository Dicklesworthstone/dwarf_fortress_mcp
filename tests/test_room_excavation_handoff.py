"""Whole-room handoff tests using raw fixtures and the real map client/CLI.

The joined peer is synthetic; these tests do not execute DFHack or game effects.
"""
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import excavation_observer as e
import room_terrain as terrain
import room_excavation_handoff as h
import survey_rooms as cli
import room_terrain_fixtures as f
from room_terrain_peer import Peer, TOKEN

ROOT = Path(__file__).resolve().parents[1]
ADDRESS = '127.0.0.1:5000'


def environment(address):
    return {'DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5': '1', 'DFMCP_MAP_TOKEN': TOKEN.decode(),
            'DFMCP_MAP_ENDPOINT': address}


def fixture(plan=None, **kwargs):
    plan = plan or f.plan()
    selected = terrain.selection(plan)
    floors = sorted(selected.floors, key=terrain.order)
    already_dug = set(floors[::3])
    raw = f.capture(selected.region, {p: f.visible(3) for p in already_dug}, **kwargs)
    return plan, selected, raw, already_dug


def rehash(value):
    value = copy.deepcopy(value)
    value.pop('handoff_digest', None)
    return h.canonical({**value, 'handoff_digest': hashlib.sha256(
        b'dfmcp-room-excavation-handoff/1\0' + h.canonical(value)).hexdigest()})


class HandoffTests(unittest.TestCase):
    def setUp(self):
        self.plan, self.selected, self.raw, self.dug = fixture()
        self.handoff = h.RoomExcavationHandoff.create(self.plan, self.raw, f.MANIFEST, ADDRESS)

    def test_partial_excavation_retains_whole_plan_but_only_remaining_walls(self):
        handoff = h.RoomExcavationHandoff.decode(self.handoff.encode())
        self.assertEqual(handoff.plan().encode(), self.plan.encode())
        self.assertEqual(bytes.fromhex(handoff.json()['capture_hex']), self.raw)
        self.assertEqual(f.expand(handoff.json()['remaining_blueprint']), self.selected.floors - self.dug)
        self.assertEqual(handoff.summary()['room_plan']['excavation_blueprint'], self.plan.json()['excavation_blueprint'])
        self.assertFalse(handoff.summary()['room_completion_proven'])
        self.assertFalse(handoff.summary()['replacement_effect_key_authorized'])
        self.assertEqual(handoff.encode(), self.handoff.encode())

    def test_original_constraints_exclusions_and_all_rooms_survive(self):
        request = f.request(f.bedroom(3), f.dining(2, 2, origin=(23, 10, 2)))
        request['areas'][0]['item_constraints'] = {'bed': {'material': [7, -1], 'max_distance': 11}}
        request['excluded_items'] = [2, 7, 42]
        request['excluded_regions'] = [{'origin': [100, 100, 1], 'size': [2, 2, 1]}]
        plan, selected, raw, dug = fixture(f.RoomPlan.compile(request))
        restored = h.RoomExcavationHandoff.decode(h.RoomExcavationHandoff.create(plan, raw, f.MANIFEST, ADDRESS).encode())
        self.assertEqual(restored.plan().json(), plan.json())
        self.assertEqual(restored.plan().json()['intent']['excluded_items'], [2, 7, 42])
        self.assertEqual(f.expand(restored.json()['remaining_blueprint']), selected.floors - dug)

    def test_recomputed_checksum_cannot_change_derived_residual_or_survey(self):
        for field, substitute in (('remaining_mask_digest', '0' * 64), ('survey_digest', '1' * 64),
                                  ('remaining_blueprint', self.plan.json()['excavation_blueprint'])):
            value = self.handoff.json()
            value[field] = substitute
            with self.subTest(field=field):
                self.assertRaises(ValueError, h.RoomExcavationHandoff.decode, rehash(value))
        value = self.handoff.json()
        value['room_plan']['areas'][0]['entry_point'] = [10, 10, 2]
        self.assertRaises(ValueError, h.RoomExcavationHandoff.decode, rehash(value))

    def test_changed_raw_evidence_must_regenerate_residual(self):
        value = self.handoff.json()
        point = next(iter(self.dug))
        value['capture_hex'] = f.capture(self.selected.region, {p: f.visible(3) for p in self.dug - {point}}).hex()
        self.assertRaises(ValueError, h.RoomExcavationHandoff.decode, rehash(value))

    def test_blocked_or_empty_work_never_produces_executable_handoff(self):
        p = next(iter(self.selected.floors))
        wall = next(iter(self.selected.walls))
        for overrides in ({p: b'\x01'}, {p: b'\x00'}, {p: f.visible(depth=1)},
                          {p: f.visible(dig=1)}, {wall: f.visible(3)},
                          {p: f.visible(3) for p in self.selected.floors}):
            with self.subTest(overrides=len(overrides)):
                raw = f.capture(self.selected.region, overrides)
                self.assertRaises(ValueError, h.RoomExcavationHandoff.create, self.plan, raw, f.MANIFEST, ADDRESS)

    def test_capture_substitution_and_wrong_fortress_rejected(self):
        for kwargs in ({'folder': 'other'}, {'site': 5}, {'dimensions': (10, 10, 10)}):
            raw = f.capture(self.selected.region, **kwargs)
            self.assertRaises(ValueError, h.RoomExcavationHandoff.create, self.plan, raw, f.MANIFEST, ADDRESS)
        for raw in (self.raw[:-1], self.raw + b'x', b''):
            self.assertRaises(ValueError, h.RoomExcavationHandoff.create, self.plan, raw, f.MANIFEST, ADDRESS)

    def test_closed_canonical_codec_and_bounds(self):
        raw = self.handoff.encode()
        invalid = [raw + b'\n', b'x' * (h.MAX_BYTES + 1), b'[' * 13 + b'0' + b']' * 13,
                   raw.replace(b'"policy":', b'"policy":"duplicate","policy":', 1),
                   rehash({**self.handoff.json(), 'extra': 1}),
                   rehash({**self.handoff.json(), 'capture_hex': self.raw.hex().upper()}),
                   rehash({**self.handoff.json(), 'schema': 'wrong'})]
        for value in invalid:
            with self.subTest(size=len(value)):
                self.assertRaises((ValueError, UnicodeError), h.RoomExcavationHandoff.decode, value)

    def test_every_bound_source_component_changes_handoff_identity(self):
        for manifest, address, raw in (
            (e.Manifest(8, 'test-df', 'test-dfhack'), ADDRESS, self.raw),
            (e.Manifest(7, 'other-df', 'test-dfhack'), ADDRESS, self.raw),
            (f.MANIFEST, '127.0.0.1:5001', self.raw),
            (f.MANIFEST, ADDRESS, f.capture(self.selected.region, tick=101)),
        ):
            other = h.RoomExcavationHandoff.create(self.plan, raw, manifest, address)
            self.assertNotEqual(other.digest, self.handoff.digest)
        self.assertEqual(self.handoff.source()['game_tick'], 100)

    def test_native_source_checks_software_fortress_endpoint_dimensions_clock_and_pause(self):
        native = {'folder': 'region1', 'site': 2, 'dimensions': [32768] * 3, 'tick': 100, 'paused': True}
        manifest = {'generation': 900, 'df_version': 'test-df', 'dfhack_version': 'test-dfhack'}
        # Independent map/dig generations MUST differ safely; no shared epoch claim.
        self.handoff.check_native_source(ADDRESS, manifest, native)
        for field, value in (('folder', 'other'), ('site', 4), ('dimensions', [100] * 3),
                             ('tick', 99), ('tick', True), ('paused', False), ('paused', 1)):
            with self.subTest(field=field, value=value):
                self.assertRaises(ValueError, self.handoff.check_native_source, ADDRESS, manifest, {**native, field: value})
        for field in ('df_version', 'dfhack_version'):
            self.assertRaises(ValueError, self.handoff.check_native_source, ADDRESS, {**manifest, field: 'changed'}, native)
        self.assertRaises(ValueError, self.handoff.check_native_source, '127.0.0.1:5001', manifest, native)

    def test_summary_is_owned_and_keeps_evidence_without_disclosing_raw(self):
        before = self.handoff.encode()
        summary = self.handoff.summary()
        summary['room_plan']['intent']['areas'].clear()
        value = self.handoff.json()
        value['remaining_blueprint']['parts'].clear()
        self.assertEqual(self.handoff.encode(), before)
        self.assertNotIn('capture_hex', self.handoff.summary())

    def test_interruption_does_not_return_an_artifact(self):
        checks = []
        h.RoomExcavationHandoff.decode(self.handoff.encode(), lambda: checks.append(1))
        class Stop(Exception):
            pass
        for boundary in (0, 1, 10, len(checks) // 2, len(checks) - 1):
            count = [0]
            def guard():
                if count[0] == boundary:
                    raise Stop()
                count[0] += 1
            self.assertRaises(Stop, h.RoomExcavationHandoff.decode, self.handoff.encode(), guard)

    def test_large_sparse_multilevel_and_32_slot_artifacts_are_complete(self):
        intent = f.request(f.bedroom(10), f.dining(1, 1, origin=(26, 10, 2)))
        intent['excluded_items'] = list(range(2147400000, 2147400646))
        for plan in (f.RoomPlan.compile(intent), f.plan(f.dining(1, 1), f.dining(1, 1, origin=(132, 13, 2), name='far')),
                     f.plan(f.dining(1, 1), f.dining(1, 1, origin=(10, 10, 4), name='upper'))):
            selected = terrain.selection(plan)
            raw = f.capture(selected.region)
            value = h.RoomExcavationHandoff.create(plan, raw, f.MANIFEST, ADDRESS)
            restored = h.RoomExcavationHandoff.decode(value.encode())
            self.assertEqual(restored.plan().encode(), plan.encode())
            self.assertEqual(f.expand(restored.json()['remaining_blueprint']), selected.floors)
            self.assertLessEqual(len(value.encode()), h.MAX_BYTES)


class ExportTests(unittest.TestCase):
    def setUp(self):
        self.plan, self.selected, self.raw, self.dug = fixture()

    def call_cli(self, peer, *, emit='excavation-handoff'):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'plan.json'
            path.write_bytes(self.plan.encode())
            result = subprocess.run([sys.executable, str(ROOT / 'scripts/survey_rooms.py'), '--plan-file', str(path),
                                     '--emit', emit], capture_output=True, timeout=10,
                                     env={**{k:v for k,v in os.environ.items() if not k.startswith('DFMCP_')},
                                          **environment(peer.address)})
            self.assertEqual(result.stderr, b'')
            return result

    def test_actual_cli_export_uses_one_original_map_capture(self):
        with Peer(self.raw, self.selected.region) as peer:
            result = self.call_cli(peer)
            self.assertEqual(result.returncode, 0, result.stdout)
            handoff = h.RoomExcavationHandoff.decode(result.stdout)
            self.assertEqual(handoff.plan().encode(), self.plan.encode())
            self.assertEqual(bytes.fromhex(handoff.json()['capture_hex']), self.raw)
            self.assertEqual(handoff.source()['endpoint'], peer.address)
            self.assertEqual((peer.accepted, peer.observations), (1, 1))
            self.assertEqual(peer.binds, ['Handshake', 'ReadObservation'])
            self.assertNotIn(TOKEN, result.stdout)

    def test_existing_default_and_residual_exports_are_byte_unchanged(self):
        for emit in ('survey', 'remaining-blueprint'):
            with Peer(self.raw, self.selected.region) as peer:
                result = self.call_cli(peer, emit=emit)
                survey = terrain.survey(self.plan, self.raw, f.MANIFEST)
                if emit == 'remaining-blueprint':
                    expected = h.canonical(survey['remaining_blueprint'])
                else:
                    packet = cli.packet(survey, peer.address)
                    packet['report_digest'] = hashlib.sha256(b'dfmcp-room-terrain-report/1\0' + h.canonical(packet)).hexdigest()
                    expected = cli.serialize(packet)
                self.assertEqual((result.returncode, result.stdout), (0, expected))
                self.assertEqual(peer.observations, 1)

    def test_empty_blocked_and_lost_reads_never_export_partial_handoff(self):
        for raw, fault in ((f.capture(self.selected.region, {p: f.visible(3) for p in self.selected.floors}), None),
                           (f.capture(self.selected.region, {next(iter(self.selected.walls)): f.visible(3)}), None),
                           (self.raw, 'lost_reply'), (self.raw, 'generation')):
            with Peer(raw, self.selected.region, fault=fault) as peer:
                result = self.call_cli(peer)
                self.assertEqual(result.returncode, 2)
                self.assertFalse(json.loads(result.stdout)['ok'])
                self.assertIsNone(json.loads(result.stdout)['result'])
                self.assertEqual(peer.accepted, 1)

    def test_final_handoff_serialization_revocation_withholds_result(self):
        original = h.RoomExcavationHandoff.encode
        def revoked(value):
            result = original(value)
            os.environ.pop('DFMCP_MAP_TOKEN', None)
            return result
        with Peer(self.raw, self.selected.region) as peer:
            with patch.dict(os.environ, environment(peer.address), clear=True):
                authority = cli.Authority.load()
                with patch.object(h.RoomExcavationHandoff, 'encode', revoked):
                    self.assertRaises(ValueError, cli.run, self.plan, authority, cli.Budget(10000), emit='excavation-handoff')
                self.assertEqual(peer.observations, 1)

    def test_shared_budget_does_not_renew_for_handoff_derivation(self):
        with Peer(self.raw, self.selected.region) as peer:
            with patch.dict(os.environ, environment(peer.address), clear=True):
                budget = cli.Budget(10000)
                output = cli.run(self.plan, cli.Authority.load(), budget, emit='excavation-handoff')
                self.assertGreater(len(output), 0)
                self.assertEqual(budget.calls_left, 0)
                self.assertLess(budget.work_left, cli.MAX_WORK)
                self.assertLess(budget.network_left, e.MAX_WIRE)


if __name__ == '__main__':
    unittest.main()
