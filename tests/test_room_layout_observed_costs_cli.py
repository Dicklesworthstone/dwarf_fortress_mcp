"""Real read-only TCP, complete-source replay and CLI publication boundaries.

The peer emits synthetic DFMM1500 bytes. The actual existing MapClient, framing,
full module imports, compiler, private test files and CLI subprocesses run here;
no native DFHack or live-fortress qualification is inferred.
"""
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

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import excavation_observer as native
from furniture_allocation import Candidate, Request, allocate
from furniture_plan import canonical
import layout_rooms as offline
import room_layout as layout
import room_layout_observed as observed
from room_provisioning import RoomPlan
import survey_room_layout as command
from test_room_layout_costs import hall
from test_room_layout_excavation_observed import MANIFEST, map_bytes, request, solve

TOKEN = b'synthetic-query-only-token-12345678'


def environment(peer=None):
    env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
    env['PYTHONDONTWRITEBYTECODE'] = '1'
    if peer is not None:
        env.update({command.OPT_IN: '1', command.ENDPOINT: peer.address,
                    command.TOKEN: TOKEN.decode('ascii')})
    return env


class MapPeer:
    """One joined finite session with independently selected source faults."""
    def __init__(self, value, *, capture=None, fault=None, fragmented=False):
        self.value, self.capture = deepcopy(value), map_bytes(value) if capture is None else capture
        self.fault, self.fragmented = fault, fragmented
        self.events, self.errors, self.closed = [], [], False
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(1)
        self.listener.settimeout(5)
        self.address = '127.0.0.1:' + str(self.listener.getsockname()[1])
        self.thread = threading.Thread(target=self._run, daemon=False)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, kind, *_args):
        self.thread.join(6)
        self.listener.close()
        if self.thread.is_alive():
            self.thread.join(6)
            raise AssertionError('synthetic map peer did not drain')
        if kind is None and self.errors:
            raise self.errors[0]

    @staticmethod
    def read(conn, count):
        value = bytearray()
        while len(value) < count:
            chunk = conn.recv(count - len(value))
            if not chunk:
                raise AssertionError('client closed before complete expected request')
            value += chunk
        return bytes(value)

    def send(self, conn, payload):
        data = struct.pack('<h2xi', -1, len(payload)) + payload
        if self.fragmented:
            for start in range(0, len(data), 7):
                conn.sendall(data[start:start + 7])
        else:
            conn.sendall(data)

    def frame(self, conn):
        method, length = struct.unpack('<h2xi', self.read(conn, 8))
        if not 0 <= length <= 2048:
            raise AssertionError('unbounded test request')
        return method, native.decode(self.read(conn, length), 11)

    def _run(self):
        try:
            with self.listener.accept()[0] as conn:
                conn.settimeout(5)
                self.events.append('connected')
                if self.read(conn, 12) != b'DFHack?\n' + struct.pack('<i', 1):
                    raise AssertionError('wrong client greeting')
                conn.sendall(b'DFHack!\n' + struct.pack('<i', 1))
                for number, name in enumerate(('Handshake', 'ReadObservation'), 2):
                    method, fields = self.frame(conn)
                    if method != 0 or fields != {1: name.encode(), 2: b'dfmcp.map.v1_5.Request',
                            3: b'dfmcp.map.v1_5.Reply', 4: b'dfmcp_map_v1_5'}:
                        raise AssertionError('a non-map or mutation method was bound')
                    self.events.append('bind:' + name)
                    self.send(conn, native.encode({1: number}))
                nonce = None
                for number, operation in ((2, 'handshake'), (3, 'capture')):
                    method, fields = self.frame(conn)
                    expected = {1: TOKEN, 3: 1, 4: 5,
                        **dict(zip(range(5, 11), self.value['region']['origin'] + self.value['region']['size'])),
                        11: max(1024, 575 + 20 * len(self.capture_cells()))}
                    if method != number or {k: v for k, v in fields.items() if k != 2} != expected:
                        raise AssertionError('native request changed original selection or authority')
                    if not isinstance(fields.get(2), bytes) or len(fields[2]) != 32:
                        raise AssertionError('missing fresh nonce')
                    nonce = fields[2] if nonce is None else nonce
                    if fields[2] != nonce:
                        raise AssertionError('connection nonce changed')
                    self.events.append(operation)
                    if operation == 'capture' and self.fault == 'lost_reply':
                        return
                    response = {1: 1, 2: 0, 3: nonce, 4: 1, 5: 5,
                                6: MANIFEST.generation, 7: MANIFEST.df_version.encode(),
                                8: MANIFEST.dfhack_version.encode()}
                    if operation == 'capture':
                        response[9] = self.capture
                        if self.fault == 'source_changed': response[6] += 1
                        elif self.fault == 'wrong_nonce': response[3] = b'X' * 32
                        elif self.fault == 'wrong_profile': response[5] = 6
                    self.send(conn, native.encode(response))
                if conn.recv(1) != b'':
                    raise AssertionError('unexpected repeat read or mutation after original capture')
        except BaseException as error:
            self.errors.append(error)
        finally:
            self.closed = True

    def capture_cells(self):
        x, y, z = self.value['region']['size']
        return range(x * y * z)


