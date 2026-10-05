"""Actual map client, bounded room CLI, and joined TCP fault tests; no game writes."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))

import excavation_observer as e
import room_terrain as terrain
import survey_rooms as cli
from furniture_plan import canonical
from room_provisioning import RoomPlan
from room_terrain_fixtures import MANIFEST, bedroom, capture, coordinates, dining, expand, plan, request, visible
from room_terrain_peer import Peer, TOKEN


def environment(address):
    env = {key: value for key, value in os.environ.items() if not key.startswith('DFMCP_')}
    env.update(DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5='1', DFMCP_MAP_ENDPOINT=address,
               DFMCP_MAP_TOKEN=TOKEN.decode())
    return env


def command(path, env, *extra, request_file=False):
    return subprocess.run([sys.executable, str(ROOT / 'scripts/survey_rooms.py'),
                           '--request-file' if request_file else '--plan-file', str(path), *extra],
                          env=env, capture_output=True, timeout=8)


class SurveyRoomsTests(unittest.TestCase):
    def test_actual_cli_partial_rooms_keeps_full_intent_and_one_capture(self):
        original = plan()
        selected = terrain.selection(original)
        completed = set(sorted(selected.floors)[::3])
        raw = capture(selected.region, {p: visible(3) for p in completed})
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'rooms.json'
            path.write_bytes(original.encode())
            with Peer(raw, selected.region) as peer:
                proc = command(path, environment(peer.address))
                self.assertEqual(proc.returncode, 0, proc.stderr)
                output = json.loads(proc.stdout)
                self.assertEqual(proc.stdout, canonical(output))
                result = output['result']
                self.assertEqual(result['room_plan'], original.json())
                self.assertEqual(expand(result['remaining_blueprint']), selected.floors - completed)
                self.assertEqual(output['acquisition']['endpoint'], peer.address)
                self.assertEqual(output['acquisition']['native_observations'], 1)
                self.assertTrue(output['acquisition']['native_capture_established'])
                digest = output.pop('report_digest')
                self.assertEqual(digest, hashlib.sha256(b'dfmcp-room-terrain-report/1\0' + canonical(output)).hexdigest())
                self.assertEqual((peer.accepted, peer.handshakes, peer.observations), (1, 1, 1))
                self.assertEqual(peer.binds, ['Handshake', 'ReadObservation'])
                self.assertGreater(peer.fragments, 12)
                self.assertNotIn(TOKEN, proc.stdout + proc.stderr)
            self.assertEqual(path.read_bytes(), original.encode())
            self.assertEqual(list(Path(directory).iterdir()), [path])

    def test_actual_request_cli_exports_exact_nonempty_existing_blueprint_shape(self):
        value = request(bedroom(1, origin=(10, 10, 2)), dining(2, 2, origin=(10, 10, 4)))
        original = RoomPlan.compile(value)
        selected = terrain.selection(original)
        completed = {p: visible(3) for p in selected.floors if p[2] == 2}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'request.json'
            path.write_bytes(canonical(value))
            with Peer(capture(selected.region, completed), selected.region) as peer:
                proc = command(path, environment(peer.address), '--emit', 'remaining-blueprint', request_file=True)
                self.assertEqual(proc.returncode, 0, proc.stderr)
                artifact = json.loads(proc.stdout)
                self.assertEqual(proc.stdout, canonical(artifact))
                self.assertEqual(set(artifact), {'schema', 'parts'})
                self.assertEqual(artifact['schema'], 'dfmcp.excavation-blueprint/1')
                self.assertEqual(expand(artifact), selected.floors - completed.keys())
                self.assertLessEqual(len(artifact['parts']), 32)
                self.assertEqual(peer.observations, 1)

    def test_satisfied_and_blocked_are_observations_not_empty_executable_plans(self):
        original = plan()
        selected = terrain.selection(original)
        done = {p: visible(3) for p in selected.floors}
        cases = [('terrain_shapes_satisfied_at_sample', done),
                 ('blocked', {min(selected.floors): b'\x01'})]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'rooms.json'
            path.write_bytes(original.encode())
            for expected, overrides in cases:
                for export in (False, True):
                    with self.subTest(expected=expected, export=export):
                        with Peer(capture(selected.region, overrides), selected.region) as peer:
                            proc = command(path, environment(peer.address), *(['--emit', 'remaining-blueprint'] if export else []))
                            self.assertEqual(proc.returncode, 2 if export else 0, proc.stderr)
                            value = json.loads(proc.stdout)
                            if export:
                                self.assertFalse(value['ok'])
                                self.assertIsNone(value['result'])
                                self.assertIsNone(value['acquisition'])
                            else:
                                self.assertTrue(value['ok'])
                                self.assertEqual(value['result']['status'], expected)
                                self.assertIsNone(value['result']['remaining_blueprint'])
                            self.assertFalse(value['game_mutation_dispatched'])
                            self.assertFalse(value['agent_turn']['briefing']['native_effect_inventory_verified'])
                            self.assertEqual(peer.observations, 1)

    def test_malformed_protocol_and_lost_replies_never_retry_or_publish_partial_state(self):
        original = plan()
        selected = terrain.selection(original)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'rooms.json'
            path.write_bytes(original.encode())
            faults = ('alias_binding', 'refuse_handshake', 'handshake_payload', 'nonce', 'generation',
                      'software', 'profile', 'native_refusal', 'notifications', 'oversized_header',
                      'duplicate_field', 'truncated_reply', 'lost_reply')
            for fault in faults:
                with self.subTest(fault=fault):
                    with Peer(capture(selected.region), selected.region, fault=fault) as peer:
                        proc = command(path, environment(peer.address))
                        self.assertEqual(proc.returncode, 2, proc.stderr)
                        value = json.loads(proc.stdout)
                        self.assertFalse(value['ok'])
                        self.assertIsNone(value['result'])
                        self.assertEqual(peer.accepted, 1)
                        self.assertLessEqual(peer.observations, 1)
                        self.assertNotIn(TOKEN, proc.stdout + proc.stderr)
                        self.assertNotIn(str(path).encode(), proc.stdout + proc.stderr)

    def test_actual_capture_source_selection_and_native_bytes_are_revalidated(self):
        original = plan()
        selected = terrain.selection(original)
        wrong = e.Region((selected.region.origin[0] + 1, *selected.region.origin[1:]), selected.region.size)
        raws = (capture(selected.region, folder='foreign'), capture(selected.region, site=3),
                capture(wrong), capture(selected.region, dimensions=(1, 1, 1)),
                b'not-map-evidence', capture(selected.region) + b'\0')
        for raw in raws:
            with Peer(raw, selected.region) as peer, patch.dict(os.environ, environment(peer.address), clear=True):
                with self.assertRaises(ValueError):
                    cli.run(original, cli.Authority.load(), cli.Budget(3000))
                self.assertEqual(peer.observations, 1)
                self.assertEqual(peer.accepted, 1)

    def test_operator_revocation_at_handshake_observation_and_cpu_boundaries(self):
        original = plan()
        selected = terrain.selection(original)
        def revoke():
            os.environ.pop('DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5', None)
        for stage in ('handshake', 'observation', 'cpu'):
            with self.subTest(stage=stage):
                kwargs = {'on_' + stage: revoke} if stage != 'cpu' else {}
                with Peer(capture(selected.region), selected.region, **kwargs) as peer:
                    with patch.dict(os.environ, environment(peer.address), clear=True):
                        authority, budget = cli.Authority.load(), cli.Budget(3000)
                        real = terrain.partition
                        def partition(*args, **kw):
                            revoke()
                            return real(*args, **kw)
                        with patch.object(terrain, 'partition', partition if stage == 'cpu' else real):
                            with self.assertRaises(ValueError):
                                cli.run(original, authority, budget)
                        self.assertLessEqual(peer.observations, 1)
                        self.assertEqual(peer.accepted, 1)

    def test_final_serialization_revocation_applies_to_full_and_narrow_exports(self):
        original = plan()
        selected = terrain.selection(original)
        real = cli.serialize
        for emit in ('survey', 'remaining-blueprint'):
            with Peer(capture(selected.region), selected.region) as peer:
                with patch.dict(os.environ, environment(peer.address), clear=True):
                    def revoke(value):
                        output = real(value)
                        os.environ['DFMCP_MAP_TOKEN'] = 'different-query-token' * 3
                        return output
                    with patch.object(cli, 'serialize', revoke):
                        with self.assertRaises(ValueError):
                            cli.run(original, cli.Authority.load(), cli.Budget(3000), emit=emit)
                    self.assertEqual(peer.observations, 1)

    def test_unchanged_default_map_client_still_works(self):
        original = plan()
        selected = terrain.selection(original)
        raw = capture(selected.region)
        with Peer(raw, selected.region) as peer:
            with e.MapClient(peer.address, TOKEN, selected.region, 3000) as client:
                observed = client.observe()
                self.assertEqual(observed.raw, raw)
                with self.assertRaises(ValueError):
                    client.observe()
            self.assertEqual(peer.observations, 1)

    def test_shared_budget_is_not_renewed_at_connection_or_projection(self):
        original = plan()
        selected = terrain.selection(original)
        for allowance in ('calls_left', 'network_left'):
            with self.subTest(allowance=allowance):
                with Peer(capture(selected.region), selected.region) as peer:
                    with patch.dict(os.environ, environment(peer.address), clear=True):
                        budget = cli.Budget(3000)
                        setattr(budget, allowance, 3 if allowance == 'calls_left' else 16)
                        with self.assertRaises(ValueError):
                            cli.run(original, cli.Authority.load(), budget)
                        self.assertEqual(peer.observations, 0)
        with patch.dict(os.environ, environment('127.0.0.1:5000'), clear=True):
            for expired in (False, True):
                budget = cli.Budget(3000)
                if expired:
                    budget.deadline = time.monotonic() - 1
                else:
                    budget.work_left = 0
                with patch.object(e.socket, 'socket', side_effect=AssertionError('unexpected connect')):
                    with self.assertRaises(ValueError):
                        cli.run(original, cli.Authority.load(), budget)
        with Peer(capture(selected.region), selected.region) as peer:
            with patch.dict(os.environ, environment(peer.address), clear=True):
                budget = cli.Budget(3000)
                cli.run(original, cli.Authority.load(), budget)
                self.assertEqual(budget.calls_left, 0)
                self.assertLess(budget.work_left, cli.MAX_WORK)
                self.assertLess(budget.network_left, e.MAX_WIRE - len(capture(selected.region)))

    def test_wall_deadline_during_fragmented_read_is_not_retried(self):
        original = plan()
        selected = terrain.selection(original)
        with Peer(capture(selected.region), selected.region, delay=0.2) as peer:
            with patch.dict(os.environ, environment(peer.address), clear=True):
                started = time.monotonic()
                with self.assertRaises((ValueError, OSError)):
                    cli.run(original, cli.Authority.load(), cli.Budget(100))
                self.assertLess(time.monotonic() - started, 1)
                self.assertEqual(peer.observations, 1)
                self.assertEqual(peer.accepted, 1)

    def test_bad_original_artifacts_and_special_files_fail_before_connect(self):
        original = plan()
        selected = terrain.selection(original)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            paths = []
            corrupt = original.json()
            corrupt['room_completion_proven'] = True
            for name, raw in [('corrupt', canonical(corrupt)), ('huge', b'x' * (cli.MAX_PLAN_BYTES + 1)),
                              ('duplicate', b'{"intent":{},"intent":{}}'), ('empty', b'')]:
                path = root / name
                path.write_bytes(raw)
                paths.append(path)
            link = root / 'link'
            link.symlink_to(paths[0])
            paths.extend((link, root))
            fifo = root / 'fifo'
            os.mkfifo(fifo)
            paths.append(fifo)
            for path in paths:
                with self.subTest(path=path.name):
                    with Peer(capture(selected.region), selected.region) as peer:
                        proc = command(path, environment(peer.address))
                        self.assertEqual(proc.returncode, 2, proc.stderr)
                        self.assertIsNone(json.loads(proc.stdout)['result'])
                        self.assertEqual(peer.accepted, 0)

    def test_input_replacement_and_revocation_during_read_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'plan'
            replacement = Path(directory) / 'replacement'
            raw = plan().encode()
            path.write_bytes(raw)
            replacement.write_bytes(raw)
            budget = cli.Budget(3000)
            real_read = os.read
            changed = False
            def read(fd, count):
                nonlocal changed
                value = real_read(fd, count)
                if not changed:
                    changed = True
                    os.replace(replacement, path)
                return value
            with patch.object(cli.os, 'read', read):
                with self.assertRaises(ValueError):
                    cli.read_input(path, cli.MAX_PLAN_BYTES, budget.work, budget)
            with patch.dict(os.environ, environment('127.0.0.1:5000'), clear=True):
                authority = cli.Authority.load()
                budget = cli.Budget(3000)
                def revoked_read(fd, count):
                    value = real_read(fd, count)
                    del os.environ['DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5']
                    return value
                with patch.object(cli.os, 'read', revoked_read):
                    with self.assertRaises(ValueError):
                        cli.read_input(path, cli.MAX_PLAN_BYTES, authority.guard, budget)

    def test_closed_environment_and_endpoint_contract(self):
        variants = [
            {'DFMCP_DIG_TOKEN': 'x' * 40}, {'DFMCP_BUILD_TOKEN': 'x' * 40},
            {'DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5': 'true'}, {'DFMCP_MAP_TOKEN': 'x' * 31},
            {'DFMCP_MAP_ENDPOINT': 'localhost:5000'}, {'DFMCP_MAP_ENDPOINT': '8.8.8.8:5000'},
            {'DFMCP_MAP_ENDPOINT': '127.0.0.1:05000'}, {'DFMCP_MAP_ENDPOINT': '127.0.0.1:0'},
        ]
        for change in variants:
            with patch.dict(os.environ, {**environment('127.0.0.1:5000'), **change}, clear=True):
                with self.assertRaises(ValueError):
                    cli.Authority.load()
        for timeout in (0, 60001, True, 1.5):
            with self.assertRaises(ValueError):
                cli.Budget(timeout)

    def test_output_failure_never_reconnects_or_emits_a_second_packet(self):
        original = plan()
        selected = terrain.selection(original)
        class Stream:
            def __init__(self, mode):
                self.buffer, self.mode, self.writes = self, mode, []
            def write(self, data):
                self.writes.append(data)
                if self.mode == 'raise':
                    raise BrokenPipeError('fixture output failure')
                return len(data) - int(self.mode == 'partial')
            def flush(self):
                raise OSError('fixture flush failure')
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'rooms.json'
            path.write_bytes(original.encode())
            for mode in ('raise', 'partial', 'flush'):
                with Peer(capture(selected.region), selected.region) as peer:
                    stream = Stream(mode)
                    with patch.dict(os.environ, environment(peer.address), clear=True):
                        with patch.object(cli.sys, 'stdout', stream):
                            status = cli.main(['--plan-file', str(path)])
                    self.assertEqual(status, 2)
                    self.assertEqual(len(stream.writes), 1)
                    self.assertEqual(peer.observations, 1)
                    self.assertEqual(peer.accepted, 1)

    def test_large_multilevel_capture_and_complete_32_slot_envelope(self):
        roomy = request(dining(1, 1, origin=(1, 10, 1)), dining(1, 1, origin=(123, 11, 2), name='far'))
        roomy['world_folder'], roomy['site'] = 'f' * 512, 2**31 - 1
        maximum = e.Manifest(2**64 - 2, 'd' * 128, 'h' * 128)
        full = request(bedroom(8), dining(4, 4, origin=(10, 25, 2)))
        full['excluded_items'] = list(range(2147483000, 2147483646))
        for value, manifest in ((roomy, maximum), (full, MANIFEST)):
            original = RoomPlan.compile(value)
            selected = terrain.selection(original)
            raw = capture(selected.region, folder=value['world_folder'], site=value['site'])
            with Peer(raw, selected.region, manifest=manifest) as peer:
                with patch.dict(os.environ, environment(peer.address), clear=True):
                    output = cli.run(original, cli.Authority.load(), cli.Budget(5000))
                    result = json.loads(output)['result']
                    self.assertEqual(result['room_plan'], original.json())
                    self.assertEqual(expand(result['remaining_blueprint']), selected.floors)
                    self.assertLess(len(output), terrain.MAX_OUTPUT)
                    self.assertEqual(peer.observations, 1)
                    if value is roomy:
                        self.assertEqual(selected.region.volume, 3072)
                        self.assertGreater(len(raw), 60000)
                    else:
                        self.assertEqual(result['room_plan']['furniture_count'], 32)
                        self.assertEqual(result['room_plan']['intent']['excluded_items'], full['excluded_items'])


if __name__ == '__main__':
    unittest.main()
