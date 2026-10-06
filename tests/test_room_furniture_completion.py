"""Room-backed placement -> real completion CLI, journals and joined TCP reads."""
from dataclasses import replace
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
import furniture_completion as completion
import construction_plan as condition
import construction_monitor_rpc as monitor_rpc
import construction_plan_rpc as rpc
import furniture_completion_store as store
import track_furniture_batch as cli
from build_placement_wire import Record
import build_placement_rpc as placement_rpc
from test_room_furniture_handoff import room, allocated
from room_operations_peer import Peer, PAGE

ROOT = Path(__file__).resolve().parents[1]


class RoomCompletionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.path = self.root / 'batch'
        self.path.mkdir(mode=0o700)
        self.journal = self.root / 'completion'
        self.plan = room(count=1)
        self.peer = Peer(allocated(self.plan))
        self.addCleanup(self.peer.close)
        self.address = f'{self.peer.address[0]}:{self.peer.address[1]}'
        self.handoff = allocated(self.plan, address=self.address)
        env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
        env['PYTHONDONTWRITEBYTECODE'] = '1'
        self.env = patch.dict(os.environ, env, clear=True)
        self.env.start()
        self.addCleanup(self.env.stop)

    def environment(self, mode):
        for name in list(os.environ):
            if name.startswith('DFMCP_'):
                del os.environ[name]
        if mode == 'placement':
            os.environ.update({placement_rpc.OPT_IN: '1', placement_rpc.TOKEN: 't' * 32,
                               placement_rpc.ENDPOINT: self.address, placement_rpc.PLACE: '1'})
        elif mode == 'monitor':
            os.environ.update({monitor_rpc.OPT_IN: '1', monitor_rpc.BUILD_TOKEN: 't' * 32,
                               monitor_rpc.OPERATIONS_TOKEN: 'o' * 32, monitor_rpc.ENDPOINT: self.address})
        self.peer.mode = mode if mode != 'offline' else 'monitor'

    def placed(self, count=None):
        self.environment('placement')
        h = self.handoff.allocation
        out = batch.initialize(str(self.path), h.plan(), 'region1', 2, room_handoff=self.handoff)
        self.identity = out['batch_id']
        for _ in range(len(h.plan().steps) if count is None else count):
            review = batch.review(str(self.path), self.identity)
            out = batch.advance(str(self.path), self.identity, review['expected_plan'], review['confirm_review'])
        self.environment('monitor')
        return out

    def call(self, operation, *extra, path=None, success=True):
        args = [operation, '--journal', str(path or self.journal), '--timeout-ms', '60000', *extra]
        out = subprocess.run([sys.executable, str(ROOT / 'scripts/track_furniture_batch.py'), *args],
                             capture_output=True, timeout=70)
        self.assertEqual(out.returncode, 0 if success else 2, out.stdout + out.stderr)
        value = json.loads(out.stdout)
        self.assertEqual(value['ok'], success)
        return value

    def start(self, **kwargs):
        return self.call('start', '--batch', str(self.path), '--batch-id', self.identity,
                         '--deadline-tick', '2000', **kwargs)

    def origin(self):
        with batch.Batch(str(self.path), rpc.Budget(60000)) as owner:
            return completion.Origin.from_batch(owner, self.identity)

    def test_cli_shared_completion_retains_rooms_targets_and_original_native_keys(self):
        self.placed()
        originals = {p.relative_to(self.path): p.read_bytes() for p in self.path.rglob('*') if p.is_file()}
        first = self.start()
        self.assertEqual(first['result']['progress']['phase'], 'candidate')
        self.assertEqual(first['result']['requested_room_plan'], self.plan.json())
        self.assertFalse(first['result']['room_completion_proven'])
        self.peer.operations_tick += 1
        done = self.call('sample')
        result = done['result']
        self.assertTrue(result['complete_original_room_furnishings_sampled_condition'])
        self.assertEqual(result['progress']['phase'], 'satisfied')
        self.assertEqual(len(result['targets']), 3)
        self.assertTrue(all(t['room_unit'] == 'rooms.001' and t['room_area'] == 'rooms' for t in result['targets']))
        self.assertEqual({t['placement_key'] for t in result['targets']}, set(self.peer.commits))
        for events in self.peer.read_sessions:
            release = events.index(('operations', 'release'))
            before = [e[2] for e in events[:release] if e[:2] == ('build', 'QueryPlacement')]
            after = [e[2] for e in events[release:] if e[:2] == ('build', 'QueryPlacement')]
            self.assertEqual(before, after)
            self.assertEqual(len(before), 3)
        self.environment('offline')
        reads = len(self.peer.read_sessions)
        self.assertEqual(self.call('sample')['result']['progress'], result['progress'])
        self.assertEqual(self.call('inspect')['result']['requested_room_plan'], self.plan.json())
        self.assertEqual(self.call('wait')['result']['wait']['stop_reason'], 'terminal')
        self.assertEqual(len(self.peer.read_sessions), reads)
        self.assertEqual(originals, {p.relative_to(self.path): p.read_bytes() for p in self.path.rglob('*') if p.is_file()})

    def test_partial_original_batch_cannot_start_monitor(self):
        self.placed(1)
        self.start(success=False)
        self.assertFalse(self.journal.exists())
        self.assertEqual(self.peer.read_sessions, [])

    def test_origin_and_goal_roundtrip_and_rejection_of_subset_or_wrong_magic(self):
        self.placed()
        origin = self.origin()
        self.assertEqual(origin.encode()[:8], completion.ROOM_ORIGIN_MAGIC)
        goal = completion.Goal(origin, condition.Goal(origin.receipts, 2000))
        restored = completion.Goal.decode(goal.encode())
        self.assertEqual(restored.origin.room_handoff, self.handoff)
        self.assertEqual(restored.digest, goal.digest)
        self.assertRaises(ValueError, completion.Goal, origin, condition.Goal(origin.receipts[:-1], 2000))
        for magic in (completion.GOAL_MAGIC, completion.HANDOFF_GOAL_MAGIC):
            self.assertRaises(ValueError, completion.Goal.decode, magic + goal.encode()[8:])
        for raw in (origin.encode()[:-1], origin.encode() + b'x', b'x' * (completion.MAX_ORIGIN + 1)):
            self.assertRaises(ValueError, completion.Origin.decode, raw)
        for magic in (completion.ORIGIN_MAGIC, completion.HANDOFF_ORIGIN_MAGIC):
            self.assertRaises(ValueError, completion.Origin.decode, magic + origin.encode()[8:])

    def test_changed_room_only_geometry_cannot_adopt_original_completion(self):
        self.placed()
        self.start()
        original = self.origin()
        value = batch.unseal(original.manifest_raw)
        from room_furniture_handoff import RoomFurnitureHandoff
        value['room_handoff'] = RoomFurnitureHandoff(room(count=1, height=4), self.handoff.allocation).json()
        self.assertRaises(ValueError, replace, original, manifest_raw=batch.seal(value), guard=lambda: None)
        (self.path / 'batch.json').write_bytes(batch.seal(value))
        out = self.call('sample', success=False)
        self.assertFalse(out['result']['original_placement_history_verified'])
        self.assertFalse(out['result']['complete_original_room_furnishings_sampled_condition'])
        self.assertEqual(len(self.peer.read_sessions), 1)

    def test_every_target_must_hold_together_and_paused_ticks_do_not_count(self):
        self.placed()
        self.start()
        repeated = self.call('sample')
        self.assertEqual(repeated['result']['progress']['streak'], 1)
        for offset in range(1, 4):
            self.peer.operations_tick += 1
            self.peer.unverified_items = {100 + offset % 3}
            out = self.call('sample')
            self.assertEqual(out['result']['progress']['phase'], 'active')
            self.assertEqual(out['result']['progress']['condition_met_count'], 2)
        self.peer.unverified_items.clear()
        self.peer.operations_tick += 1
        self.assertEqual(self.call('sample')['result']['progress']['streak'], 1)
        self.peer.operations_tick += 1
        self.assertTrue(self.call('sample')['result']['complete_original_room_furnishings_sampled_condition'])

    def test_lost_sample_retains_read_intent_resets_streak_and_never_retries(self):
        self.placed()
        self.start()
        self.peer.operations_fault = 'lost'
        self.peer.operations_tick += 1
        result = self.call('sample', success=False)
        self.assertIsNone(result['result']['progress'])
        self.assertEqual(len(self.peer.read_sessions), 2)
        with store.Journal(str(self.journal), rpc.Budget(60000)) as journal:
            self.assertTrue(journal.state.progress.reading)
            self.assertEqual(journal.state.progress.observations, 1)
        self.peer.operations_fault = None
        self.peer.operations_tick += 1
        next_sample = self.call('sample')['result']['progress']
        self.assertEqual((next_sample['streak'], next_sample['interrupted_reads']), (1, 1))
        self.assertEqual(len(self.peer.commits), 3)

    def test_page_release_and_original_receipt_failures_never_publish_success(self):
        self.placed()
        for fault in ('release', 'digest', 'source', 'receipt_after'):
            self.peer.operations_fault = fault
            path = self.root / ('fault-' + fault)
            self.start(path=path, success=False)
            with store.Journal(str(path), rpc.Budget(60000)) as owner:
                self.assertTrue(owner.state.progress.reading)
                self.assertEqual(owner.state.progress.observations, 0)
        self.assertEqual(len(self.peer.commits), 3)

    def test_operations_generation_remains_bound_to_original_inventory(self):
        self.placed()
        self.peer.operations_generation = 8
        self.start(success=False)
        with store.Journal(str(self.journal), rpc.Budget(60000)) as owner:
            self.assertEqual(owner.state.goal.origin.handoff.source.generation, 7)
            self.assertTrue(owner.state.progress.reading)
            self.assertEqual(owner.state.progress.observations, 0)

    def test_cancel_after_original_file_loss_keeps_complete_historical_room_without_claims(self):
        self.placed()
        self.start()
        (self.path / 'batch.json').rename(self.root / 'retained-manifest')
        self.call('inspect', success=False)
        self.environment('offline')
        out = self.call('cancel')['result']
        self.assertEqual(out['progress']['phase'], 'cancelled')
        self.assertEqual(out['requested_room_plan'], self.plan.json())
        self.assertFalse(out['original_placement_history_verified'])
        self.assertFalse(out['complete_original_room_furnishings_sampled_condition'])
        self.assertEqual(len(self.peer.read_sessions), 1)
        self.assertEqual(len(self.peer.commits), 3)

    def test_revoked_authority_after_final_serialization_withholds_success_but_keeps_evidence(self):
        self.placed()
        self.start()
        self.peer.operations_tick += 1
        original = cli.output
        def revoke(value):
            raw = original(value)
            if value['ok'] and value['result'].get('journal', {}).get('frames') != store.MAX_FRAMES:
                os.environ[monitor_rpc.OPERATIONS_TOKEN] = 'x' * 32
            return raw
        stdout = io.StringIO()
        with patch.object(cli, 'output', side_effect=revoke), patch.object(sys, 'stdout', stdout):
            code = cli.main(['sample', '--journal', str(self.journal)])
        self.assertEqual(code, 2)
        self.assertFalse(json.loads(stdout.getvalue())['ok'])
        with store.Journal(str(self.journal), rpc.Budget(60000)) as owner:
            self.assertEqual(owner.state.progress.phase, 'satisfied')
        self.assertEqual(len(self.peer.read_sessions), 2)

    def test_short_stdout_does_not_emit_a_second_object_or_repeat_acquisition(self):
        self.placed()
        class Short:
            def __init__(self): self.calls = []
            def write(self, text):
                self.calls.append(text)
                return len(text) - 1
            def flush(self): raise AssertionError('unexpected flush')
        output = Short()
        with patch.object(sys, 'stdout', output):
            code = cli.main(['start', '--journal', str(self.journal), '--batch', str(self.path),
                             '--batch-id', self.identity, '--deadline-tick', '2000'])
        self.assertEqual(code, 2)
        self.assertEqual(len(output.calls), 1)
        self.assertEqual(len(self.peer.read_sessions), 1)
        with store.Journal(str(self.journal), rpc.Budget(60000)) as owner:
            self.assertEqual(owner.state.progress.observations, 1)

    def test_maximum_room_set_and_multipage_capture_reach_shared_completion(self):
        self.plan = room(dining=True, count=16, excluded=range(10000, 10646))
        self.handoff = allocated(self.plan, address=self.address)
        self.peer.items = {r.candidate.id: r for r in self.handoff.allocation.selections}
        self.placed()
        self.peer.operations_padding = 2000
        self.assertGreater(len(self.peer.operations()), PAGE)
        first = self.start()
        self.assertEqual(len(first['result']['targets']), 32)
        self.peer.operations_tick += 1
        out = self.call('sample')['result']
        self.assertTrue(out['complete_original_room_furnishings_sampled_condition'])
        self.assertEqual(out['requested_room_plan']['intent']['excluded_items'], list(range(10000, 10646)))
        self.assertEqual(out['progress']['condition_met_count'], 32)
        self.assertEqual(len(self.peer.commits), 32)
        self.assertLessEqual(len(cli.output({'result': out})), cli.MAX_ROOM_OUTPUT)
        for events in self.peer.read_sessions:
            self.assertEqual(sum(e[:2] == ('build', 'QueryPlacement') for e in events), 64)
            self.assertGreater(sum(e[:2] == ('operations', 'page') for e in events), 1)


if __name__ == '__main__':
    unittest.main()