class Buffer:
    def __init__(self, *, short=False):
        self.buffer = self
        self.raw, self.calls, self.short = b'', 0, short

    def write(self, raw):
        self.calls += 1
        self.raw += raw
        return len(raw) - int(self.short)

    def flush(self):
        pass


class ObservedExcavationCLITests(unittest.TestCase):
    def cli(self, path, mode, kind='survey', *, env=None, strict=True, script='survey_room_layout.py'):
        args = [sys.executable, '-W', 'error', str(ROOT / 'scripts' / script),
                mode, str(path), '--emit', kind]
        if strict:
            args.append('--require-optimal-layout')
        return subprocess.run(args, capture_output=True, check=False,
                              env=environment() if env is None else env, timeout=15)

    def assert_one_read(self, peer):
        self.assertEqual(peer.events, ['connected', 'bind:Handshake', 'bind:ReadObservation', 'handshake', 'capture'])
        self.assertTrue(peer.closed)
        self.assertEqual(peer.errors, [])

    def test_all_seven_real_tcp_exports_preserve_exact_complete_artifacts(self):
        value = request()
        result = solve(value)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'request.json'
            original = canonical(value)
            path.write_bytes(original)
            for kind in command.EXPORTS:
                with self.subTest(kind=kind), MapPeer(value, fragmented=True) as peer:
                    response = self.cli(path, '--request-file', kind, env=environment(peer))
                self.assert_one_read(peer)
                self.assertEqual(response.returncode, 0, response.stdout)
                self.assertEqual(response.stderr, b'')
                self.assertEqual(response.stdout, command.export(result, kind, True, require_optimal=True))
                self.assertNotIn(TOKEN, response.stdout)
                self.assertEqual(path.read_bytes(), original)
                if kind == 'report':
                    body = json.loads(response.stdout)
                    self.assertTrue(body['agent_turn']['coverage']['native_acquisition_verified_this_call'])
                    self.assertEqual(body['agent_turn']['briefing']['excavation_comparison']['observed_wall_targets'], 0)
                elif kind == 'room-plan':
                    plan = RoomPlan.decode(response.stdout)
                    self.assertEqual(plan.json()['furniture_count'], 2)
                elif kind == 'furniture-request':
                    wanted = Request.decode(response.stdout)
                    items = tuple(Candidate(i, s.kind, s.target, (0, 1), -1) for i, s in enumerate(wanted.slots))
                    self.assertEqual(len(allocate(wanted, items)['plan']['steps']), 2)

    def test_saved_survey_exports_are_offline_and_do_not_reacquire_evidence(self):
        result = solve(request())
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'survey.json'
            path.write_bytes(result.encode())
            env = {**environment(), 'DFMCP_UNRELATED_INVALID_AUTHORITY': '1'}
            for kind in command.EXPORTS:
                with self.subTest(kind=kind):
                    response = self.cli(path, '--survey-file', kind, env=env)
                    self.assertEqual(response.returncode, 0, response.stdout)
                    self.assertEqual(response.stderr, b'')
                    self.assertEqual(response.stdout, command.export(result, kind, False, require_optimal=True))
            body = json.loads(self.cli(path, '--survey-file', 'report', env=env).stdout)
            self.assertFalse(body['agent_turn']['coverage']['native_acquisition_verified_this_call'])
            self.assertEqual(body['agent_turn']['coverage']['native_observations'], 0)
            self.assertEqual(path.read_bytes(), result.encode())

    def test_standalone_layout_export_replays_as_assumptions_not_native_evidence(self):
        result = solve(request())
        nested = command.export(result, 'layout', False)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'layout.json'; path.write_bytes(nested)
            response = self.cli(path, '--layout-file', 'report', script='layout_rooms.py')
        self.assertEqual(response.returncode, 0, response.stdout)
        body = json.loads(response.stdout)
        self.assertFalse(body['result']['terrain_observed'])
        self.assertEqual(body['result']['optimization']['cost_evidence'], 'caller_declared_not_observed')
        self.assertEqual(body['agent_turn']['coverage']['native_observations'], 0)
        self.assertNotIn('excavation_evidence', body['result'])

    def test_strict_live_and_saved_exports_refuse_node_limited_complete_incumbents(self):
        value = request(search={'max_nodes': 2})
        result = solve(value)
        self.assertEqual(result.json()['status'], 'layout_found')
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'request.json'; source.write_bytes(canonical(value))
            saved = Path(directory) / 'survey.json'; saved.write_bytes(result.encode())
            for kind in command.EXPORTS:
                with self.subTest(kind=kind), MapPeer(value) as peer:
                    live = self.cli(source, '--request-file', kind, env=environment(peer))
                self.assert_one_read(peer)
                replay = self.cli(saved, '--survey-file', kind)
                for response in (live, replay):
                    self.assertEqual(response.returncode, 2)
                    self.assertEqual(response.stderr, b'')
                    body = json.loads(response.stdout)
                    self.assertIsNone(body['result'])
                    self.assertEqual(body['error']['code'], 'ROOM_LAYOUT_OPTIMUM_UNPROVEN')
                    self.assertFalse(body['agent_turn']['briefing']['mutation_admissible'])
            ordinary = self.cli(saved, '--survey-file', strict=False)
            self.assertEqual(ordinary.returncode, 0)
            self.assertEqual(ordinary.stdout, result.encode())

    def test_strict_flag_never_upgrades_legacy_first_feasible_search(self):
        value = request(); value.pop('objective')
        result = solve(value)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'legacy.json'; path.write_bytes(result.encode())
            response = self.cli(path, '--survey-file')
            default = self.cli(path, '--survey-file', strict=False)
        self.assertEqual(response.returncode, 2)
        self.assertEqual(json.loads(response.stdout)['error']['code'], 'ROOM_LAYOUT_OPTIMUM_UNPROVEN')
        self.assertEqual(default.stdout, result.encode())
        self.assertEqual(default.returncode, 0)

    def test_caller_costs_and_bad_requests_fail_before_native_contact(self):
        for value in (request(excavation_costs='0' * 24), request(objective=layout.COST_OBJECTIVE),
                      request(objective='unsupported'), request(size=(33, 32, 1))):
            with tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / 'request.json'; path.write_bytes(canonical(value))
                output = Buffer()
                with patch.dict(os.environ, environment(), clear=True), patch.object(sys, 'stdout', output), \
                     patch.object(native.socket, 'socket', side_effect=AssertionError('unexpected contact')):
                    code = command.main(['--request-file', str(path), '--emit', 'survey'])
                self.assertEqual(code, 2)
                self.assertEqual(output.calls, 1)
                self.assertFalse(json.loads(output.raw)['agent_turn']['coverage']['native_connection_attempted'])

    def test_lost_reply_wrong_source_and_malformed_captures_never_retry_or_publish(self):
        value = request()
        cases = [('lost_reply', None), ('source_changed', None), ('wrong_nonce', None), ('wrong_profile', None),
                 (None, map_bytes(value)[:-1]), (None, map_bytes(value, paused=False)),
                 (None, map_bytes(value, folder='another-fortress'))]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'request.json'; path.write_bytes(canonical(value))
            for fault, capture in cases:
                with self.subTest(fault=fault, capture=capture is not None), \
                     MapPeer(value, fault=fault, capture=capture) as peer:
                    response = self.cli(path, '--request-file', env=environment(peer))
                self.assert_one_read(peer)
                self.assertEqual(response.returncode, 2)
                self.assertEqual(response.stderr, b'')
                body = json.loads(response.stdout)
                self.assertIsNone(body['result'])
                self.assertFalse(body['agent_turn']['coverage']['native_acquisition_verified_this_call'])
                self.assertNotIn(TOKEN, response.stdout)
                self.assertNotIn(b'another-fortress', response.stdout)

    def test_map_connection_is_closed_before_real_ranking_search(self):
        value = request()
        closed, searches = [], []
        original_close, original_solve = native.MapClient.close, layout.solve
        def close(client):
            original_close(client)
            closed.append(client.closed)
        def solve_after_close(*args, **kwargs):
            self.assertTrue(closed and all(closed))
            searches.append(1)
            return original_solve(*args, **kwargs)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'request.json'; path.write_bytes(canonical(value))
            output = Buffer()
            with MapPeer(value) as peer:
                with patch.dict(os.environ, environment(peer), clear=True), patch.object(sys, 'stdout', output), \
                     patch.object(native.MapClient, 'close', close), patch.object(layout, 'solve', solve_after_close):
                    code = command.main(['--request-file', str(path), '--emit', 'survey'])
            self.assert_one_read(peer)
        self.assertEqual(code, 0, output.raw)
        self.assertEqual(searches, [1])

    def test_revoked_or_replaced_authority_after_real_search_cannot_publish_success(self):
        value = request()
        original = observed.solve
        for replaced in (False, True):
            def changed(*args, **kwargs):
                answer = original(*args, **kwargs)
                if replaced: os.environ[command.TOKEN] = 'different-query-token-123456789012345'
                else: os.environ.pop(command.OPT_IN)
                return answer
            with tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / 'request.json'; path.write_bytes(canonical(value))
                output = Buffer()
                with MapPeer(value) as peer:
                    with patch.dict(os.environ, environment(peer), clear=True), patch.object(sys, 'stdout', output), \
                         patch.object(observed, 'solve', changed):
                        code = command.main(['--request-file', str(path), '--emit', 'survey'])
                self.assert_one_read(peer)
            self.assertEqual(code, 2)
            self.assertEqual(output.calls, 1)
            self.assertIsNone(json.loads(output.raw)['result'])

    def test_final_deadline_or_work_loss_discards_a_completely_solved_candidate(self):
        value = request()
        original_solve, original_export = observed.solve, command.export
        for stage in ('work', 'deadline'):
            def exhausted_search(*args, **kwargs):
                answer = original_solve(*args, **kwargs)
                # The real enclosing Budget owns this injected guard.
                if stage == 'work':
                    args[3].__self__.left = 0
                    args[3]()
                return answer
            def expired_export(*args, **kwargs):
                answer = original_export(*args, **kwargs)
                if stage == 'deadline':
                    raise offline.DeadlineExceeded('injected final publication deadline')
                return answer
            with tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / 'request.json'; path.write_bytes(canonical(value))
                output = Buffer()
                with MapPeer(value) as peer:
                    with patch.dict(os.environ, environment(peer), clear=True), patch.object(sys, 'stdout', output), \
                         patch.object(observed, 'solve', exhausted_search), patch.object(command, 'export', expired_export):
                        code = command.main(['--request-file', str(path), '--emit', 'survey'])
                self.assert_one_read(peer)
            self.assertEqual(code, 2)
            self.assertEqual(output.calls, 1)
            self.assertIsNone(json.loads(output.raw)['result'])

    def test_short_output_has_one_write_and_one_capture_without_second_response(self):
        value = request()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'request.json'; path.write_bytes(canonical(value))
            output = Buffer(short=True)
            with MapPeer(value) as peer:
                with patch.dict(os.environ, environment(peer), clear=True), patch.object(sys, 'stdout', output):
                    code = command.main(['--request-file', str(path), '--emit', 'survey'])
            self.assert_one_read(peer)
        self.assertEqual(code, 2)
        self.assertEqual(output.calls, 1)
        self.assertEqual(output.raw, solve(value).encode())

    def test_rehashed_forged_proof_and_counts_are_rejected_by_saved_cli(self):
        result = solve(request(search={'max_nodes': 2}))
        for field in ('proof', 'counts', 'costs'):
            body = result.json(); body.pop('survey_digest')
            if field == 'proof':
                body['excavation_evidence']['minimum_in_compiler_domain_proven'] = True
                body['layout']['optimization']['minimum_in_compiler_domain_proven'] = True
            elif field == 'counts': body['excavation_evidence']['observed_wall_targets'] = 0
            else: body['layout']['request']['excavation_costs'] = '0' * 24
            forged = observed.ObservedLayout(canonical(body))
            with tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / 'survey.json'; path.write_bytes(forged.encode())
                response = self.cli(path, '--survey-file')
            self.assertEqual(response.returncode, 2)
            self.assertIsNone(json.loads(response.stdout)['result'])

    def test_process_hash_seeds_leave_saved_cost_evidence_identical(self):
        result = solve(request())
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'survey.json'; path.write_bytes(result.encode())
            for seed in ('1', '29', 'random'):
                response = self.cli(path, '--survey-file', env={**environment(), 'PYTHONHASHSEED': seed})
                self.assertEqual(response.returncode, 0, response.stdout)
                self.assertEqual(response.stdout, result.encode())


if __name__ == '__main__':
    unittest.main()
