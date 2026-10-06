"""Original room request -> native allocation -> placement -> completion.

Actual Python CLI processes and private journals use a joined synthetic peer.
No DFHack game, Rust/MCP admission or native acquisition signature is claimed.
"""
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import furniture_batch as batch
import furniture_inventory as inventory
import plan_rooms as planner
import build_placement_rpc as placement_rpc
import construction_monitor_rpc as monitor_rpc
from room_furniture_handoff import RoomFurnitureHandoff, MAX_BYTES
from furniture_plan import canonical
from test_room_furniture_handoff import room, allocated
from room_operations_peer import Peer, PAGE

ROOT = Path(__file__).resolve().parents[1]


class RoomPipelineTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.path = self.root / 'batch'
        self.path.mkdir(mode=0o700)
        self.plan = room(count=1)
        self.peer = Peer(allocated(self.plan))
        self.addCleanup(self.peer.close)
        self.peer.operations_tick = 100
        self.address = f'{self.peer.address[0]}:{self.peer.address[1]}'
        env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
        env['PYTHONDONTWRITEBYTECODE'] = '1'
        self.env = patch.dict(os.environ, env, clear=True)
        self.env.start()
        self.addCleanup(self.env.stop)
        self.environment('allocation')
        self.spec = self.root / 'plan.json'
        self.spec.write_bytes(self.plan.encode())

    def environment(self, mode):
        for name in list(os.environ):
            if name.startswith('DFMCP_'):
                del os.environ[name]
        self.peer.mode = mode
        if mode == 'allocation':
            os.environ.update({inventory.OPT_IN: '1', inventory.ENDPOINT: self.address,
                               inventory.OPERATIONS_TOKEN: 'o' * 32})
        elif mode == 'placement':
            os.environ.update({placement_rpc.OPT_IN: '1', placement_rpc.TOKEN: 't' * 32,
                               placement_rpc.ENDPOINT: self.address, placement_rpc.PLACE: '1'})
        elif mode == 'monitor':
            os.environ.update({monitor_rpc.OPT_IN: '1', monitor_rpc.BUILD_TOKEN: 't' * 32,
                               monitor_rpc.OPERATIONS_TOKEN: 'o' * 32, monitor_rpc.ENDPOINT: self.address})

    def run_cli(self, name, *args, success=True):
        out = subprocess.run([sys.executable, str(ROOT / 'scripts' / name), *args],
                             capture_output=True, timeout=70)
        self.assertEqual(out.returncode, 0 if success else 2, out.stdout + out.stderr)
        return out.stdout

    def export(self, success=True):
        return self.run_cli('plan_rooms.py', 'allocate', '--plan-file', str(self.spec),
                            '--emit', 'room-handoff', '--timeout-ms', '60000', success=success)

    def pipeline(self):
        request = self.root / 'request.json'
        request.write_bytes(canonical(self.plan.json()['intent']))
        compiled = self.run_cli('plan_rooms.py', 'compile', '--request-file', str(request))
        self.assertEqual(compiled, self.plan.encode())
        self.spec.write_bytes(compiled)
        capture = self.peer.operations()
        raw = self.export()
        h = RoomFurnitureHandoff.decode(raw)
        self.assertEqual(h.room_plan, self.plan)
        self.assertEqual(h.allocation.source.capture_sha256, hashlib.sha256(capture).hexdigest())
        self.assertEqual(h.allocation.source.capture_bytes, len(capture))
        self.assertEqual(h.allocation.source.generation, 7)
        self.assertEqual(h.allocation.request, self.plan.request())
        input_path = self.root / 'room-handoff.json'
        input_path.write_bytes(raw)
        self.environment('placement')
        result = json.loads(self.run_cli('furniture_batch.py', 'init', '--directory', str(self.path),
                                        '--room-handoff', str(input_path)))['result']
        identity = result['batch_id']
        input_path.unlink()
        self.spec.unlink()
        request.unlink()
        for _ in h.allocation.plan().steps:
            review = batch.review(str(self.path), identity)
            result = batch.advance(str(self.path), identity, review['expected_plan'], review['confirm_review'])
            self.assertEqual(result['room_plan'], self.plan.json())
        self.assertEqual(result['status'], 'all_placed')
        self.assertEqual(result['source']['generation'], 987)
        self.assertEqual(result['room_origin']['allocation_digest'], h.allocation.digest)
        self.environment('monitor')
        self.peer.operations_tick = 1000
        journal = self.root / 'completion'
        first = json.loads(self.run_cli('track_furniture_batch.py', 'start', '--journal', str(journal),
            '--batch', str(self.path), '--batch-id', identity, '--deadline-tick', '2000', '--timeout-ms', '60000'))
        self.assertEqual(first['result']['progress']['phase'], 'candidate')
        self.peer.operations_tick += 1
        done = json.loads(self.run_cli('track_furniture_batch.py', 'sample', '--journal', str(journal),
                                      '--timeout-ms', '60000'))['result']
        self.assertTrue(done['complete_original_room_furnishings_sampled_condition'])
        self.assertEqual(done['requested_room_plan'], self.plan.json())
        self.assertEqual(done['room_origin']['room_handoff_digest'], h.digest)
        self.assertEqual(done['progress']['condition_met_count'], len(h.allocation.selections))
        self.assertFalse(done['room_completion_proven'])
        self.assertFalse(done['terrain_completion_proven'])
        self.assertEqual(len(self.peer.commits), len(h.allocation.selections))
        self.assertEqual(len(self.peer.read_sessions), 3)  # One allocation, two completion samples.
        return h

    def test_real_original_recipe_allocation_batch_and_completion_pipeline(self):
        self.pipeline()

    def test_full_32_slot_646_exclusion_multipage_pipeline(self):
        self.plan = room(count=16, dining=True, excluded=range(10000, 10646))
        initial = allocated(self.plan)
        self.peer.items = {r.candidate.id: r for r in initial.allocation.selections}
        self.peer.operations_padding = 2000
        self.spec.write_bytes(self.plan.encode())
        h = self.pipeline()
        self.assertEqual(h.allocation.request.excluded_items, tuple(range(10000, 10646)))
        self.assertLessEqual(len(h.encode()), MAX_BYTES)
        for events in self.peer.read_sessions:
            self.assertGreater(sum(e[:2] == ('operations', 'page') for e in events), 1)

    def test_shortage_retains_complete_diagnostics_but_cannot_export_any_handoff(self):
        self.peer.inventory_omitted = {100}
        raw = self.run_cli('plan_rooms.py', 'allocate', '--plan-file', str(self.spec))
        report = json.loads(raw)['result']
        self.assertEqual(report['status'], 'shortage')
        self.assertEqual(report['request'], self.plan.request().json())
        self.assertIsNone(report['handoff'])
        refused = json.loads(self.export(success=False))
        self.assertFalse(refused['ok'])
        self.assertIsNone(refused['result'])
        self.assertEqual(self.peer.commits, [])
        self.assertEqual(list(self.path.iterdir()), [])

    def test_native_failure_cannot_fallback_to_cached_or_partial_capsule(self):
        for fault in ('lost', 'release', 'digest', 'source'):
            self.peer.operations_fault = fault
            old = len(self.peer.read_sessions)
            value = json.loads(self.export(success=False))
            self.assertFalse(value['ok'])
            self.assertIsNone(value['result'])
            self.assertEqual(len(self.peer.read_sessions), old + 1)
        self.assertEqual(self.peer.commits, [])

    def test_foreign_fortress_and_changed_original_constraint_fail_closed(self):
        self.peer.operations_folder = 'different'
        self.assertFalse(json.loads(self.export(success=False))['ok'])
        self.peer.operations_folder = 'region1'
        value = self.plan.json()
        value['furniture_request']['slots'].pop()
        self.spec.write_bytes(canonical(value))
        old = self.peer.connections
        self.assertFalse(json.loads(self.export(success=False))['ok'])
        self.assertEqual(self.peer.connections, old)

    def test_room_only_geometry_changes_remain_part_of_export_identity(self):
        first = RoomFurnitureHandoff.decode(self.export())
        self.plan = room(count=1, height=4)
        self.spec.write_bytes(self.plan.encode())
        second = RoomFurnitureHandoff.decode(self.export())
        self.assertEqual(first.allocation, second.allocation)
        self.assertNotEqual(first.digest, second.digest)

    def test_narrow_export_still_reserves_complete_report(self):
        with patch.object(planner, 'encode_output', side_effect=ValueError('full report does not fit')):
            self.assertRaises(ValueError, planner.allocate_plan, self.plan, inventory.Authority.load(),
                              inventory.Budget(10000), emit='room-handoff')
        self.assertEqual(len(self.peer.read_sessions), 1)
        self.assertEqual(self.peer.commits, [])

    def test_connection_is_closed_before_cpu_projection_and_composite_encoding(self):
        clients = []
        actual = inventory.InventoryClient
        class Captured(actual):
            def __init__(self, authority, budget):
                super().__init__(authority, budget)
                clients.append(self)
        original = inventory.project
        def projection(*args, **kwargs):
            self.assertEqual(len(clients), 1)
            self.assertTrue(clients[0].closed)
            self.assertEqual(clients[0].socket.fileno(), -1)
            return original(*args, **kwargs)
        with patch.object(inventory, 'InventoryClient', Captured), patch.object(inventory, 'project', side_effect=projection):
            raw = planner.allocate_plan(self.plan, inventory.Authority.load(), inventory.Budget(10000), emit='room-handoff')
        self.assertEqual(RoomFurnitureHandoff.decode(raw).room_plan, self.plan)

    def test_final_authority_revocation_refuses_without_cached_native_output(self):
        original = planner.allocate_plan
        def revoke(*args, **kwargs):
            raw = original(*args, **kwargs)
            os.environ[inventory.OPERATIONS_TOKEN] = 'x' * 32
            return raw
        stream = io.BytesIO()
        class Stdout:
            buffer = stream
        with patch.object(planner, 'allocate_plan', side_effect=revoke), patch.object(sys, 'stdout', Stdout()):
            code = planner.main(['allocate', '--plan-file', str(self.spec), '--emit', 'room-handoff'])
        self.assertEqual(code, 2)
        self.assertFalse(json.loads(stream.getvalue())['ok'])
        self.assertEqual(len(self.peer.read_sessions), 1)

    def test_short_stdout_never_retries_or_emits_second_object(self):
        class Short:
            def __init__(self): self.calls=[]
            def write(self, raw):
                self.calls.append(raw)
                return len(raw)-1
            def flush(self): raise AssertionError('unexpected flush')
        stream = Short()
        class Stdout:
            buffer = stream
        with patch.object(sys, 'stdout', Stdout()):
            code = planner.main(['allocate', '--plan-file', str(self.spec), '--emit', 'room-handoff'])
        self.assertEqual(code, 2)
        self.assertEqual(len(stream.calls), 1)
        self.assertEqual(len(self.peer.read_sessions), 1)

    def test_expired_or_exhausted_shared_budget_refuses_before_native_contact(self):
        for kind in ('deadline', 'work_steps'):
            budget = inventory.Budget(10000)
            setattr(budget, kind, 0)
            self.assertRaises(ValueError, planner.allocate_plan, self.plan, inventory.Authority.load(), budget,
                              emit='room-handoff')
        self.assertEqual(self.peer.connections, 0)
        self.assertEqual(list(self.path.iterdir()), [])

    def test_original_offline_compile_export_formats_are_unchanged(self):
        request = self.root / 'request.json'
        request.write_bytes(canonical(self.plan.json()['intent']))
        with patch.dict(os.environ, {}, clear=True):
            for name, expected in (('plan', self.plan.encode()),
                    ('excavation', canonical(self.plan.json()['excavation_blueprint'])),
                    ('furniture-request', canonical(self.plan.request().json()))):
                raw = self.run_cli('plan_rooms.py', 'compile', '--request-file', str(request), '--emit', name)
                self.assertEqual(raw, expected)
        self.assertEqual(self.peer.connections, 0)


if __name__ == '__main__':
    unittest.main()
