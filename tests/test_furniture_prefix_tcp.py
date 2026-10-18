"""Original prefix execution with real files, owners, RPC and joined TCP peers.

No backend or native-client doubles: only the simulated native game source and
explicit fault hooks are synthetic. Beads: df-dfhack-bridge-plane-c-pic.4/.5.
"""
from __future__ import annotations

from dataclasses import replace
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1] / 'scripts'
sys.path.insert(0, str(SCRIPTS))
import furniture_batch as batch
import run_furniture_batch as run
import build_placement_rpc as rpc
import build_placement_wire as wire
from furniture_plan import FurniturePlan, Step, canonical
from furniture_recovery_peer import Peer


class Buffer:
    def __init__(self, short=False):
        self.buffer, self.short, self.writes, self.data = self, short, 0, b''

    def write(self, raw):
        self.writes += 1
        self.data += raw
        return len(raw) - int(self.short)

    def flush(self):
        pass


class PrefixTCPTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = str(Path(self.directory.name).resolve())
        self.peer = self.enterContext(Peer())
        clean = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
        self.enterContext(patch.dict(os.environ, {**clean, **self.peer.environment()}, clear=True))

    def initialize(self, count=4, dense=False):
        names = [f'item{i:02d}' + ('-' + 'a' * 41 if dense else '') for i in range(count)]
        self.plan = FurniturePlan(tuple(Step(name, ('bed', 'chair', 'table')[i % 3], i + 1,
            (20 + i * 3, 20, 2), tuple(names[max(0, i - 8):i]) if dense else ())
            for i, name in enumerate(names)))
        self.id = batch.initialize(self.path, self.plan, 'recovery-fixture', 3)['batch_id']

    def review(self, count):
        return run.review(self.path, self.id, run.Budget(60000, count))

    def execute(self, count, reviewed=None, budget=None):
        reviewed = reviewed or self.review(count)
        return run.run(self.path, self.id, reviewed['prefix_review']['confirm_run'],
                       budget or run.Budget(60000, count))

    def files(self):
        return {str(p.relative_to(self.path)): p.read_bytes() for p in Path(self.path).rglob('*') if p.is_file()}

    def argv(self, count, review):
        return ['run', '--directory', self.path, '--batch-id', self.id, '--steps', str(count),
                '--timeout-ms', '60000', '--confirm-run', review['prefix_review']['confirm_run']]

    def test_32_original_items_finish_in_four_exact_eight_step_runs(self):
        self.initialize(32)
        prior = {}
        for index in range(4):
            out = self.execute(8)
            self.assertEqual(out['placed'], (index + 1) * 8)
            self.assertEqual(out['plan'], self.plan.json())
            self.assertEqual(out['prefix_run']['rpc_calls_used'], 152)
            current = self.files()
            for name, raw in prior.items():
                if name.endswith('.placement'):
                    self.assertEqual(current[name], raw)
            prior = current
        self.assertEqual(out['status'], 'all_placed')
        self.assertEqual(self.peer.applied, 32)
        keys = [key for name, key in self.peer.calls if name == 'CommitPlacement']
        self.assertEqual(len(keys), len(set(keys)))
        self.assertFalse(out['construction_completion_proven'])

    def test_large_legal_dependency_plan_no_longer_stalls_on_output_reservation(self):
        self.initialize(32, dense=True)
        self.assertEqual(len(canonical(self.plan.json())), 14888)
        initial_files = self.files()
        old_prefix_would_refuse = old_single_would_refuse = False
        for index in range(4):
            review = self.review(8)
            old_prefix_would_refuse |= len(batch.encoded('review_prefix', review)) + 8 * 4096 + 8192 > batch.MAX_OUTPUT
            out = self.execute(8, review)
            original = {k: v for k, v in out.items() if not k.startswith('prefix_')}
            old_single_would_refuse |= len(batch.encoded('advance', original)) + 32768 >= batch.MAX_OUTPUT
            self.assertLessEqual(len(batch.encoded('run_prefix', out)), batch.MAX_OUTPUT)
            self.assertEqual(out['placed'], (index + 1) * 8)
        self.assertTrue(old_prefix_would_refuse)
        self.assertTrue(old_single_would_refuse)
        self.assertEqual(out['status'], 'all_placed')
        self.assertEqual(out['plan'], self.plan.json())
        self.assertEqual(self.files()['batch.json'], initial_files['batch.json'])
        self.assertEqual(self.peer.applied, 32)

    def test_second_indeterminate_effect_stops_then_explicit_query_unlocks_only_new_work(self):
        self.initialize()
        old = self.review(4)
        def uncertain_second(name, fields):
            if name == 'CommitPlacement' and self.peer.applied == 1:
                self.peer.commit_mode = 'indeterminate'
        self.peer.hook = uncertain_second
        out = self.execute(4, old)
        self.assertEqual(out['prefix_run']['stop_reason'], 'indeterminate')
        self.assertEqual(out['prefix_run']['steps_recorded'], 2)
        self.assertEqual(out['plan'], self.plan.json())
        before = self.files()
        calls = list(self.peer.calls)
        with self.assertRaises(ValueError):
            self.execute(4, old)
        self.assertEqual(self.peer.calls, calls)
        os.environ[rpc.PLACE] = '0'
        recovered = batch.recover(self.path, self.id, out['pending_step'])
        self.assertEqual(recovered['placed'], 2)
        self.assertEqual(self.peer.applied, 2)
        self.assertTrue(all(self.files()[name].startswith(raw) for name, raw in before.items()))
        self.peer.hook, self.peer.commit_mode = None, 'placed'
        os.environ[rpc.PLACE] = '1'
        with self.assertRaises(ValueError):
            self.execute(2, old)
        final = self.execute(2)
        self.assertEqual(final['status'], 'all_placed')
        self.assertEqual(self.peer.applied, 4)
        keys = [key for name, key in self.peer.calls if name == 'CommitPlacement']
        self.assertEqual(len(set(keys)), 4)

    def test_lost_second_reply_keeps_original_key_pending_and_never_retries(self):
        self.initialize()
        old = self.review(4)
        def lose_second(name, fields):
            if name == 'CommitPlacement' and self.peer.applied == 1:
                self.peer.commit_mode = 'lost'
        self.peer.hook = lose_second
        with self.assertRaises(ValueError):
            self.execute(4, old)
        out = batch.inspect(self.path, self.id)
        self.assertEqual((out['placed'], out['pending_step']), (1, 'item01'))
        self.assertEqual(self.peer.applied, 2)
        calls = list(self.peer.calls)
        with self.assertRaises(ValueError):
            self.execute(4, old)
        self.assertEqual(self.peer.calls, calls)
        os.environ[rpc.PLACE] = '0'
        batch.recover(self.path, self.id, 'item01')
        self.peer.hook, self.peer.commit_mode = None, 'placed'
        os.environ[rpc.PLACE] = '1'
        self.assertEqual(self.execute(2)['status'], 'all_placed')
        self.assertEqual(self.peer.applied, 4)

    def test_second_item_preflight_blocker_never_creates_a_child_or_skips_it(self):
        self.initialize()
        capture = self.peer.capture
        def blocked(selection):
            value = capture(selection)
            return replace(value, free_tile=False) if self.peer.applied == 1 else value
        self.peer.capture = blocked
        out = self.execute(4)
        self.assertEqual(out['prefix_run']['stop_reason'], 'preflight_blocked')
        self.assertEqual(out['prefix_run']['steps_considered'], 2)
        self.assertEqual(out['prefix_run']['steps_recorded'], 1)
        self.assertEqual(out['prefix_run']['outcomes'][-1]['blockers'], ['target_not_free'])
        self.assertEqual(len(list(Path(self.path, 'effects').glob('*.placement'))), 1)
        self.assertEqual(self.peer.applied, 1)

    def test_changed_capture_between_review_and_advance_prevents_first_intent(self):
        self.initialize()
        reads = 0
        def change(name, fields):
            nonlocal reads
            if name == 'ReadPlacement':
                reads += 1
                if reads == 2:
                    self.peer.sequence += 1
        self.peer.hook = change
        with self.assertRaisesRegex(ValueError, 'confirmed plan differs'):
            self.execute(4)
        self.assertEqual(self.peer.applied, 0)
        self.assertEqual(list(Path(self.path, 'effects').iterdir()), [])

    def test_raced_preparation_cannot_restore_a_commit_permit(self):
        self.initialize()
        def race(name, fields):
            if name == 'PreparePlacement':
                key = fields[10].decode('ascii')
                selected = wire.Selection(*(fields[n] for n in range(5, 10)))
                plan = wire.Plan(key, self.peer.capture(selected))
                self.peer.records[key] = wire.Record(plan, 'prepared', 'none')
        self.peer.hook = race
        with self.assertRaisesRegex(ValueError, 'replayed preparation'):
            self.execute(4)
        self.assertEqual(self.peer.applied, 0)
        self.assertFalse(any(name == 'CommitPlacement' for name, _ in self.peer.calls))
        out = batch.inspect(self.path, self.id)
        self.assertEqual(out['pending_step'], 'item00')
        self.peer.hook = None
        out = batch.recover(self.path, self.id, 'item00', cancel=True)
        self.assertEqual(out['status'], 'halted_cancelled')
        self.assertEqual(self.peer.applied, 0)

    def test_real_stop_between_existing_advance_calls_fences_next_step(self):
        self.initialize()
        original = batch.advance
        def advance_then_stop(*args, **kwargs):
            out = original(*args, **kwargs)
            batch.stop(self.path, self.id)
            return out
        with patch.object(batch, 'advance', advance_then_stop), self.assertRaises(ValueError):
            self.execute(4)
        self.assertTrue(batch.inspect(self.path, self.id)['stopped'])
        self.assertEqual(self.peer.applied, 1)
        self.assertEqual(len(list(Path(self.path, 'effects').iterdir())), 1)

    def test_source_change_after_first_effect_stops_before_second_intent(self):
        self.initialize()
        def change(name, fields):
            if name == 'ReadPlacement' and self.peer.applied == 1:
                self.peer.generation += 1
        self.peer.hook = change
        with self.assertRaises(ValueError):
            self.execute(4)
        self.assertEqual(self.peer.applied, 1)
        self.assertEqual(batch.inspect(self.path, self.id)['placed'], 1)

    def test_revoked_placement_authority_preserves_second_intent_without_commit(self):
        self.initialize()
        def revoke(name, fields):
            if name == 'PreparePlacement' and self.peer.applied == 1:
                os.environ[rpc.PLACE] = '0'
        self.peer.hook = revoke
        with self.assertRaises(ValueError):
            self.execute(4)
        out = batch.inspect(self.path, self.id)
        self.assertEqual((out['placed'], out['pending_step']), (1, 'item01'))
        self.assertFalse(out['steps'][1]['dispatch_recorded'])
        self.assertEqual(self.peer.applied, 1)

    def test_shared_call_budget_does_not_renew_after_first_effect(self):
        self.initialize()
        budget = run.Budget(60000, 4)
        def consume(name, fields):
            if name == 'CommitPlacement':
                budget.calls_left = run.RPC_PER_STEP - 1
        self.peer.hook = consume
        with self.assertRaisesRegex(ValueError, 'cannot reserve another'):
            self.execute(4, budget=budget)
        self.assertEqual(self.peer.applied, 1)
        self.assertEqual(batch.inspect(self.path, self.id)['placed'], 1)

    def test_escaped_blocker_byte_bound_refuses_before_dispatch(self):
        self.initialize()
        original = batch.review
        def oversized(*args, **kwargs):
            out = original(*args, **kwargs)
            out.update(blockers=['\U0001f680' * 128], expected_plan=None, confirm_review=None)
            return out
        with patch.object(batch, 'review', oversized), self.assertRaisesRegex(ValueError, 'blocker disclosure'):
            self.execute(4)
        self.assertEqual(self.peer.applied, 0)
        self.assertEqual(list(Path(self.path, 'effects').iterdir()), [])

    def test_small_output_limit_refuses_before_native_contact(self):
        self.initialize()
        calls, files = list(self.peer.calls), self.files()
        with patch.object(batch, 'MAX_OUTPUT', 1024), self.assertRaises(ValueError):
            self.review(4)
        self.assertEqual(self.peer.calls, calls)
        self.assertEqual(self.files(), files)

    def test_subprocess_cli_executes_actual_eight_step_owner_path(self):
        self.initialize(8)
        review = self.review(8)
        calls = len(self.peer.calls)
        process = subprocess.run([sys.executable, '-W', 'error', str(SCRIPTS / 'run_furniture_batch.py'),
            *self.argv(8, review)], capture_output=True, timeout=30)
        self.assertEqual(process.returncode, 0, process.stderr or process.stdout)
        reply = json.loads(process.stdout)
        self.assertEqual(reply['result']['status'], 'all_placed')
        self.assertEqual(reply['result']['plan'], self.plan.json())
        self.assertEqual(reply['result']['prefix_run']['rpc_calls_used'], 152)
        self.assertEqual(self.peer.applied, 8)
        self.assertEqual(sum(n == 'CommitPlacement' for n, _ in self.peer.calls[calls:]), 8)
        self.assertNotIn(self.peer.environment()[rpc.TOKEN].encode(), process.stdout)

    def test_short_final_write_cannot_repeat_a_placement(self):
        self.initialize(2)
        review = self.review(2)
        output = Buffer(short=True)
        with patch.object(run.sys, 'stdout', output):
            code = run.main(self.argv(2, review))
        self.assertEqual(code, 2)
        self.assertEqual(output.writes, 1)
        self.assertEqual(self.peer.applied, 2)
        self.assertEqual(batch.inspect(self.path, self.id)['status'], 'all_placed')

    def test_authority_loss_at_final_rendering_retains_receipts_and_refuses_success(self):
        self.initialize(2)
        review = self.review(2)
        original = batch.encoded
        def revoke(operation, out, ok=True):
            raw = original(operation, out, ok)
            if out.get('prefix_run', {}).get('stop_reason') == 'all_placed':
                os.environ[rpc.PLACE] = '0'
            return raw
        output = Buffer()
        with patch.object(batch, 'encoded', revoke), patch.object(run.sys, 'stdout', output):
            code = run.main(self.argv(2, review))
        self.assertEqual(code, 2)
        self.assertEqual(output.writes, 1)
        self.assertIsNone(json.loads(output.data)['result'])
        self.assertEqual(self.peer.applied, 2)
        self.assertEqual(batch.inspect(self.path, self.id)['status'], 'all_placed')

    def test_short_fragmented_replies_use_actual_frame_reader(self):
        self.initialize(2)
        self.peer.fragment = 17
        out = self.execute(2)
        self.assertEqual(out['status'], 'all_placed')
        self.assertEqual(self.peer.applied, 2)


if __name__ == '__main__':
    unittest.main()
