"""Real map transport and original-handoff CLI; synthetic peers only."""
from contextlib import contextmanager
from copy import deepcopy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import layout_rooms as offline
import room_layout as layout
import room_layout_excavation as compilation
import survey_room_excavation as command
import survey_room_layout as survey
from furniture_plan import canonical
from room_excavation_handoff import RoomExcavationHandoff
from room_provisioning import RoomPlan
import test_room_cost_routing_cli as fixture_peer
from test_room_residual_capacity import capture
from test_room_residual_observed import MANIFEST, observed_request, source_request, compiled

ROOT = Path(__file__).resolve().parents[1]


@contextmanager
def peer_for(value, *, mode='normal', cells=None):
    """Configure the existing test peer, not a replacement runtime/client."""
    _, region, _ = compilation.prepare(canonical(value))
    source = source_request(value)
    if cells is not None:
        source['excavation_costs'] = cells
    payload = capture(source, region)
    with patch.object(fixture_peer, 'REGION', region.json()), patch.object(fixture_peer, 'MANIFEST', MANIFEST):
        with fixture_peer.Peer(mode=mode, payload=payload) as peer:
            yield peer


class ResidualCLITests(unittest.TestCase):
    def process(self, raw, *, source='--request-file', emit='report', address=None, strict=False,
                script='survey_room_excavation.py'):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'input.json'
            path.write_bytes(raw)
            args = [sys.executable, '-W', 'error', str(ROOT / 'scripts' / script), source, str(path), '--emit', emit]
            if strict:
                args.append('--require-optimal-layout')
            result = subprocess.run(args, capture_output=True, env=fixture_peer.environment(address), timeout=15)
            self.assertEqual(path.read_bytes(), raw)
            self.assertEqual(result.stderr, b'')
            return result

    def local(self, raw, *, source='--request-file', emit='report', address=None, strict=False):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'input.json'
            path.write_bytes(raw)
            output = []
            args = [source, str(path), '--emit', emit]
            if strict:
                args.append('--require-optimal-layout')
            with patch.dict(os.environ, fixture_peer.environment(address), clear=True), \
                    patch.object(offline, 'publish', side_effect=lambda data: output.append(data) or 0):
                code = command.main(args)
            self.assertEqual(len(output), 1)
            return code, output[0]

    def assert_refused(self, process, code=None):
        self.assertEqual(process.returncode, 2)
        value = json.loads(process.stdout)
        self.assertFalse(value['ok'])
        self.assertIsNone(value['result'])
        if code:
            self.assertEqual(value['error']['code'], code)
        self.assertNotIn(fixture_peer.TOKEN.encode(), process.stdout)
        self.assertNotIn(b'input.json', process.stdout)
        return value

    def native_exports(self, kinds):
        value = observed_request()
        for emit in kinds:
            with self.subTest(emit=emit), peer_for(value) as peer:
                expected, handoff = compilation.solve(canonical(value), peer.payload, MANIFEST, peer.address)
                result = self.process(canonical(value), emit=emit, address=peer.address, strict=True)
                self.assertEqual(result.returncode, 0, result.stdout[:300])
                self.assertEqual(result.stdout, command.export(expected, handoff, emit, True, require_optimal=True))
                self.assertEqual(peer.calls, ['Handshake', 'ReadObservation'])
                self.assertEqual(peer.connections, 1)
                self.assertTrue(peer.closed.wait(2))
                if emit == 'excavation-handoff':
                    self.assertEqual(RoomExcavationHandoff.decode(result.stdout), handoff)
                if emit == 'room-plan':
                    self.assertEqual(len(RoomPlan.decode(result.stdout).request().slots), 32)

    def saved_exports(self, kinds):
        result, handoff = compiled(observed_request())
        for emit in kinds:
            with self.subTest(emit=emit), patch.object(survey, '_MapClient', side_effect=AssertionError('offline native contact')):
                code, raw = self.local(result.encode(), source='--compilation-file', emit=emit, strict=True)
                self.assertEqual(code, 0)
                self.assertEqual(raw, command.export(result, handoff, emit, False, require_optimal=True))
                actual = self.process(result.encode(), source='--compilation-file', emit=emit, strict=True)
                self.assertEqual(actual.returncode, 0)
                self.assertEqual(actual.stdout, raw)
        report = json.loads(command.export(result, handoff, 'report', False))
        self.assertEqual(report['agent_turn']['coverage']['native_observations'], 0)
        self.assertFalse(report['agent_turn']['coverage']['native_acquisition_verified_this_call'])
        self.assertEqual(report['result']['residual_partition']['selected']['blueprint_parts'], 28)

    def test_native_report_compilation_and_layout_exports(self):
        self.native_exports(command.EXPORTS[:3])

    def test_native_room_request_and_furniture_exports(self):
        self.native_exports(command.EXPORTS[3:6])

    def test_native_terrain_residual_and_handoff_exports(self):
        self.native_exports(command.EXPORTS[6:])

    def test_saved_report_compilation_and_layout_exports(self):
        self.saved_exports(command.EXPORTS[:3])

    def test_saved_room_request_and_furniture_exports(self):
        self.saved_exports(command.EXPORTS[3:6])

    def test_saved_terrain_residual_and_handoff_exports(self):
        self.saved_exports(command.EXPORTS[6:])

    def test_strict_export_rejects_all_node_limited_artifact_kinds(self):
        value = observed_request()
        value['search'] = {'max_nodes': 9}
        result, handoff = compiled(value)
        self.assertIsNotNone(handoff)
        for emit in command.EXPORTS:
            with self.subTest(emit=emit):
                self.assert_refused(self.process(result.encode(), source='--compilation-file', emit=emit, strict=True),
                                    'ROOM_EXCAVATION_OPTIMUM_UNPROVEN')
        permissive = self.process(result.encode(), source='--compilation-file', emit='excavation-handoff')
        self.assertEqual(permissive.returncode, 0)
        self.assertEqual(RoomExcavationHandoff.decode(permissive.stdout), handoff)

    def test_empty_work_never_becomes_a_nonempty_or_replacement_handoff(self):
        result, _ = compiled(observed_request(), cells='0'*252)
        self.assert_refused(self.process(result.encode(), source='--compilation-file', emit='excavation-handoff', strict=True),
                            'ROOM_EXCAVATION_NO_REMAINING_WORK')

    def test_native_stream_closes_before_capacity_search_starts(self):
        value = observed_request()
        real = compilation.solve
        with peer_for(value) as peer:
            def checked(*args):
                self.assertTrue(peer.closed.wait(2), 'native stream retained during search')
                return real(*args)
            with patch.object(compilation, 'solve', side_effect=checked):
                code, raw = self.local(canonical(value), address=peer.address)
            self.assertEqual(code, 0)
            self.assertTrue(json.loads(raw)['result']['handoff_available'])

    def test_lost_or_confused_native_reply_returns_no_plan_and_never_retries(self):
        value = observed_request()
        for mode in ('lost', 'nonce', 'source', 'profile', 'malformed'):
            with self.subTest(mode=mode), peer_for(value, mode=mode) as peer:
                self.assert_refused(self.process(canonical(value), emit='excavation-handoff', address=peer.address))
                self.assertEqual(peer.connections, 1)
                self.assertEqual(peer.calls, ['Handshake', 'ReadObservation'])

    def test_invalid_strategy_or_imported_costs_refuse_before_native_contact(self):
        for changes in ({'residual_partition': True}, {'residual_partition': 'other'},
                        {'excavation_costs': '0'*252}, {'search': {'max_work': 1}}):
            value = {**observed_request(), **changes}
            with self.subTest(changes=changes), patch.object(survey, '_MapClient', side_effect=AssertionError('premature native contact')):
                code, raw = self.local(canonical(value), address='127.0.0.1:5000')
                self.assertEqual(code, 2)
                self.assertFalse(json.loads(raw)['agent_turn']['coverage']['native_connection_attempted'])

    def test_authority_loss_after_capture_and_final_encoding_discards_handoff(self):
        value = observed_request()
        for boundary in ('capture', 'encoding'):
            with self.subTest(boundary=boundary), peer_for(value) as peer:
                target, name = (compilation, 'solve') if boundary == 'capture' else (command, 'export')
                real = getattr(target, name)
                def revoked(*args, **kwargs):
                    if boundary == 'capture':
                        os.environ[survey.TOKEN] = 'different-credential-long-enough-for-validation'
                        return real(*args, **kwargs)
                    result = real(*args, **kwargs)
                    os.environ[survey.TOKEN] = 'different-credential-long-enough-for-validation'
                    return result
                with patch.object(target, name, side_effect=revoked):
                    code, raw = self.local(canonical(value), address=peer.address, emit='excavation-handoff')
                self.assertEqual(code, 2)
                self.assertIsNone(json.loads(raw)['result'])
                self.assertEqual(peer.connections, 1)

    def test_deadline_and_work_expiry_after_capture_do_not_publish_incumbent(self):
        value = observed_request()
        real = compilation.solve
        for field in ('end', 'left'):
            with self.subTest(field=field), peer_for(value) as peer:
                def exhausted(raw, payload, manifest, address, guard):
                    setattr(guard.__self__, field, 0)
                    return real(raw, payload, manifest, address, guard)
                with patch.object(compilation, 'solve', side_effect=exhausted):
                    code, raw = self.local(canonical(value), address=peer.address, emit='excavation-handoff')
                self.assertEqual(code, 2)
                self.assertIsNone(json.loads(raw)['result'])
                self.assertEqual(json.loads(raw)['error']['code'],
                    'ROOM_EXCAVATION_TIMEOUT' if field == 'end' else 'ROOM_EXCAVATION_WORK_LIMIT')

    def test_rehashed_capacity_forgery_cannot_pass_strict_cli_replay(self):
        result, _ = compiled(observed_request())
        body = result.json()
        body.pop('compilation_digest')
        body['layout']['residual_partition']['selected']['blueprint_parts'] = 1
        forged = compilation.ExcavationLayout(canonical(body)).encode()
        self.assert_refused(self.process(forged, source='--compilation-file', emit='excavation-handoff', strict=True))

    def test_short_output_attempt_never_emits_a_second_packet_or_contacts_native(self):
        result, _ = compiled(observed_request())
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'saved.json'
            path.write_bytes(result.encode())
            output = []
            with patch.object(survey, '_MapClient', side_effect=AssertionError('offline contact')), \
                    patch.object(offline, 'publish', side_effect=lambda data: output.append(data) or 2):
                code = command.main(['--compilation-file', str(path), '--emit', 'excavation-handoff', '--require-optimal-layout'])
            self.assertEqual(code, 2)
            self.assertEqual(len(output), 1)
            self.assertEqual(RoomExcavationHandoff.decode(output[0]).plan().json()['furniture_count'], 32)

    def test_strict_flag_does_not_invent_an_objective_for_legacy_requests(self):
        value = observed_request(restricted=False, small=True)
        value.pop('objective')
        result, _ = compiled(value)
        self.assert_refused(self.process(result.encode(), source='--compilation-file', strict=True),
                            'ROOM_EXCAVATION_OPTIMUM_UNPROVEN')
        normal = self.process(result.encode(), source='--compilation-file')
        self.assertEqual(normal.returncode, 0)


if __name__ == '__main__':
    unittest.main()
