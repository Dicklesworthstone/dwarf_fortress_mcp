"""Actual TCP reader and CLI exports; replay never reconnects or authorizes effects."""
from __future__ import annotations

from copy import deepcopy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from room_layout_capture_fixtures import MANIFEST, capture, request, tile
from room_layout_excavation_peer import Peer, TOKEN
import layout_rooms as offline
import room_layout as layout
import room_layout_excavation as compiler
import room_terrain as terrain
import survey_room_excavation as command
import survey_room_layout as survey
from furniture_plan import canonical
from room_excavation_handoff import RoomExcavationHandoff
from room_provisioning import RoomPlan

ROOT = Path(__file__).resolve().parents[1]


def environment(address=None):
    values = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
    values['PYTHONDONTWRITEBYTECODE'] = '1'
    if address is not None:
        values.update({survey.OPT_IN: '1', survey.TOKEN: TOKEN, survey.ENDPOINT: address})
    return values


class CommandTests(unittest.TestCase):
    def run_cli(self, raw, source='--request-file', emit='report', env=None, script='survey_room_excavation.py'):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'input.json'
            path.write_bytes(raw)
            return subprocess.run([sys.executable, str(ROOT / 'scripts' / script), source, str(path),
                '--emit', emit], capture_output=True, env=environment() if env is None else env, timeout=20)

    def local_call(self, raw, source='--request-file', emit='report', env=None):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'input.json'
            path.write_bytes(raw)
            outputs = []
            with patch.dict(os.environ, environment() if env is None else env, clear=True), \
                    patch.object(offline, 'publish', side_effect=lambda data: outputs.append(data) or 0):
                code = command.main([source, str(path), '--emit', emit])
            self.assertEqual(len(outputs), 1)
            return code, outputs[0]

    def complete(self, value=None):
        value = request() if value is None else value
        _, region, _ = compiler.prepare(canonical(value))
        raw = capture(region, {(10, 10, 2): tile(3)})
        return compiler.solve(canonical(value), raw, MANIFEST, '127.0.0.1:5000')

    def assert_refused(self, process, code=None):
        self.assertEqual(process.returncode, 2, process.stderr.decode())
        self.assertEqual(process.stderr, b'')
        result = json.loads(process.stdout)
        self.assertFalse(result['ok'])
        self.assertIsNone(result['result'])
        self.assertIn('agent_turn', result)
        if code:
            self.assertEqual(result['error']['code'], code)
        self.assertNotIn(TOKEN.encode(), process.stdout)
        self.assertNotIn(b'input.json', process.stdout)
        return result

    def test_actual_single_capture_all_nine_exports_match_real_component_results(self):
        value = request()
        _, region, _ = compiler.prepare(canonical(value))
        for kind in command.EXPORTS:
            with self.subTest(kind=kind), Peer(region) as peer:
                expected, handoff = compiler.solve(canonical(value), peer.payload, MANIFEST, peer.address)
                actual = self.run_cli(canonical(value), emit=kind, env=environment(peer.address))
                self.assertEqual(actual.returncode, 0, actual.stdout[:300])
                self.assertEqual(actual.stderr, b'')
                self.assertEqual(actual.stdout, command.export(expected, handoff, kind, True))
                self.assertEqual(peer.calls, ['Handshake', 'ReadObservation'])
                self.assertEqual(peer.connections, 1)
                self.assertTrue(peer.closed.wait(2))
                if kind == 'excavation-handoff':
                    self.assertEqual(RoomExcavationHandoff.decode(actual.stdout), handoff)
                if kind == 'room-plan':
                    self.assertEqual(RoomPlan.decode(actual.stdout), handoff.plan())

    def test_all_nine_offline_exports_work_without_credentials_or_socket(self):
        result, handoff = self.complete()
        for kind in command.EXPORTS:
            with self.subTest(kind=kind), patch.object(survey, '_MapClient', side_effect=AssertionError('offline contact')):
                code, raw = self.local_call(result.encode(), '--compilation-file', kind)
                self.assertEqual(code, 0)
                self.assertEqual(raw, command.export(result, handoff, kind, False))
                actual = self.run_cli(result.encode(), '--compilation-file', kind)
                self.assertEqual(actual.returncode, 0)
                self.assertEqual(actual.stdout, raw)
        report = json.loads(command.export(result, handoff, 'report', False))
        coverage = report['agent_turn']['coverage']
        self.assertFalse(coverage['native_acquisition_verified_this_call'])
        self.assertFalse(coverage['native_connection_attempted'])
        self.assertEqual(coverage['native_observations'], 0)
        self.assertEqual(coverage['derived_subregions'], 2)

    def test_native_stream_closes_before_real_solver_starts(self):
        value = request()
        _, region, _ = compiler.prepare(canonical(value))
        real = compiler.solve
        with Peer(region) as peer:
            def checked(*args):
                self.assertTrue(peer.closed.wait(2), 'native stream stayed open during planning')
                return real(*args)
            with patch.object(compiler, 'solve', side_effect=checked):
                code, raw = self.local_call(canonical(value), env=environment(peer.address))
            self.assertEqual(code, 0)
            report = json.loads(raw)
            self.assertTrue(report['agent_turn']['coverage']['native_acquisition_verified_this_call'])
            self.assertIsNone(report['agent_turn']['anchor'])
            self.assertEqual(report['agent_turn']['coverage']['native_observations'], 1)

    def test_dropped_invalid_or_confused_native_responses_never_export_handoff(self):
        _, region, _ = compiler.prepare(canonical(request()))
        for mode in ('alias', 'lost', 'nonce', 'protocol', 'generation', 'refused',
                     'malformed', 'truncated', 'notification_limit'):
            with self.subTest(mode=mode), Peer(region, mode=mode) as peer:
                actual = self.run_cli(canonical(request()), emit='excavation-handoff', env=environment(peer.address))
                out = self.assert_refused(actual, 'ROOM_EXCAVATION_REFUSED')
                self.assertTrue(out['agent_turn']['coverage']['native_connection_attempted'])
                self.assertEqual(peer.connections, 1)
                self.assertLessEqual(len(peer.calls), 2)

    def test_foreign_fortress_clock_and_dimensions_fail_after_real_acquisition(self):
        _, region, _ = compiler.prepare(canonical(request()))
        for options in ({'folder': 'other-fort'}, {'site': 3}, {'paused': False}, {'dimensions': (12, 12, 3)}):
            with self.subTest(options=options), Peer(region, capture_options=options) as peer:
                self.assert_refused(self.run_cli(canonical(request()), emit='excavation-handoff',
                                                env=environment(peer.address)))

    def test_native_halo_is_checked_and_alternate_location_is_chosen(self):
        value = request(access=False, size=(10, 3, 1), fixed=False)
        _, region, _ = compiler.prepare(canonical(value))
        with Peer(region, changes={(11, 11, 3): tile(presence=1)}) as peer:
            actual = self.run_cli(canonical(value), emit='excavation-handoff', env=environment(peer.address))
            self.assertEqual(actual.returncode, 0, actual.stdout[:300])
            handoff = RoomExcavationHandoff.decode(actual.stdout)
            self.assertGreaterEqual(handoff.plan().json()['intent']['areas'][0]['origin'][0], 13)

    def test_empty_work_or_unusable_halo_cannot_export_a_replacement_blueprint(self):
        _, region, _ = compiler.prepare(canonical(request()))
        cases = [({'default': tile(3)}, 'ROOM_EXCAVATION_NO_REMAINING_WORK'),
                 ({'changes': {(10, 10, 2): tile(3), (17, 11, 3): tile(presence=1)}}, 'ROOM_EXCAVATION_LAYOUT_NOT_FOUND')]
        for options, code in cases:
            for kind in ('remaining-excavation', 'excavation-handoff'):
                with self.subTest(code=code, kind=kind), Peer(region, **options) as peer:
                    self.assert_refused(self.run_cli(canonical(request()), emit=kind,
                                                    env=environment(peer.address)), code)

    def test_inconclusive_search_has_report_but_no_handoff(self):
        value = request()
        value['search'] = {'max_nodes': 1}
        _, region, _ = compiler.prepare(canonical(value))
        for kind in ('report', 'compilation', 'excavation-handoff'):
            with self.subTest(kind=kind), Peer(region) as peer:
                actual = self.run_cli(canonical(value), emit=kind, env=environment(peer.address))
                if kind == 'excavation-handoff':
                    self.assert_refused(actual, 'ROOM_EXCAVATION_SEARCH_LIMIT')
                else:
                    self.assertEqual(actual.returncode, 0)
                    body = json.loads(actual.stdout)
                    if kind == 'report': body = body['result']
                    self.assertEqual(body['status'], 'search_limit_reached')
                    self.assertIsNone(body['handoff_digest'])

    def test_complete_32_slot_native_pipeline_keeps_every_area(self):
        value = request(access=False, size=(32, 32, 1), fixed=False)
        value['areas'] = [{'name': str(i), 'template': {'kind': 'dining_hall', 'table_count': 2}} for i in range(8)]
        value['search'] = {'max_work': layout.MAX_WORK}
        _, region, _ = compiler.prepare(canonical(value))
        with Peer(region, changes={}) as peer:
            actual = self.run_cli(canonical(value), emit='excavation-handoff', env=environment(peer.address))
            self.assertEqual(actual.returncode, 0, actual.stdout[:300])
            handoff = RoomExcavationHandoff.decode(actual.stdout)
            self.assertEqual(len(handoff.plan().request().slots), 32)
            self.assertEqual(len(handoff.plan().json()['areas']), 8)

    def test_malformed_input_and_unrepresentable_context_refuse_before_contact(self):
        cases = [b'', b'x' * (layout.MAX_INPUT + 1), b'[' * 17 + b']' * 17,
                 canonical({**request(), 'site': True}), canonical({**request(), 'search': {'max_work': 1}}),
                 canonical(request(access=False, size=(128, 8, 1), fixed=False))]
        for raw in cases:
            with self.subTest(raw=raw[:50]), patch.object(survey, '_MapClient', side_effect=AssertionError('premature contact')):
                code, output = self.local_call(raw, env=environment('127.0.0.1:5000'))
                self.assertEqual(code, 2)
                self.assertFalse(json.loads(output)['agent_turn']['coverage']['native_connection_attempted'])

    def test_wrong_or_mixed_authority_refuses_before_contact(self):
        base = environment('127.0.0.1:5000')
        for key, value in ((survey.OPT_IN, 'true'), (survey.TOKEN, 'short'),
                           (survey.ENDPOINT, '10.0.0.1:5000'), ('DFMCP_ADMITTED_BRIDGE_PROTOCOL', '1.0'),
                           ('DFMCP_DIG_TOKEN', TOKEN)):
            with self.subTest(key=key), patch.object(survey, '_MapClient', side_effect=AssertionError('premature contact')):
                code, output = self.local_call(canonical(request()), env={**base, key: value})
                self.assertEqual(code, 2)
                self.assertNotIn(TOKEN.encode(), output)
                self.assertFalse(json.loads(output)['agent_turn']['coverage']['native_connection_attempted'])

    def test_revocation_after_capture_and_after_serialization_discards_outputs(self):
        _, region, _ = compiler.prepare(canonical(request()))
        for boundary in ('after_capture', 'after_serialization'):
            with self.subTest(boundary=boundary), Peer(region) as peer:
                real = compiler.solve if boundary == 'after_capture' else command.export
                def revoked(*args):
                    if boundary == 'after_capture':
                        os.environ[survey.TOKEN] = 'revoked-token-still-long-enough-for-validation'
                        return real(*args)
                    result = real(*args)
                    os.environ[survey.TOKEN] = 'revoked-token-still-long-enough-for-validation'
                    return result
                target, name = (compiler, 'solve') if boundary == 'after_capture' else (command, 'export')
                with patch.object(target, name, side_effect=revoked):
                    code, raw = self.local_call(canonical(request()), emit='excavation-handoff', env=environment(peer.address))
                self.assertEqual(code, 2)
                self.assertIsNone(json.loads(raw)['result'])
                self.assertNotIn(b'"room_plan"', raw)

    def test_expired_outer_deadline_after_capture_does_not_publish(self):
        _, region, _ = compiler.prepare(canonical(request()))
        original = compiler.solve
        with Peer(region) as peer:
            def expired(raw, payload, manifest, address, guard):
                guard.__self__.end = 0
                return original(raw, payload, manifest, address, guard)
            with patch.object(compiler, 'solve', side_effect=expired):
                code, raw = self.local_call(canonical(request()), env=environment(peer.address))
            self.assertEqual(code, 2)
            self.assertEqual(json.loads(raw)['error']['code'], 'ROOM_EXCAVATION_TIMEOUT')

    def test_work_allowance_is_not_renewed_after_capture(self):
        _, region, _ = compiler.prepare(canonical(request()))
        original = compiler.solve
        with Peer(region) as peer:
            def exhausted(raw, payload, manifest, address, guard):
                guard.__self__.left = 0
                return original(raw, payload, manifest, address, guard)
            with patch.object(compiler, 'solve', side_effect=exhausted):
                code, raw = self.local_call(canonical(request()), env=environment(peer.address))
            self.assertEqual(code, 2)
            self.assertEqual(json.loads(raw)['error']['code'], 'ROOM_EXCAVATION_WORK_LIMIT')

    def test_retained_compilation_rehash_does_not_bypass_evidence_replay(self):
        result, _ = self.complete()
        body = result.json(); body.pop('compilation_digest')
        body['terrain_survey']['remaining_wall_targets'].pop()
        forged = compiler.ExcavationLayout(canonical(body)).encode()
        actual = self.run_cli(forged, '--compilation-file', 'excavation-handoff')
        self.assert_refused(actual)

    def test_rehydrated_legacy_terrain_survey_has_valid_original_digest(self):
        result, handoff = self.complete()
        actual = self.run_cli(result.encode(), '--compilation-file', 'terrain-survey')
        self.assertEqual(actual.returncode, 0)
        doc = json.loads(actual.stdout)
        digest = doc.pop('survey_digest')
        self.assertEqual(digest, hashlib.sha256(b'dfmcp-room-terrain-survey/1\0' + canonical(doc)).hexdigest())
        self.assertEqual(doc['room_plan'], handoff.plan().json())
        self.assertLessEqual(len(actual.stdout), terrain.MAX_OUTPUT)

    def test_file_symlink_and_nonregular_input_refuse_without_native_contact(self):
        with tempfile.TemporaryDirectory() as directory:
            real = Path(directory) / 'real.json'; real.write_bytes(canonical(request()))
            link = Path(directory) / 'link.json'; link.symlink_to(real)
            fifo = Path(directory) / 'fifo'; os.mkfifo(fifo)
            for path in (link, fifo, Path(directory)):
                output = []
                with patch.object(survey, '_MapClient', side_effect=AssertionError('premature contact')), \
                     patch.object(offline, 'publish', side_effect=lambda raw: output.append(raw) or 0):
                    code = command.main(['--request-file', str(path)])
                self.assertEqual(code, 2)
                self.assertEqual(len(output), 1)
                self.assertNotIn(str(path).encode(), output[0])

    def test_short_output_write_never_publishes_a_second_object(self):
        result, _ = self.complete()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'compiled.json'; path.write_bytes(result.encode())
            output = []
            with patch.object(offline, 'publish', side_effect=lambda data: output.append(data) or 2), \
                 patch.object(survey, '_MapClient', side_effect=AssertionError('offline contact')):
                self.assertEqual(command.main(['--compilation-file', str(path), '--emit', 'excavation-handoff']), 2)
            self.assertEqual(len(output), 1)
            self.assertEqual(RoomExcavationHandoff.decode(output[0]).digest, result.json()['handoff_digest'])

    def test_existing_layout_survey_remains_single_level_and_byte_identical(self):
        value = request()
        region = survey.native.Region.from_json(value['region'])
        with Peer(region) as peer:
            expected = survey.observed.solve(canonical(value), peer.payload, MANIFEST)
            actual = self.run_cli(canonical(value), emit='survey', env=environment(peer.address), script='survey_room_layout.py')
            self.assertEqual(actual.returncode, 0)
            self.assertEqual(actual.stdout, expected.encode())
            self.assertEqual(peer.calls, ['Handshake', 'ReadObservation'])

    def test_invalid_flags_never_contact_native_or_expose_paths(self):
        outputs = []
        with patch.object(offline, 'publish', side_effect=lambda raw: outputs.append(raw) or 0), \
             patch.object(survey, '_MapClient', side_effect=AssertionError('premature contact')):
            code = command.main(['--request-file', '/not/a/real/private/path', '--emit', 'excavation'])
        self.assertEqual(code, 2)
        self.assertEqual(len(outputs), 1)
        self.assertNotIn(b'/not/a/real/private/path', outputs[0])


if __name__ == '__main__':
    unittest.main()
