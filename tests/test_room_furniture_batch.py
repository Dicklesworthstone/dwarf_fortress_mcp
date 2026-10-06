"""Original room intent through actual TCP placement, private files and CLI.

Synthetic native peer uses production wire values; no SDK/live-game/MCP claim.
"""
from contextlib import contextmanager
from dataclasses import replace
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

import furniture_batch as b
import build_placement_rpc as rpc
import build_placement_store as store
from furniture_handoff import Handoff
from furniture_plan import canonical
from room_provisioning import RoomPlan
from room_furniture_handoff import RoomFurnitureHandoff
from test_room_furniture_handoff import room, allocated
from room_furniture_peer import Peer

ROOT = Path(__file__).resolve().parents[1]


class RoomBatchTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.path = self.root / 'batch'
        self.path.mkdir(mode=0o700)
        self.room = room()
        self.peer = Peer(allocated(self.room))
        self.addCleanup(self.peer.close)
        self.handoff = allocated(self.room, address=f'{self.peer.address[0]}:{self.peer.address[1]}')
        env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
        env.update({rpc.OPT_IN: '1', rpc.TOKEN: 't' * 32, rpc.ENDPOINT: self.handoff.allocation.source.address,
                    rpc.PLACE: '1', 'PYTHONDONTWRITEBYTECODE': '1'})
        self.env = patch.dict(os.environ, env, clear=True)
        self.env.start()
        self.addCleanup(self.env.stop)

    def init(self):
        h = self.handoff.allocation
        return b.initialize(str(self.path), h.plan(), h.request.folder, h.request.site, room_handoff=self.handoff)

    def advance(self, identity):
        review = b.review(str(self.path), identity)
        self.assertEqual(review['blockers'], [])
        return b.advance(str(self.path), identity, review['expected_plan'], review['confirm_review'])

    def cli(self, *args):
        return subprocess.run([sys.executable, str(ROOT / 'scripts/furniture_batch.py'), *args],
                              capture_output=True, timeout=15)

    def test_full_original_batch_places_all_types_with_reopen_between_every_step(self):
        out = self.init()
        identity = out['batch_id']
        self.assertEqual(out['room_plan'], self.room.json())
        self.assertEqual(out['room_origin'], self.handoff.compact())
        for step in self.handoff.allocation.plan().ordered:
            with b.Batch(str(self.path), rpc.Budget(10000)) as batch:
                self.assertEqual(batch.key(step), 'fr-' + identity + '-' + step.name)
            out = self.advance(identity)
            self.assertEqual(out['advanced_step'], step.name)
            self.assertEqual(out['room_plan'], self.room.json())
            self.assertEqual(len(out['steps']), 6)
        self.assertEqual(out['status'], 'all_placed')
        self.assertEqual(len(self.peer.commits), 6)
        self.assertEqual(len(set(self.peer.commits)), 6)
        self.assertFalse(out['construction_completion_proven'])
        self.assertFalse(out['room_origin']['terrain_completion_proven'])
        self.assertEqual(b.export_room_plan(str(self.path), identity), self.room.encode())

    def test_lost_commit_fences_new_work_until_original_key_recovery_without_repeat(self):
        out = self.init()
        identity = out['batch_id']
        self.peer.fault = 'lost_commit'
        self.assertRaises(ValueError, self.advance, identity)
        pending = b.inspect(str(self.path), identity)
        self.assertEqual(pending['status'], 'pending_recovery')
        self.assertEqual(pending['room_plan'], self.room.json())
        self.assertFalse(pending['advance_allowed'])
        self.assertRaises(ValueError, b.review, str(self.path), identity)
        self.peer.fault = 'missing'
        unknown = b.recover(str(self.path), identity, pending['pending_step'])
        self.assertEqual(unknown['status'], 'pending_recovery')
        self.peer.fault = None
        recovered = b.recover(str(self.path), identity, pending['pending_step'])
        self.assertEqual(recovered['placed'], 1)
        self.assertEqual(len(self.peer.commits), 1)
        with patch.dict(os.environ, {}, clear=True):
            terminal = b.recover(str(self.path), identity, pending['pending_step'])
        self.assertFalse(terminal['native_contacted'])
        self.assertEqual(len(self.peer.commits), 1)

    def test_lost_prepare_can_be_cancelled_but_never_replayed_as_commit(self):
        identity = self.init()['batch_id']
        self.peer.fault = 'lost_prepare'
        self.assertRaises(ValueError, self.advance, identity)
        pending = b.inspect(str(self.path), identity)
        self.peer.fault = None
        queried = b.recover(str(self.path), identity, pending['pending_step'])
        self.assertEqual(queried['status'], 'pending_recovery')
        self.assertEqual(self.peer.commits, [])
        cancelled = b.recover(str(self.path), identity, pending['pending_step'], True)
        self.assertEqual(cancelled['status'], 'halted_cancelled')
        self.assertRaises(ValueError, b.review, str(self.path), identity)
        self.assertEqual(self.peer.commits, [])

    def test_forged_valid_room_change_cannot_adopt_existing_effects(self):
        identity = self.init()['batch_id']
        self.advance(identity)
        file = self.path / 'batch.json'
        original = file.read_bytes()
        value = b.unseal(original)
        # Same exact furniture allocation, different original room geometry.
        value['room_handoff'] = RoomFurnitureHandoff(room(height=4), self.handoff.allocation).json()
        file.write_bytes(b.seal(value))
        self.assertRaises(ValueError, b.Batch, str(self.path), rpc.Budget(10000))
        self.assertEqual(len(self.peer.commits), 1)
        file.write_bytes(original)
        self.assertEqual(b.inspect(str(self.path), identity)['placed'], 1)

    def test_complete_room_geometry_must_fit_before_custody_even_if_furniture_fits(self):
        self.peer.dimensions = (30, 13, 3)
        self.handoff.allocation.plan().check_dimensions(self.peer.dimensions)
        self.assertRaises(ValueError, self.init)
        self.assertEqual(list(self.path.iterdir()), [])
        self.assertNotIn('PreparePlacement', self.peer.calls)

    def test_original_constraints_and_source_refuse_changed_native_items(self):
        identity = self.init()['batch_id']
        for changes in ({'material': 9}, {'subtype': 3}, {'native_type': 99}, {'pos': (100, 100, 2)}, {'pos': (10, 10, 3)}):
            self.peer.change = lambda c, changes=changes: replace(c, item=replace(c.item, **changes))
            self.assertRaises(ValueError, b.review, str(self.path), identity)
        self.peer.change = lambda c: c
        for field, value in [('folder', 'other'), ('site', 3), ('df', 'changed'), ('generation', 988), ('tick', 899)]:
            prior = getattr(self.peer, field)
            setattr(self.peer, field, value)
            self.assertRaises(ValueError, b.review, str(self.path), identity)
            setattr(self.peer, field, prior)
        self.assertNotIn('PreparePlacement', self.peer.calls)

    def test_stale_confirmation_never_creates_a_child_intent(self):
        identity = self.init()['batch_id']
        review = b.review(str(self.path), identity)
        self.assertRaises(ValueError, b.advance, str(self.path), identity, review['expected_plan'], '0' * 64)
        self.assertEqual(list((self.path / 'effects').iterdir()), [])
        self.assertEqual(self.peer.commits, [])

    def test_missing_torn_or_replaced_original_custody_refuses_unchanged(self):
        identity = self.init()['batch_id']
        self.advance(identity)
        file = self.path / 'batch.json'
        original = file.read_bytes()
        for bad in (original[:-1], b'{}\n', b'[' * 17):
            file.write_bytes(bad)
            self.assertRaises(ValueError, b.inspect, str(self.path), identity)
            self.assertEqual(file.read_bytes(), bad)
        file.write_bytes(original)
        child = next((self.path / 'effects').iterdir())
        child.rename(self.root / 'retained-child')
        self.assertRaises(ValueError, b.inspect, str(self.path), identity)
        self.assertEqual(len(self.peer.commits), 1)

    def test_cli_import_and_offline_original_plan_export_survive_input_deletion(self):
        source = self.root / 'room.json'
        source.write_bytes(self.handoff.encode())
        result = self.cli('init', '--directory', str(self.path), '--room-handoff', str(source))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        identity = json.loads(result.stdout)['result']['batch_id']
        source.unlink()
        with patch.dict(os.environ, {}, clear=True):
            out = self.cli('inspect', '--directory', str(self.path), '--batch-id', identity, '--emit', 'room-plan')
        self.assertEqual(out.returncode, 0, out.stdout + out.stderr)
        self.assertEqual(out.stdout, self.room.encode())
        self.assertEqual(self.peer.calls, ['Handshake', 'ReadPlacement'])

    def test_cli_forbids_room_fortress_override_before_native_contact(self):
        source = self.root / 'room.json'
        source.write_bytes(self.handoff.encode())
        out = self.cli('init', '--directory', str(self.path), '--room-handoff', str(source),
                       '--world-folder', 'region1', '--site', '2')
        self.assertEqual(out.returncode, 2)
        self.assertFalse(json.loads(out.stdout)['ok'])
        self.assertEqual(self.peer.connections, 0)

    def test_final_serialization_revocation_keeps_original_receipt_without_retry(self):
        identity = self.init()['batch_id']
        review = b.review(str(self.path), identity)
        original = b.encoded
        def revoked(operation, out, ok=True):
            raw = original(operation, out, ok)
            if operation == 'advance' and 'advanced_step' in out:
                os.environ[rpc.TOKEN] = 'x' * 32
            return raw
        with patch.object(b, 'encoded', side_effect=revoked):
            self.assertRaises(ValueError, b.advance, str(self.path), identity,
                              review['expected_plan'], review['confirm_review'])
        self.assertEqual(len(self.peer.commits), 1)
        with patch.dict(os.environ, {}, clear=True):
            out = b.inspect(str(self.path), identity)
        self.assertEqual(out['placed'], 1)
        self.assertEqual(out['room_plan'], self.room.json())

    def test_short_stdout_after_native_commit_never_repeats_effect_or_error_object(self):
        identity = self.init()['batch_id']
        review = b.review(str(self.path), identity)
        class Short:
            def __init__(self):
                self.calls = []
            def write(self, raw):
                self.calls.append(raw)
                return len(raw) - 1
            def flush(self):
                raise AssertionError('flush after short output')
        output = Short()
        class Stdout:
            buffer = output
        with patch.object(sys, 'stdout', Stdout()):
            code = b.main(['advance', '--directory', str(self.path), '--batch-id', identity,
                          '--expected-plan', review['expected_plan'], '--confirm-review', review['confirm_review']])
        self.assertEqual(code, 2)
        self.assertEqual(len(output.calls), 1)
        self.assertEqual(len(self.peer.commits), 1)
        self.assertEqual(b.inspect(str(self.path), identity)['placed'], 1)

    def test_expired_shared_budget_refuses_before_native_or_disk_creation(self):
        budget = rpc.Budget(10000)
        budget.deadline = 0
        h = self.handoff.allocation
        self.assertRaises(ValueError, b.initialize, str(self.path), h.plan(), 'region1', 2,
                          room_handoff=self.handoff, _budget=budget)
        self.assertEqual(self.peer.connections, 0)
        self.assertEqual(list(self.path.iterdir()), [])

    def test_stop_is_local_and_preserves_pending_effect_and_complete_room(self):
        identity = self.init()['batch_id']
        self.peer.fault = 'lost_commit'
        self.assertRaises(ValueError, self.advance, identity)
        before = len(self.peer.calls)
        with patch.dict(os.environ, {}, clear=True):
            out = b.stop(str(self.path), identity)
        self.assertTrue(out['stopped'])
        self.assertEqual(out['status'], 'pending_recovery')
        self.assertEqual(out['room_plan'], self.room.json())
        self.assertEqual(len(self.peer.calls), before)

    def test_both_legacy_batch_formats_keep_old_keys_and_refuse_room_exports(self):
        for handoff in (None, self.handoff.allocation):
            path = self.root / ('legacy1' if handoff is None else 'legacy2')
            path.mkdir(mode=0o700)
            out = b.initialize(str(path), self.handoff.allocation.plan(), 'region1', 2, handoff=handoff)
            with b.Batch(str(path), rpc.Budget(10000)) as batch:
                self.assertEqual(batch.value['schema'], b.SCHEMA if handoff is None else b.HANDOFF_SCHEMA)
                self.assertIsNone(batch.room_handoff)
                self.assertEqual(batch.key(batch.plan.steps[0]), 'fb-' + batch.value['nonce'] + '-' + batch.plan.steps[0].name)
                self.assertNotIn('room_origin', batch.audit())
            self.assertRaises(ValueError, b.export_room_plan, str(path), out['batch_id'])
            self.assertLessEqual(len(b.encoded('init', out)), b.MAX_OUTPUT)

    def test_room_definition_must_retain_the_exact_normalized_plan(self):
        identity = self.init()['batch_id']
        file = self.path / 'batch.json'
        value = b.unseal(file.read_bytes())
        value['plan']['steps'].reverse()
        file.write_bytes(b.seal(value))
        self.assertRaises(ValueError, b.inspect, str(self.path), identity)
        self.assertEqual(self.peer.commits, [])

    def test_32_slots_646_exclusions_all_native_steps_and_views_remain_complete(self):
        plan = room(dining=True, count=16, excluded=range(10000, 10646))
        self.room = plan
        self.handoff = allocated(plan, address=self.handoff.allocation.source.address)
        self.peer.items = {r.candidate.id: r for r in self.handoff.allocation.selections}
        out = self.init()
        identity = out['batch_id']
        for i in range(32):
            out = self.advance(identity)
            self.assertEqual(out['placed'], i + 1)
            self.assertEqual(out['room_plan'], plan.json())
        self.assertEqual(out['status'], 'all_placed')
        self.assertEqual(len(self.peer.commits), 32)
        full = b.inspect(str(self.path), identity, allocation=True)
        self.assertEqual(full['allocation'], self.handoff.allocation.json())
        self.assertLessEqual(len(b.encoded('inspect', full)), b.MAX_ROOM_OUTPUT)
        self.assertEqual(b.export_room_plan(str(self.path), identity), plan.encode())


if __name__ == '__main__':
    unittest.main()
