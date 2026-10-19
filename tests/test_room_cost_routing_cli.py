"""Real map TCP/CLI execution with synthetic terrain; no native game qualification."""
from __future__ import annotations

from copy import deepcopy
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

from test_room_cost_workflow import MANIFEST, REGION, capture, layout_request
import excavation_observer as native
import layout_rooms as offline
import room_layout as layout
import room_layout_observed as observed
import survey_room_layout as command
from furniture_plan import canonical
from room_provisioning import RoomPlan

ROOT = Path(__file__).resolve().parents[1]
TOKEN = 'synthetic-corridor-read-token-for-test-only'


def environment(address=None):
    value = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
    value['PYTHONDONTWRITEBYTECODE'] = '1'
    if address is not None:
        value.update({command.OPT_IN: '1', command.ENDPOINT: address, command.TOKEN: TOKEN})
    return value


class Peer:
    """One joined source lifetime; only fixed map Handshake and ReadObservation."""
    def __init__(self, mode='normal', payload=None):
        self.mode = mode
        self.payload = capture() if payload is None else payload
        self.calls, self.connections, self.errors = [], 0, []
        self.closed = threading.Event()
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(1)
        self.listener.settimeout(5)
        host, port = self.listener.getsockname()
        self.address = f'{host}:{port}'
        self.thread = threading.Thread(target=self.serve)

    @staticmethod
    def read(stream, count):
        raw = bytearray()
        while len(raw) < count:
            part = stream.recv(count - len(raw))
            if not part:
                raise EOFError('test peer input ended')
            raw.extend(part)
        return bytes(raw)

    @staticmethod
    def send(stream, raw):
        frame = struct.pack('<h2xi', -1, len(raw)) + raw
        # Split both frame header and payload; do not assume one recv per frame.
        for start in range(0, len(frame), 13):
            stream.sendall(frame[start:start + 13])

    def serve(self):
        try:
            with self.listener.accept()[0] as stream:
                self.connections += 1
                stream.settimeout(5)
                if self.read(stream, 12) != b'DFHack?\n' + struct.pack('<i', 1):
                    raise AssertionError('wrong greeting')
                stream.sendall(b'DFHack!\n' + struct.pack('<i', 1))
                methods = {}
                for identifier, name in ((2, 'Handshake'), (3, 'ReadObservation')):
                    method, size = struct.unpack('<h2xi', self.read(stream, 8))
                    body = native.decode(self.read(stream, size), 4)
                    if method != 0 or body != {1: name.encode(), 2: b'dfmcp.map.v1_5.Request',
                            3: b'dfmcp.map.v1_5.Reply', 4: b'dfmcp_map_v1_5'}:
                        raise AssertionError('unexpected binding')
                    methods[identifier] = name
                    self.send(stream, native.encode({1: identifier}))
                for name in ('Handshake', 'ReadObservation'):
                    method, size = struct.unpack('<h2xi', self.read(stream, 8))
                    request = native.decode(self.read(stream, size), 11)
                    if methods[method] != name or request[1] != TOKEN.encode():
                        raise AssertionError('unexpected credential or operation')
                    if tuple(request[i] for i in range(5, 11)) != tuple(REGION['origin'] + REGION['size']):
                        raise AssertionError('substituted map selection')
                    self.calls.append(name)
                    if name == 'ReadObservation' and self.mode == 'lost':
                        return
                    reply = {1: 1, 2: 0, 3: request[2], 4: 1, 5: 5, 6: MANIFEST.generation,
                             7: MANIFEST.df_version.encode(), 8: MANIFEST.dfhack_version.encode()}
                    if name == 'ReadObservation':
                        reply[9] = self.payload
                        if self.mode == 'nonce': reply[3] = b'wrong-nonce'
                        if self.mode == 'source': reply[6] += 1
                        if self.mode == 'profile': reply[5] += 1
                        if self.mode == 'malformed': reply[9] = b'not-a-capture'
                    self.send(stream, native.encode(reply))
                if stream.recv(1) != b'':
                    raise AssertionError('unexpected second observation')
        except (OSError, EOFError) as error:
            self.errors.append(error)
        except BaseException as error:
            self.errors.append(error)
        finally:
            self.closed.set()

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_args):
        self.thread.join(8)
        self.listener.close()
        if self.thread.is_alive():
            raise AssertionError('test map peer did not quiesce')
        if self.errors:
            raise AssertionError(f'test map peer failed: {self.errors!r}')


