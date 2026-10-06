"""Real terrain custody, placement TCP and durable batch/CLI regressions.

Terrain histories and allocation inputs are constructed with the actual codecs
and private writer, not acquired from DFHack. The joined placement peer uses
production wire codecs. These tests do not establish live-game qualification.
"""
from contextlib import contextmanager
from dataclasses import replace
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

import furniture_batch as b
import build_placement_rpc as rpc
from furniture_plan import canonical
from room_provisioning import RoomPlan
from room_terrain_origin import TerrainOrigin, ReadBudget
from terrain_furniture_handoff import TerrainFurnitureHandoff
from room_furniture_peer import Peer
import room_terrain_fixtures as f
import track_excavation as t
from terrain_furniture_fixtures import capture, room_allocation, write_history


@contextmanager
def environment(address):
    env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
    env.update({rpc.OPT_IN: '1', rpc.TOKEN: 't' * 32, rpc.ENDPOINT: address, rpc.PLACE: '1'})
    with patch.dict(os.environ, env, clear=True):
        yield


class Fixture:
    def __init__(self, root, plan=None):
        self.root = Path(root)
        self.plan = plan or f.plan()
        self.peer = Peer(room_allocation(self.plan, '127.0.0.1:1'))
        self.peer.dimensions = (32768,) * 3
        self.peer.generation = 654  # Independent map=7 and inventory=987 namespaces.
        self.address = f'{self.peer.address[0]}:{self.peer.address[1]}'
        self.terrain_path = self.root / 'terrain'
        self.batch_path = self.root / 'batch'
        self.batch_path.mkdir(mode=0o700)
        self.path = str(self.batch_path)
        self.input = self.root / 'terrain-handoff.json'
        write_history(self.terrain_path, self.plan, self.address)
        self.room = room_allocation(self.plan, self.address)
        with t.open_journal(self.terrain_path, t.Budget(60000)) as journal:
            self.terrain = TerrainFurnitureHandoff(self.room, TerrainOrigin.from_journal(journal), capture(self.plan))
        self.input.write_bytes(self.terrain.encode())
        self.original_history = self.terrain_path.read_bytes()
        self.id = None

    def initialize(self, **kwargs):
        h = self.room.allocation
        out = b.initialize(self.path, h.plan(), h.request.folder, h.request.site,
                           terrain_handoff=self.terrain, **kwargs)
        self.id = out['batch_id']
        return out

    def review(self):
        return b.review(self.path, self.id)

    def advance(self, review=None):
        review = self.review() if review is None else review
        return b.advance(self.path, self.id, review['expected_plan'], review['confirm_review'])

    def cli(self, operation, *args):
        argv = [sys.executable, b.__file__, operation, '--directory', self.path, '--timeout-ms', '60000']
        if operation == 'init':
            argv += ['--terrain-handoff', str(self.input)]
        else:
            argv += ['--batch-id', self.id]
        result = subprocess.run(argv + list(args), capture_output=True, timeout=65)
        decoded = json.loads(result.stdout)
        return result.returncode, decoded

    def close(self):
        self.peer.close()


class TerrainBatchTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.fixture = Fixture(self.temp.name)
        self.addCleanup(self.fixture.close)
        self.env = environment(self.fixture.address)
        self.env.__enter__()
        self.addCleanup(self.env.__exit__, None, None, None)

    def test_original_terrain_retained_through_every_placement_and_offline_inspect(self):
        fx = self.fixture
        initial = fx.initialize()
        self.assertTrue(initial['terrain_history_verified_this_call'])
        self.assertEqual(initial['terrain_origin']['terrain_handoff_digest'], fx.terrain.digest)
        fx.input.unlink()
        for index, step in enumerate(fx.room.allocation.plan().ordered):
            out = fx.advance()
            self.assertEqual(out['placed'], index + 1)
            self.assertTrue(out['terrain_history_verified_this_call'])
            self.assertEqual(out['room_plan'], fx.plan.json())
            self.assertEqual(fx.peer.commits[-1], 'fr-' + fx.id + '-' + step.name)
        self.assertEqual(out['status'], 'all_placed')
        self.assertEqual(fx.terrain_path.read_bytes(), fx.original_history)
        with patch('socket.socket', side_effect=AssertionError('offline contact')):
            offline = b.inspect(fx.path, fx.id)
            self.assertFalse(offline['terrain_history_verified_this_call'])
            self.assertEqual(b.export_room_plan(fx.path, fx.id), fx.plan.encode())
        with b.Batch(fx.path, b.Budget(60000)) as batch:
            self.assertEqual(batch.value['schema'], b.TERRAIN_SCHEMA)
            self.assertEqual(batch.terrain_handoff.encode(), fx.terrain.encode())
            self.assertNotIn('room_handoff', batch.value)
            self.assertNotIn('handoff', batch.value)

    def test_maximum_32_slot_646_exclusion_cli_import_and_all_placements(self):
        with tempfile.TemporaryDirectory() as temp:
            request = f.request(f.dining(16, 4))
            request['excluded_items'] = list(range(10000, 10646))
            fx = Fixture(temp, RoomPlan.compile(request))
            try:
                with environment(fx.address):
                    code, value = fx.cli('init')
                    self.assertEqual(code, 0, value)
                    fx.id = value['result']['batch_id']
                    fx.input.unlink()
                    for index, step in enumerate(fx.room.allocation.plan().ordered):
                        out = fx.advance()
                        self.assertEqual(out['placed'], index + 1)
                    code, value = fx.cli('inspect', '--allocation')
                    self.assertEqual(code, 0, value)
                    out = value['result']
                    self.assertEqual(out['status'], 'all_placed')
                    self.assertEqual(len(fx.peer.commits), 32)
                    self.assertEqual(out['room_plan'], fx.plan.json())
                    self.assertEqual(out['allocation']['request']['excluded_items'], request['excluded_items'])
                    self.assertEqual(fx.terrain_path.read_bytes(), fx.original_history)
                    self.assertLess(len(canonical(value)), b.MAX_ROOM_OUTPUT)
            finally:
                fx.close()

    def test_missing_history_prevents_initialization_without_native_or_files(self):
        fx = self.fixture
        fx.terrain_path.unlink()
        with self.assertRaises(OSError):
            fx.initialize()
        self.assertEqual(fx.peer.connections, 0)
        self.assertEqual(list(fx.batch_path.iterdir()), [])

    def test_missing_history_blocks_new_work_but_inspect_export_and_stop_remain_offline(self):
        fx = self.fixture
        fx.initialize()
        review = fx.review()
        calls = list(fx.peer.calls)
        fx.terrain_path.unlink()
        with self.assertRaises(OSError):
            fx.review()
        with self.assertRaises(OSError):
            fx.advance(review)
        out = b.inspect(fx.path, fx.id)
        self.assertFalse(out['advance_allowed'])
        self.assertFalse(out['terrain_history_verified_this_call'])
        self.assertEqual(b.export_room_plan(fx.path, fx.id), fx.plan.encode())
        self.assertTrue(b.stop(fx.path, fx.id)['stopped'])
        self.assertEqual(fx.peer.calls, calls)
        self.assertEqual(fx.peer.commits, [])

    def test_lost_commit_then_terrain_loss_recovers_original_key_without_retry(self):
        fx = self.fixture
        fx.initialize()
        review = fx.review()
        fx.peer.fault = 'lost_commit'
        with self.assertRaises(ValueError):
            fx.advance(review)
        self.assertEqual(len(fx.peer.commits), 1)
        key = fx.peer.commits[0]
        pending = b.inspect(fx.path, fx.id)
        self.assertEqual(pending['status'], 'pending_recovery')
        fx.terrain_path.unlink()
        fx.peer.fault = None
        recovered = b.recover(fx.path, fx.id, pending['pending_step'])
        self.assertEqual(recovered['placed'], 1)
        self.assertFalse(recovered['advance_allowed'])
        self.assertFalse(recovered['terrain_history_verified_this_call'])
        self.assertEqual(recovered['steps'][0]['key'], key)
        calls = list(fx.peer.calls)
        b.recover(fx.path, fx.id, pending['pending_step'])
        self.assertEqual(fx.peer.calls, calls)  # Terminal evidence is offline.
        self.assertEqual(fx.peer.commits, [key])

    def test_lost_preparation_then_terrain_loss_can_cancel_without_placement(self):
        fx = self.fixture
        fx.initialize()
        review = fx.review()
        fx.peer.fault = 'lost_prepare'
        with self.assertRaises(ValueError):
            fx.advance(review)
        pending = b.inspect(fx.path, fx.id)
        fx.terrain_path.unlink()
        fx.peer.fault = None
        result = b.recover(fx.path, fx.id, pending['pending_step'], cancel=True)
        self.assertEqual(result['status'], 'halted_cancelled')
        self.assertEqual(fx.peer.commits, [])
        self.assertEqual(len(fx.peer.records), 1)

    def test_custody_loss_after_native_preparation_blocks_commit_and_preserves_recovery(self):
        fx = self.fixture
        fx.initialize()
        review = fx.review()
        def remove(operation):
            if operation == 'PreparePlacement':
                fx.terrain_path.unlink()
        fx.peer.before_reply = remove
        with self.assertRaises(OSError):
            fx.advance(review)
        self.assertEqual(fx.peer.commits, [])
        out = b.inspect(fx.path, fx.id)
        self.assertEqual(out['status'], 'pending_recovery')
        fx.peer.before_reply = None
        out = b.recover(fx.path, fx.id, out['pending_step'], cancel=True)
        self.assertEqual(out['status'], 'halted_cancelled')
        self.assertEqual(fx.peer.commits, [])

    def test_byte_identical_inode_replacement_cannot_restore_terrain_custody(self):
        fx = self.fixture
        fx.initialize()
        fx.terrain_path.rename(fx.root / 'original')
        fx.terrain_path.write_bytes(fx.original_history)
        fx.terrain_path.chmod(0o600)
        calls = list(fx.peer.calls)
        self.assertRaises(ValueError, fx.review)
        self.assertEqual(fx.peer.calls, calls)
        self.assertEqual(b.inspect(fx.path, fx.id)['status'], 'ready')
        self.assertFalse(b.inspect(fx.path, fx.id)['advance_allowed'])

    def test_placement_source_cannot_predate_or_substitute_terrain(self):
        fx = self.fixture
        for attr, bad in (('tick', 119), ('dimensions', (256, 256, 16)), ('df', 'other')):
            with self.subTest(attr=attr):
                old = getattr(fx.peer, attr)
                setattr(fx.peer, attr, bad)
                self.assertRaises(ValueError, fx.initialize)
                self.assertEqual(list(fx.batch_path.iterdir()), [])
                setattr(fx.peer, attr, old)
        fx.initialize()
        fx.peer.tick = 119
        self.assertRaises(ValueError, fx.review)
        self.assertEqual(fx.peer.commits, [])

    def test_rehashed_manifest_cannot_discard_or_change_embedded_evidence(self):
        fx = self.fixture
        fx.initialize()
        path = fx.batch_path / 'batch.json'
        raw = path.read_bytes()
        variants = []
        value = b.unseal(raw)
        value['terrain_handoff']['fresh_map']['capture_hex'] += '00'
        variants.append(b.seal(value))
        value = b.unseal(raw)
        value['first_tick'] = 119
        variants.append(b.seal(value))
        value = b.unseal(raw)
        del value['terrain_handoff']
        variants.append(b.seal(value))
        calls = list(fx.peer.calls)
        for changed in variants:
            path.write_bytes(changed)
            self.assertRaises(ValueError, b.Batch, fx.path, b.Budget(10000))
        path.write_bytes(raw)
        self.assertEqual(fx.peer.calls, calls)
        self.assertEqual(b.inspect(fx.path, fx.id)['batch_id'], fx.id)

    def test_history_identity_changes_batch_and_keys_even_for_same_furniture(self):
        fx = self.fixture
        fx.initialize()
        fx.advance()
        original_key = fx.peer.commits[0]
        path = fx.batch_path / 'batch.json'
        value = b.unseal(path.read_bytes())
        value['terrain_handoff']['terrain_origin']['journal_sha256'] = 'c' * 64
        path.write_bytes(b.seal(value))
        # Pure artifact can structurally represent a different claimed history,
        # but old indexed keys are no longer members of that changed batch.
        self.assertRaises(ValueError, b.Batch, fx.path, b.Budget(10000))
        self.assertEqual(fx.peer.commits, [original_key])

    def test_changed_item_constraints_and_stale_confirmation_cannot_commit(self):
        fx = self.fixture
        fx.initialize()
        review = fx.review()
        fx.peer.change = lambda cap: replace(cap, item=replace(cap.item, material_index=99))
        self.assertRaises(ValueError, fx.advance, review)
        self.assertEqual(fx.peer.commits, [])
        self.assertEqual(list((fx.batch_path / 'effects').iterdir()), [])
        fx.peer.change = lambda cap: cap
        fx.peer.tick += 1
        self.assertRaises(ValueError, fx.advance, review)
        self.assertEqual(fx.peer.commits, [])

    def test_shared_work_and_rpc_budgets_never_renew_before_new_work(self):
        fx = self.fixture
        fx.initialize()
        budget = b.Budget(60000)
        budget._terrain_origin_work_left = 0
        calls = list(fx.peer.calls)
        self.assertRaises(ValueError, b.review, fx.path, fx.id, _budget=budget)
        self.assertEqual(fx.peer.calls, calls)
        budget = b.Budget(60000, max_calls=1)
        self.assertRaises(ValueError, b.review, fx.path, fx.id, _budget=budget)
        self.assertEqual(fx.peer.commits, [])
        self.assertEqual(budget.calls_left, 0)

    def test_final_cli_serialization_loss_refuses_after_durable_placement_without_retry(self):
        fx = self.fixture
        fx.initialize()
        review = fx.review()
        original = b.encoded
        def encode(operation, out, ok=True):
            raw = original(operation, out, ok)
            # Called internally and by CLI; the second advanced-step render is
            # after the batch owner closed and must still be terrain-checked.
            if out.get('advanced_step') and fx.terrain_path.exists():
                encode.calls += 1
                if encode.calls == 2:
                    fx.terrain_path.unlink()
            return raw
        encode.calls = 0
        stream = io.BytesIO()
        class Output:
            buffer = stream
        with patch.object(b, 'encoded', encode), patch.object(sys, 'stdout', Output()):
            code = b.main(['advance', '--directory', fx.path, '--batch-id', fx.id,
                '--expected-plan', review['expected_plan'], '--confirm-review', review['confirm_review']])
        self.assertEqual(code, 2)
        self.assertFalse(json.loads(stream.getvalue())['ok'])
        self.assertEqual(len(fx.peer.commits), 1)
        out = b.inspect(fx.path, fx.id)
        self.assertEqual(out['placed'], 1)
        self.assertFalse(out['advance_allowed'])
        self.assertEqual(len(fx.peer.commits), 1)

    def test_short_stdout_is_one_write_and_never_another_effect(self):
        fx = self.fixture
        fx.initialize()
        review = fx.review()
        writes = []
        class Short:
            @property
            def buffer(self):
                return self
            def write(self, raw):
                writes.append(raw)
                return len(raw) - 1
        with patch.object(sys, 'stdout', Short()):
            code = b.main(['advance', '--directory', fx.path, '--batch-id', fx.id,
                '--expected-plan', review['expected_plan'], '--confirm-review', review['confirm_review']])
        self.assertEqual(code, 2)
        self.assertEqual(len(writes), 1)
        self.assertEqual(len(fx.peer.commits), 1)
        self.assertEqual(b.inspect(fx.path, fx.id)['placed'], 1)

    def test_native_authority_revocation_remains_checked_after_effect(self):
        fx = self.fixture
        fx.initialize()
        review = fx.review()
        def revoke(operation):
            if operation == 'CommitPlacement':
                os.environ.pop(rpc.TOKEN)
        fx.peer.before_reply = revoke
        self.assertRaises(ValueError, fx.advance, review)
        self.assertEqual(len(fx.peer.commits), 1)
        out = b.inspect(fx.path, fx.id)
        self.assertEqual(out['status'], 'pending_recovery')
        fx.peer.before_reply = None
        with environment(fx.address):
            self.assertEqual(b.recover(fx.path, fx.id, out['pending_step'])['placed'], 1)
        self.assertEqual(len(fx.peer.commits), 1)

    def test_cli_rejects_selector_overrides_and_legacy_does_not_consume_new_format(self):
        fx = self.fixture
        code, value = fx.cli('init', '--world-folder', 'other')
        self.assertEqual(code, 2)
        self.assertFalse(value['ok'])
        self.assertEqual(fx.peer.connections, 0)
        self.assertEqual(list(fx.batch_path.iterdir()), [])
        # The source-selected room decoder never strips the terrain layer.
        self.assertRaises(ValueError, b.RoomFurnitureHandoff.decode, fx.terrain.encode())


if __name__ == '__main__':
    unittest.main()
