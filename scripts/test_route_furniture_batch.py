"""Actual private files, route evaluation and existing placement coordinator.

Only native I/O is injected. Real wire transport has its own integration suite.
"""
from dataclasses import replace
import io
import json
import os
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch

import build_placement_client as placement
import build_placement_rpc as native
import build_placement_store as storage
from build_placement_wire import Plan, Record
import route_furniture_batch as runtime
from route_furniture_batch_store import Batch, HEADER
import route_furniture_frontier as frontier
from test_route_furniture_frontier import fixture, capture, receipt, source, terrain


class Owner:
    address = ('127.0.0.1', 5000)
    def __init__(self):
        self.valid = True
    def guard(self, operation='QueryPlacement'):
        if not self.valid:
            raise ValueError('revoked')


class World:
    def __init__(self):
        self.trace, self.records, self.placed = [], {}, 0
        self.lose_commit, self.unknown = False, False
        self.blocked, self.route_calls, self.fail_route = (), 0, None
        self.owner = Owner()
        self.manifest = native.Manifest(11, 'df', 'dfhack')
        self.review, self.items, _ = fixture()
        self.before_prepare = lambda: None
        self.before_commit = lambda: None

    def acquire(self, batch, before, authority, budget):
        self.trace.append('route')
        self.route_calls += 1
        if self.route_calls == self.fail_route:
            raise ValueError('route source lost')
        raw = terrain(blocked=self.blocked)
        candidates = self.items[self.placed:]
        return frontier.assess(batch.review, batch.id, batch.placed_prefix(), before, candidates,
            {item.id: 30 if item.kind == 'bed' else 31 for item in candidates}, raw,
            source(raw, horizons=(100 + self.placed, 200 + self.placed, 300)), budget.remaining)

    def client_type(self):
        world = self
        class Client:
            def __init__(self, authority, budget, selection):
                self.authority, self.budget, self.selection = authority, budget, selection
                self.manifest = world.manifest
                authority.guard('Handshake')
                world.trace.append('open')
            def observe(self):
                self.authority.guard('ReadPlacement')
                world.trace.append('observe')
                before = capture(self.selection.kind - 1, sequence=world.placed)
                before = replace(before, next_job=100 + world.placed, next_building=200 + world.placed,
                                 building_count=world.placed)
                return native.Reply(world.manifest, False, len(world.records), capture=before)
            def query(self, plan):
                world.trace.append('query')
                self.authority.guard('QueryPlacement')
                return native.Reply(world.manifest, False, len(world.records),
                                    record=None if world.unknown else world.records.get(plan.key))
            def prepare(self, plan):
                world.trace.append('prepare')
                self.authority.guard('PreparePlacement')
                world.before_prepare()
                value = Record(plan, 'prepared', 'none')
                world.records[plan.key] = value
                return native.Reply(world.manifest, False, len(world.records), record=value, replayed=False)
            def commit(self, plan):
                world.trace.append('commit')
                self.authority.guard('CommitPlacement')
                world.before_commit()
                batch_id = plan.key[4:68]
                value = Record.decode(receipt(plan.before, batch=batch_id), plan)
                world.records[plan.key] = value
                world.placed += 1
                if world.lose_commit:
                    raise OSError('lost commit reply')
                return native.Reply(world.manifest, False, len(world.records), record=value)
            def cancel(self, plan):
                world.trace.append('cancel')
                self.authority.guard('CancelPlacement')
                value = world.records.get(plan.key)
                if value is None or value.phase == 'prepared':
                    value = Record(plan, 'cancelled', 'cancelled')
                    world.records[plan.key] = value
                return native.Reply(world.manifest, False, len(world.records), record=value)
            def __enter__(self):
                return self
            def __exit__(self, *_):
                world.trace.append('close')
        return Client


class BatchTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name).resolve()
        self.root.chmod(0o700)
        self.world = World()
        self.patches = [patch.object(native, 'Client', self.world.client_type()),
                        patch.object(placement, 'Client', self.world.client_type())]
        for p in self.patches:
            p.start()
        with Batch(str(self.root), None, runtime.Budget(60000), writable=True,
                   initial=(self.world.review, self.world.manifest, capture())) as batch:
            self.batch_id = batch.id
        self.world.trace.clear()
    def tearDown(self):
        for p in reversed(self.patches):
            p.stop()
        self.temp.cleanup()
    def open(self, writable=False):
        return Batch(str(self.root), self.batch_id, runtime.Budget(60000), writable=writable)
    def reviewed(self):
        with self.open() as batch:
            return runtime.review_step(batch, self.world.owner, self.world.acquire)
    def advance(self, reviewed):
        with self.open(True) as batch:
            return runtime.advance(batch, reviewed['expected_plan'], reviewed['confirm_review'],
                                   self.world.owner, self.world.acquire)
    def hashes(self):
        return {str(p.relative_to(self.root)): p.read_bytes() for p in self.root.rglob('*') if p.is_file()}

    def test_two_steps_survive_reopen_with_original_routes_and_exact_keys(self):
        for count, name in enumerate(('bed', 'chair'), 1):
            reviewed = self.reviewed()
            out = self.advance(reviewed)
            self.assertEqual(out['placed'], count)
            self.assertEqual(out['advanced_step'], name)
            self.assertTrue(out['route_constraints_checked_this_call'])
            self.assertEqual(self.world.trace.count('commit'), count)
        with self.open() as batch:
            out = batch.audit()
            self.assertEqual(out['status'], 'all_placed')
            self.assertFalse(out['advance_allowed'])
            self.assertFalse(out['route_constraints_checked_this_call'])
            self.assertEqual(batch.review.raw, self.world.review.raw)
        self.assertEqual(self.world.route_calls, 6)  # Review, before intent, before commit per step.

    def test_index_and_dispatch_sync_precede_native_commit_and_last_route_gate(self):
        reviewed = self.reviewed()
        observed = []
        def inspect_prepare():
            indexed = (self.root / 'index.jsonl').read_bytes()
            self.assertIn(b'route_evidence_digest', indexed)
            observed.append('registered')
        def inspect_commit():
            raw = next((self.root / 'effects').glob('*.placement')).read_bytes()
            self.assertTrue(storage.replay(raw).dispatched)
            self.assertEqual(self.world.route_calls, 3)
            observed.append('dispatched')
        self.world.before_prepare, self.world.before_commit = inspect_prepare, inspect_commit
        self.advance(reviewed)
        self.assertEqual(observed, ['registered', 'dispatched'])

    def test_wrong_confirmation_creates_no_child_or_index_entry(self):
        reviewed = self.reviewed()
        prior = self.hashes()
        reviewed['confirm_review'] = 'd' * 64
        with self.assertRaises(ValueError):
            self.advance(reviewed)
        self.assertEqual(self.hashes(), prior)
        self.assertNotIn('prepare', self.world.trace)
        self.assertNotIn('commit', self.world.trace)

    def test_blocked_later_route_prevents_first_preparation(self):
        reviewed = self.reviewed()
        self.world.blocked = (14,)
        with self.assertRaises(ValueError):
            self.advance(reviewed)
        self.assertNotIn('prepare', self.world.trace)
        self.assertEqual((self.root / 'index.jsonl').read_bytes(), HEADER)

    def test_final_route_failure_keeps_dispatch_intent_and_never_retries(self):
        reviewed = self.reviewed()
        self.world.fail_route = 3
        with self.assertRaises(ValueError):
            self.advance(reviewed)
        with self.open() as batch:
            out = batch.audit()
            self.assertEqual(out['status'], 'pending_recovery')
            self.assertTrue(out['steps'][0]['dispatch_recorded'])
        before = self.world.route_calls
        with self.assertRaises(ValueError):
            self.advance(reviewed)
        self.assertEqual(self.world.route_calls, before)
        self.assertNotIn('commit', self.world.trace)

    def test_lost_native_reply_recovers_exact_original_without_another_commit(self):
        reviewed = self.reviewed()
        self.world.lose_commit = True
        with self.assertRaises(OSError):
            self.advance(reviewed)
        with self.open(True) as batch:
            result = runtime.recover(batch, 'bed', self.world.owner)
            self.assertEqual(result['placed'], 1)
            self.assertTrue(result['advance_allowed'])
        self.assertEqual(self.world.trace.count('commit'), 1)

    def test_unknown_query_preserves_pending_work_and_blocks_new_step(self):
        reviewed = self.reviewed()
        self.world.lose_commit = True
        with self.assertRaises(OSError):
            self.advance(reviewed)
        self.world.unknown = True
        with self.open(True) as batch:
            result = runtime.recover(batch, 'bed', self.world.owner)
            self.assertEqual(result['status'], 'pending_recovery')
            self.assertFalse(result['advance_allowed'])
        self.assertEqual(self.world.trace.count('commit'), 1)

    def test_unregistered_crash_child_can_only_recover_not_dispatch(self):
        with self.open(True) as batch:
            p = Plan(batch.key(batch.plan.ordered[0]), capture())
            batch.effects.create(p, self.world.manifest, self.world.owner.address)
        with self.open(True) as batch:
            self.assertFalse(batch.audit()['advance_allowed'])
            out = runtime.recover(batch, 'bed', self.world.owner, cancel=True)
            self.assertEqual(out['status'], 'halted_cancelled')
            self.assertIsNone(batch.entries[0]['route_evidence_digest'])
        self.assertNotIn('commit', self.world.trace)
        self.assertNotIn('prepare', self.world.trace)

    def test_deleted_or_replaced_registered_child_is_refused(self):
        self.advance(self.reviewed())
        p = next((self.root / 'effects').glob('*.placement'))
        raw = p.read_bytes()
        p.unlink()
        with self.assertRaises(ValueError):
            self.open()
        p.write_bytes(raw)
        p.chmod(0o600)
        # Ensure a replacement inode rather than relying on filesystem reuse.
        replacement = self.root / 'replacement'
        replacement.write_bytes(raw)
        replacement.chmod(0o600)
        os.replace(replacement, p)
        with self.assertRaises(ValueError):
            self.open()

    def test_index_torn_or_rechecksummed_reordered_data_not_repaired(self):
        self.advance(self.reviewed())
        p = self.root / 'index.jsonl'
        raw = p.read_bytes()
        p.write_bytes(raw[:-1])
        with self.assertRaises(ValueError):
            self.open()
        self.assertEqual(p.read_bytes(), raw[:-1])

    def test_stop_survives_reopen_and_never_cancels_a_native_effect(self):
        with self.open(True) as batch:
            self.assertFalse(batch.stop()['advance_allowed'])
        with self.open() as batch:
            self.assertTrue(batch.audit()['stopped'])
        with self.assertRaises(ValueError):
            self.reviewed()
        self.assertEqual(self.world.trace, [])

    def test_wrong_digest_copy_symlink_modes_and_foreign_root_are_refused(self):
        with self.assertRaises(ValueError):
            Batch(str(self.root), 'e' * 64, runtime.Budget())
        # The default native client cannot accidentally interpret this root as
        # a legacy effect directory and omit the route contract.
        with self.assertRaises(ValueError):
            storage.PlacementDirectory(str(self.root), native.Budget(10000))
        p = self.root / 'batch.json'
        p.chmod(0o644)
        with self.assertRaises(ValueError):
            self.open()
        p.chmod(0o600)
        with self.open() as batch:
            self.assertEqual(batch.id, self.batch_id)
        foreign = self.root / 'foreign'
        foreign.write_text('x')
        with self.assertRaises(ValueError):
            self.open()

    def test_file_change_fences_live_owner_even_after_bytes_restored(self):
        with self.open() as batch:
            p = self.root / 'index.jsonl'
            old = p.read_bytes()
            p.write_bytes(old + b'x')
            with self.assertRaises(ValueError):
                batch.check()
            p.write_bytes(old)
            with self.assertRaises(ValueError):
                batch.check()

    def test_index_sync_failure_never_prepares_or_commits(self):
        reviewed = self.reviewed()
        original = os.fsync
        def fsync(fd):
            # Index starts as HEADER and is appended only by route registration.
            try:
                if Path(f'/proc/self/fd/{fd}').resolve() == self.root / 'index.jsonl':
                    raise OSError('index sync failed')
            except FileNotFoundError:
                pass
            return original(fd)
        with patch.object(os, 'fsync', fsync), self.assertRaises(OSError):
            self.advance(reviewed)
        self.assertNotIn('prepare', self.world.trace)
        self.assertNotIn('commit', self.world.trace)
        with self.open() as batch:
            self.assertEqual(batch.audit()['status'], 'pending_recovery')

    def test_expired_budget_and_revoked_authority_make_no_new_effect(self):
        reviewed = self.reviewed()
        self.world.owner.valid = False
        with self.assertRaises(ValueError):
            self.advance(reviewed)
        budget = runtime.Budget()
        budget.deadline = time.monotonic() - 1
        with self.assertRaises(ValueError):
            Batch(str(self.root), self.batch_id, budget)
        self.assertNotIn('prepare', self.world.trace)

    def test_authority_isolation_and_explicit_placement_permission(self):
        environment = {runtime.OPT_IN: '1', runtime.ENDPOINT: '127.0.0.1:5000',
                       native.TOKEN: 'b'*32, runtime.MAP_TOKEN: 'm'*32, runtime.OPS_TOKEN: 'o'*32}
        with patch.dict(os.environ, environment, clear=True):
            a = runtime.Authority.load()
            a.guard()
            self.assertNotIn('b'*32, repr(a))
            with self.assertRaises(ValueError):
                a.guard('CommitPlacement')
            with self.assertRaises(ValueError):
                native.Authority.load()
            os.environ[runtime.PLACE] = '1'
            b = runtime.Authority.load(True)
            b.guard('CommitPlacement')
            with self.assertRaises(ValueError):
                a.guard()
            os.environ[runtime.PLACE] = '0'
            with self.assertRaises(ValueError):
                b.guard('CommitPlacement')

    def test_cli_offline_inspection_and_stop_keep_original_custody(self):
        class Output:
            def __init__(self): self.buffer = io.BytesIO()
        output = Output()
        with patch.object(runtime.sys, 'stdout', output), patch.dict(os.environ, {}, clear=True):
            self.assertEqual(runtime.main(['inspect', '--directory', str(self.root), '--batch-id', self.batch_id]), 0)
        result = json.loads(output.buffer.getvalue())
        self.assertEqual(result['result']['review_digest'], self.world.review.digest)
        self.assertEqual(self.world.trace, [])

    def test_short_stdout_is_one_attempt_with_no_native_contact(self):
        class Short:
            def __init__(self): self.buffer = self; self.calls = 0
            def write(self, raw): self.calls += 1; return 1
            def flush(self): pass
        output = Short()
        with patch.object(runtime.sys, 'stdout', output):
            self.assertEqual(runtime.main(['inspect', '--directory', str(self.root), '--batch-id', self.batch_id]), 2)
        self.assertEqual(output.calls, 1)
        self.assertEqual(self.world.trace, [])


if __name__ == '__main__':
    unittest.main()