class CostRoutingCLITests(unittest.TestCase):
    def process(self, raw, source, emit, *, address=None, strict=False):
        with tempfile.TemporaryDirectory() as directory:
            file = Path(directory) / 'input.json'
            file.write_bytes(raw)
            argv = [sys.executable, '-W', 'error', str(ROOT / 'scripts/survey_room_layout.py'),
                    source, str(file), '--emit', emit]
            if strict:
                argv.append('--require-optimal-layout')
            result = subprocess.run(argv, capture_output=True, env=environment(address), timeout=15)
            self.assertEqual(file.read_bytes(), raw)
            self.assertEqual(result.stderr, b'')
            return result

    def local(self, raw, source, emit, *, address=None):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'input.json'
            path.write_bytes(raw)
            output = []
            with patch.dict(os.environ, environment(address), clear=True), \
                    patch.object(offline, 'publish', side_effect=lambda value: output.append(value) or 0):
                code = command.main([source, str(path), '--emit', emit])
            self.assertEqual(len(output), 1)
            return code, output[0]

    def refused(self, process):
        self.assertEqual(process.returncode, 2)
        result = json.loads(process.stdout)
        self.assertFalse(result['ok'])
        self.assertIsNone(result['result'])
        self.assertNotIn(TOKEN.encode(), process.stdout)
        self.assertNotIn(b'input.json', process.stdout)
        return result

    def test_all_seven_live_exports_match_the_actual_costed_compiler(self):
        request = canonical(layout_request(observed_costs=True))
        expected = observed.solve(request, capture(), MANIFEST)
        for emit in command.EXPORTS:
            with self.subTest(emit=emit), Peer() as peer:
                result = self.process(request, '--request-file', emit, address=peer.address, strict=True)
                self.assertEqual(result.returncode, 0, result.stdout[:300])
                self.assertEqual(result.stdout, command.export(expected, emit, True, require_optimal=True))
                self.assertEqual(peer.calls, ['Handshake', 'ReadObservation'])
                self.assertEqual(peer.connections, 1)
                self.assertTrue(peer.closed.wait(2))
                if emit == 'room-plan':
                    plan = RoomPlan.decode(result.stdout)
                    self.assertEqual(plan.json()['target_tiles'], 39)
                    self.assertEqual(plan.json()['project_connections']['selected_corridor_cost'], 0)

    def test_saved_survey_all_exports_replay_offline_without_credentials(self):
        result = observed.solve(canonical(layout_request(observed_costs=True)), capture(), MANIFEST)
        for emit in command.EXPORTS:
            with self.subTest(emit=emit):
                actual = self.process(result.encode(), '--survey-file', emit, strict=True)
                self.assertEqual(actual.returncode, 0, actual.stdout[:300])
                self.assertEqual(actual.stdout, command.export(result, emit, False, require_optimal=True))
        report = json.loads(command.export(result, 'report', False))
        self.assertFalse(report['agent_turn']['coverage']['native_acquisition_verified_this_call'])
        self.assertEqual(report['agent_turn']['coverage']['native_observations'], 0)
        self.assertNotIn(b'connection compiler is not cost-rerouted', canonical(report))
        self.assertIn(b'global network cost optimality', canonical(report))

    def test_client_is_closed_before_actual_search_begins(self):
        original = observed.solve
        request = canonical(layout_request(observed_costs=True))
        with Peer() as peer:
            def checked(*args, **kwargs):
                self.assertTrue(peer.closed.wait(2), 'map connection remained open during search')
                return original(*args, **kwargs)
            with patch.object(observed, 'solve', side_effect=checked):
                code, raw = self.local(request, '--request-file', 'report', address=peer.address)
            self.assertEqual(code, 0)
            self.assertTrue(json.loads(raw)['agent_turn']['coverage']['native_acquisition_verified_this_call'])
            self.assertEqual(peer.calls, ['Handshake', 'ReadObservation'])

    def test_node_limited_incumbent_is_complete_but_every_strict_export_refuses(self):
        value = layout_request(observed_costs=True)
        value['areas'][1].pop('origin')
        value['search'] = {'max_nodes': 3}
        result = observed.solve(canonical(value), capture(), MANIFEST)
        self.assertEqual(result.json()['status'], 'layout_found')
        self.assertEqual(len(result.json()['layout']['room_plan']['areas']), 2)
        self.assertFalse(result.json()['layout']['optimization']['minimum_in_compiler_domain_proven'])
        for emit in command.EXPORTS:
            with self.subTest(emit=emit):
                packet = self.refused(self.process(result.encode(), '--survey-file', emit, strict=True))
                self.assertEqual(packet['error']['code'], 'ROOM_LAYOUT_OPTIMUM_UNPROVEN')
        actual = self.process(result.encode(), '--survey-file', 'room-plan')
        self.assertEqual(actual.returncode, 0)
        self.assertEqual(len(RoomPlan.decode(actual.stdout).request().slots), 4)

    def test_transport_and_source_faults_discard_candidate_without_retry(self):
        request = canonical(layout_request(observed_costs=True))
        cases = [('lost', None), ('nonce', None), ('source', None), ('profile', None),
                 ('malformed', None), ('normal', capture(paused=False)),
                 ('normal', capture(folder='other-fortress'))]
        for mode, payload in cases:
            with self.subTest(mode=mode), Peer(mode, payload) as peer:
                result = self.process(request, '--request-file', 'room-plan', address=peer.address)
                self.refused(result)
                self.assertEqual(peer.connections, 1)
                self.assertEqual(peer.calls, ['Handshake', 'ReadObservation'])

    def test_postcapture_authority_and_aggregate_budget_failures_publish_no_plan(self):
        request = canonical(layout_request(observed_costs=True))
        original = observed.solve
        for fault in ('authority', 'work', 'deadline'):
            with self.subTest(fault=fault), Peer() as peer:
                def interrupted(raw, payload, manifest, guard):
                    if fault == 'authority':
                        os.environ[command.TOKEN] = 'replacement-test-token-with-sufficient-length'
                    elif fault == 'work':
                        guard.__self__.left = 0
                    else:
                        guard.__self__.end = 0
                    return original(raw, payload, manifest, guard)
                with patch.object(observed, 'solve', side_effect=interrupted):
                    code, raw = self.local(request, '--request-file', 'room-plan', address=peer.address)
                self.assertEqual(code, 2)
                self.assertIsNone(json.loads(raw)['result'])
                self.assertNotIn(b'"room_plan"', raw)
                self.assertEqual(peer.calls, ['Handshake', 'ReadObservation'])

    def test_final_render_authority_loss_does_not_publish_cached_result(self):
        original = command.export
        request = canonical(layout_request(observed_costs=True))
        with Peer() as peer:
            def revoked(*args, **kwargs):
                raw = original(*args, **kwargs)
                os.environ[command.TOKEN] = 'replacement-test-token-after-result-encoding'
                return raw
            with patch.object(command, 'export', side_effect=revoked):
                code, raw = self.local(request, '--request-file', 'room-plan', address=peer.address)
            self.assertEqual(code, 2)
            self.assertIsNone(json.loads(raw)['result'])
            self.assertEqual(peer.connections, 1)

    def test_rehashed_cost_strategy_or_raw_map_tampering_is_not_replay(self):
        result = observed.solve(canonical(layout_request(observed_costs=True)), capture(), MANIFEST)
        for change in ('strategy', 'raw', 'count'):
            body = result.json(); body.pop('survey_digest')
            if change == 'strategy': body['request']['connection_strategy'] = layout.FALLBACK_STRATEGY
            elif change == 'raw': body['capture_hex'] = capture(free=set()).hex()
            else: body['excavation_evidence']['observed_wall_targets'] = 12
            forged = observed.ObservedLayout(canonical(body)).encode()
            self.refused(self.process(forged, '--survey-file', 'room-plan'))

    def test_short_output_attempt_never_repeats_search_or_writes_another_object(self):
        result = observed.solve(canonical(layout_request(observed_costs=True)), capture(), MANIFEST)
        with tempfile.TemporaryDirectory() as directory:
            file = Path(directory) / 'survey.json'; file.write_bytes(result.encode())
            output = []
            with patch.dict(os.environ, environment(), clear=True), \
                 patch.object(command, '_MapClient', side_effect=AssertionError('offline native contact')), \
                 patch.object(observed, 'solve', wraps=observed.solve) as solve, \
                 patch.object(offline, 'publish', side_effect=lambda raw: output.append(raw) or 2):
                code = command.main(['--survey-file', str(file), '--emit', 'room-plan'])
            self.assertEqual(code, 2)
            self.assertEqual(solve.call_count, 1)
            self.assertEqual(len(output), 1)
            RoomPlan.decode(output[0])


if __name__ == '__main__':
    unittest.main()
