"""Real private files and actual scope/intent/receipt codecs; no native I/O.

Original batch custody and its shrinking budget are explicit trusted test ports.
No codec, directory, filesystem, lock, or inventory algorithm is replaced.
"""
from contextlib import contextmanager
from dataclasses import replace
import hashlib
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import time
import types
import unittest
from unittest.mock import patch

import bounded_run_client as clock
import bounded_run_outcomes as outcomes
from furniture_allocation import Request, Slot, Candidate
from furniture_plan import canonical
from route_furniture_allocation import allocate_routes
from route_furniture_review import Source, create
from route_furniture_clock_context import Scope, ScopeStore, SCHEMA, POLICY
import route_furniture_clock_inventory as history


class Budget:
    def __init__(self):
        self.deadline = time.monotonic() + 60
        self.disk_bytes = 1024 * 1024 * 1024
        self.work_steps = 20_000_000
        self.work_hook = lambda: None

    def remaining(self):
        left = self.deadline - time.monotonic()
        if left <= 0:
            raise ValueError('deadline exhausted')
        return left

    def work(self):
        self.remaining()
        if self.work_steps <= 0:
            raise ValueError('work exhausted')
        self.work_steps -= 1
        self.work_hook()

    def reserve(self, kind, count):
        self.remaining()
        if kind != 'disk_bytes' or type(count) is not int or not 0 <= count <= self.disk_bytes:
            raise ValueError('disk bound')
        self.disk_bytes -= count


class Batch:
    def __init__(self, identity='a' * 64):
        raw = b'DFMM1500' + struct.pack('>IIBIH', 0, 10, 1, 2, 7) + b'region1'
        raw += struct.pack('>10I', 16, 16, 1, 1, 1, 0, 3, 3, 1, 9)
        raw += (b'\2' + struct.pack('>IBBBBBBBIHH', 1, 3, 0, 0, 0, 0, 0, 0, 1, 0, 0)) * 9
        self.source = Source('127.0.0.1:5000', 'df', 'dfhack', 7, 9,
            hashlib.sha256(raw).hexdigest(), 'b' * 64, 10,
            (16, 16, 1), (1, 1, 0), (3, 3, 1), (100, 200, 300))
        request = Request('region1', 2, (Slot('bed', 'bed', (3, 2, 0)),))
        allocation = allocate_routes(request, (Candidate(10, 'bed', (1, 2, 0), (419, -1), -1),), raw)
        self.review = create(request, allocation, self.source, {10: 30})
        self.plan = self.review.unpack()[3]
        self.address, self.id, self.budget = ('127.0.0.1', 5000), identity, Budget()
        self.writable, self.corrupt = False, False
        self.inventory = 'c' * 64
        self.verified = True
        self.next_step, self.stopped = 'bed', False

    def check(self):
        self.budget.remaining()
        if self.corrupt:
            raise ValueError('batch custody changed')

    def audit(self):
        self.check()
        return {'inventory_verified': self.verified, 'batch_id': self.id,
                'review_digest': self.review.digest, 'inventory_digest': self.inventory,
                'next_step': self.next_step, 'advance_allowed': True, 'stopped': self.stopped}


def scope_for(batch, sequence=0):
    before = b'DFMRO013' + struct.pack('>QQQBBB', 11, sequence, 10, 1, 1, 1)
    return Scope.create({'schema': SCHEMA, 'policy': POLICY,
        'batch_id': batch.id, 'review_digest': batch.review.digest, 'plan_digest': batch.plan.digest,
        'inventory_digest': batch.inventory, 'next_step': 'bed', 'frontier_evidence': 'd' * 64,
        'source': batch.source.json(), 'game_ticks': 20, 'wall_ms': 5000,
        'clock_manifest': {'generation': 11, 'df_version': 'df', 'dfhack_version': 'dfhack'},
        'clock_observation_hex': before.hex()})


def record_for(scope, phase='stopped'):
    intent = scope.intent()
    code = clock.PHASES.index(phase)
    reason = {'stopped': 1, 'refused': 9, 'source_lost': 7, 'running': 0}[phase]
    known = phase in ('stopped', 'running')
    raw = (b'DFMRE013' + clock.key_bytes(intent['idempotency_key']) + struct.pack('>II', 20, 5000)
           + bytes.fromhex(intent['observation_hex']) + bytes.fromhex(intent['plan_digest_hex'])
           + bytes.fromhex(intent['prepare_token_hex'])
           + struct.pack('>BBBBBQ', code, reason, phase != 'refused', phase == 'stopped', known, 30 if known else 0))
    raw += clock.digest(b'dfmcp-bounded-run-receipt/1', raw)
    return {'manifest': scope.value()['clock_manifest'], 'record': clock.decode_record(raw)}


def save(directory, batch, *, sequence=0, phase='stopped', with_scope=True, with_intent=True):
    scope = scope_for(batch, sequence)
    path = directory / ('progress-' + scope.digest + '.run')
    with outcomes.new_run(path) as parent:
        if with_scope:
            with ScopeStore(path, parent, batch, scope.digest, create=scope):
                pass
        if with_intent:
            with clock.capsule(path, scope.intent()):
                pass
            if phase is not None:
                with outcomes.OutcomeStore(path, parent_fd=parent) as store:
                    store.retain(record_for(scope, phase))
    return scope, path


@contextmanager
def setup():
    with tempfile.TemporaryDirectory(prefix='dfmcp-clock-inventory-') as name:
        directory = Path(name)
        os.chmod(directory, 0o700)
        yield directory, Batch()


def bytes_in(directory):
    return {p.name: p.read_bytes() for p in directory.iterdir() if p.is_file() and not p.is_symlink()}


class InventoryTests(unittest.TestCase):
    def test_empty_complete_local_history_is_not_global_or_native_authority(self):
        with setup() as (directory, batch), history.Inventory(directory, batch) as owner:
            out = owner.page()
            self.assertEqual(out['rows'], [])
            self.assertEqual(out['counts']['records'], 0)
            self.assertTrue(out['directory_history_reconciled'])
            for flag in ('native_contacted', 'files_modified', 'retry_permitted',
                         'clock_start_authorized', 'construction_completion_proven', 'production_admitted'):
                self.assertFalse(out[flag])
            self.assertTrue(out['current_pause_unproved'])

    def test_reopen_discovers_all_original_confirmations_without_network_or_writes(self):
        with setup() as (directory, batch):
            pairs = [save(directory, batch, sequence=i, phase=phase)
                     for i, phase in enumerate(('stopped', 'refused', 'source_lost', None))]
            before = bytes_in(directory)
            with patch.object(socket, 'socket', side_effect=AssertionError('offline only')), \
                 patch.object(os, 'write', side_effect=AssertionError('read only')), \
                 patch.object(os, 'fsync', side_effect=AssertionError('read only')):
                with history.Inventory(directory, batch) as owner:
                    first = owner.page()
                with history.Inventory(directory, batch) as owner:
                    second = owner.page()
            self.assertEqual(first, second)
            self.assertEqual(first['counts']['unresolved'], 2)
            self.assertEqual(first['counts']['query_supported'], 1)
            self.assertEqual({r['clock_record_id'] for r in first['rows']}, {s.digest for s, _ in pairs})
            self.assertEqual(before, bytes_in(directory))

    def test_scope_only_crash_is_visible_unresolved_and_not_a_retry_or_query_permit(self):
        with setup() as (directory, batch):
            save(directory, batch, with_intent=False)
            with history.Inventory(directory, batch) as owner:
                row = owner.page()['rows'][0]
                self.assertEqual(row['status'], 'scope_without_intent')
                self.assertFalse(row['query_supported'])
                self.assertFalse(row['resolved'])
                with self.assertRaises(ValueError):
                    owner.admit_new(scope_for(batch, 1))

    def test_unassociated_legacy_intent_is_not_adopted_even_with_terminal_receipt(self):
        with setup() as (directory, batch):
            save(directory, batch, with_scope=False)
            with history.Inventory(directory, batch) as owner:
                out = owner.page()
                self.assertEqual(out['counts']['unassociated'], 1)
                self.assertEqual(out['rows'][0]['association'], 'unbound')
                self.assertFalse(out['rows'][0]['query_supported'])
                self.assertFalse(out['directory_history_reconciled'])
                with self.assertRaises(ValueError):
                    owner.admit_new(scope_for(batch, 1))

    def test_foreign_batch_is_not_attributed_and_unresolved_foreign_history_still_blocks(self):
        with setup() as (directory, batch):
            other = Batch('e' * 64)
            save(directory, other, phase=None)
            with history.Inventory(directory, batch) as owner:
                out = owner.page(selection='terminal')
                self.assertEqual(out['rows'], [])
                self.assertEqual(out['counts']['other_batch'], 1)
                self.assertEqual(out['counts']['unresolved'], 1)
                with self.assertRaises(ValueError):
                    owner.admit_new(scope_for(batch))

    def test_all_resolved_history_allows_only_fresh_current_scope(self):
        with setup() as (directory, batch):
            old, _ = save(directory, batch)
            with history.Inventory(directory, batch) as owner:
                with self.assertRaises(ValueError): owner.admit_new(old)
                current = scope_for(batch, 1)
                owner.admit_new(current)
                self.assertEqual(len(owner._allowed_new), 3)
                with self.assertRaises(ValueError): owner.admit_new(scope_for(batch, 2))
                with self.assertRaises(ValueError): owner.page()
                owner.check()
            self.assertEqual(len(bytes_in(directory)), 3)

    def test_all_unresolved_variants_prevent_replacement_effect(self):
        for phase in (None, 'running', 'source_lost'):
            with self.subTest(phase=phase), setup() as (directory, batch):
                save(directory, batch, phase=phase)
                with history.Inventory(directory, batch) as owner:
                    with self.assertRaises(ValueError): owner.admit_new(scope_for(batch, 1))

    def test_mismatched_original_review_plan_source_and_slot_are_refused(self):
        mutations = {'review_digest': 'f' * 64, 'plan_digest': 'f' * 64, 'next_step': 'other'}
        for field, value in mutations.items():
            with self.subTest(field=field), setup() as (directory, batch):
                original = scope_for(batch).value()
                original[field] = value
                scoped = Scope.create(original)
                name = directory / ('progress-' + scoped.digest + '.run.scope.json')
                name.write_bytes(scoped.raw + b'\n'); name.chmod(0o600)
                with self.assertRaises(ValueError):
                    with history.Inventory(directory, batch): pass
        with setup() as (directory, batch):
            save(directory, batch)
            batch.source = replace(batch.source, map_generation=12)
            with self.assertRaises(ValueError):
                with history.Inventory(directory, batch): pass

    def test_native_intent_cannot_be_swapped_between_scopes(self):
        with setup() as (directory, batch):
            _, path = save(directory, batch, phase=None)
            substitute = scope_for(batch, 1).intent()
            data = canonical({'intent': substitute, 'sha256': hashlib.sha256(canonical(substitute)).hexdigest()}) + b'\n'
            path.write_bytes(data)
            with self.assertRaises(ValueError):
                with history.Inventory(directory, batch): pass

    def test_all_small_scope_truncations_and_noncanonical_values_refuse_without_repair(self):
        with setup() as (directory, batch):
            _, path = save(directory, batch, with_intent=False)
            scope_path = Path(str(path) + '.scope.json')
            original = scope_path.read_bytes()
            cuts = (0, 1, 2, 7, len(original) // 2, len(original) - 2, len(original) - 1)
            for size in cuts:
                scope_path.write_bytes(original[:size])
                with self.subTest(size=size), self.assertRaises((OSError, ValueError)):
                    with history.Inventory(directory, batch): pass
                self.assertEqual(scope_path.read_bytes(), original[:size])
            for raw in (original + b' ', original[:-1], b'{"schema":1,"schema":1}\n', b'[' * 100 + b'\n'):
                scope_path.write_bytes(raw)
                with self.assertRaises((OSError, ValueError)):
                    with history.Inventory(directory, batch): pass

    def test_corrupt_outcome_cannot_be_hidden_by_pending_filter(self):
        with setup() as (directory, batch):
            _, path = save(directory, batch)
            outcome = Path(str(path) + outcomes.SUFFIX)
            outcome.write_bytes(outcome.read_bytes()[:-2] + b'!\n')
            with self.assertRaises(ValueError):
                with history.Inventory(directory, batch) as owner: owner.page(selection='pending')

    def test_orphan_outcome_and_scope_outcome_without_intent_refuse(self):
        for keep_scope in (False, True):
            with self.subTest(keep_scope=keep_scope), setup() as (directory, batch):
                _, path = save(directory, batch)
                path.unlink()
                if not keep_scope: Path(str(path) + '.scope.json').unlink()
                before = bytes_in(directory)
                with self.assertRaises(ValueError):
                    with history.Inventory(directory, batch): pass
                self.assertEqual(before, bytes_in(directory))

    def test_unsupported_names_symlinks_hardlinks_fifo_and_modes_refuse(self):
        for variant in ('name', 'symlink', 'hardlink', 'fifo', 'mode', 'directory'):
            with self.subTest(variant=variant), setup() as (directory, batch):
                _, path = save(directory, batch, phase=None)
                if variant == 'name': (directory / 'other').write_bytes(b'x')
                elif variant == 'symlink':
                    path.unlink(); path.symlink_to('/dev/null')
                elif variant == 'hardlink': os.link(path, directory.parent / (directory.name + '.hardlink'))
                elif variant == 'fifo': path.unlink(); os.mkfifo(path, 0o600)
                elif variant == 'mode': path.chmod(0o640)
                elif variant == 'directory': path.unlink(); path.mkdir()
                try:
                    with self.assertRaises((ValueError, OSError)):
                        with history.Inventory(directory, batch): pass
                finally:
                    if variant == 'hardlink': (directory.parent / (directory.name + '.hardlink')).unlink()

    def test_parent_modes_symlinks_and_lock_contention_refuse(self):
        with setup() as (directory, batch):
            directory.chmod(0o750)
            with self.assertRaises(ValueError):
                with history.Inventory(directory, batch): pass
            directory.chmod(0o700)
            alias = directory.parent / (directory.name + '.link')
            alias.symlink_to(directory, target_is_directory=True)
            try:
                with self.assertRaises(OSError):
                    with history.Inventory(alias, batch): pass
            finally: alias.unlink()
            with history.Inventory(directory, batch):
                with self.assertRaises(BlockingIOError):
                    with history.Inventory(directory, batch): pass

    def test_checks_detect_new_removed_and_byte_identical_replaced_history(self):
        for variant in ('add', 'remove', 'replace', 'modify'):
            with self.subTest(variant=variant), setup() as (directory, batch):
                _, path = save(directory, batch, phase=None)
                owner = history.Inventory(directory, batch).__enter__()
                try:
                    if variant == 'add':
                        new = directory / ('progress-' + 'f' * 64 + '.run.scope.json')
                        new.write_bytes(b'{}\n'); new.chmod(0o600)
                    elif variant == 'remove': path.unlink()
                    elif variant == 'replace':
                        raw = path.read_bytes(); path.unlink(); path.write_bytes(raw); path.chmod(0o600)
                    else: path.write_bytes(path.read_bytes().replace(b'5000', b'5001'))
                    with self.assertRaises((ValueError, OSError)): owner.check()
                finally: owner.close()

    def test_membership_change_during_last_file_read_is_detected(self):
        with setup() as (directory, batch):
            save(directory, batch)
            owner = history.Inventory(directory, batch).__enter__()
            original = owner._read
            calls = []
            def read(name):
                out = original(name); calls.append(name)
                if len(calls) == len(owner._files):
                    extra = directory / ('progress-' + 'f' * 64 + '.run.scope.json')
                    extra.write_bytes(b'{}\n'); extra.chmod(0o600)
                return out
            try:
                with patch.object(owner, '_read', side_effect=read):
                    with self.assertRaises(ValueError): owner.check()
            finally: owner.close()

    def test_paging_is_exact_and_stale_after_publication_replacement_or_batch_change(self):
        with setup() as (directory, batch):
            pairs = [save(directory, batch, sequence=i, phase=None) for i in range(5)]
            with history.Inventory(directory, batch) as owner:
                page = owner.page(limit=2)
                token = page['continuation']
                found = list(page['rows'])
                while page['continuation']:
                    page = owner.page(limit=2, continuation=page['continuation'])
                    found.extend(page['rows'])
                self.assertEqual(len(found), 5)
                self.assertEqual([r['clock_record_id'] for r in found], sorted(s.digest for s, _ in pairs))
                for options in ({'limit': 3}, {'selection': 'pending', 'limit': 2}):
                    with self.assertRaises(ValueError): owner.page(continuation=token, **options)
            scope, path = pairs[0]
            outcomes.retain(path, record_for(scope))
            with history.Inventory(directory, batch) as owner:
                with self.assertRaises(ValueError): owner.page(limit=2, continuation=token)
            batch.inventory = 'e' * 64
            with history.Inventory(directory, batch) as owner:
                with self.assertRaises(ValueError): owner.page(limit=2, continuation=token)

    def test_page_options_are_closed_and_unreturned_rows_still_block_new_work(self):
        with setup() as (directory, batch):
            save(directory, batch, phase=None)
            with history.Inventory(directory, batch) as owner:
                terminal = owner.page(selection='terminal')
                self.assertEqual(terminal['rows'], [])
                self.assertFalse(terminal['directory_history_reconciled'])
                for value in (0, 65, True, 1.0):
                    with self.assertRaises(ValueError): owner.page(limit=value)
                for token in ('x', 'zz.' + '0' * 64, '[]'.encode().hex() + '.' + '0' * 64, 'x' * 2049):
                    with self.assertRaises(ValueError): owner.page(continuation=token)
                with self.assertRaises(ValueError): owner.admit_new(scope_for(batch, 1))

    def test_borrowed_lock_stays_owned_after_inventory_closes(self):
        with setup() as (directory, batch):
            path = directory / ('progress-' + 'f' * 64 + '.run')
            with outcomes.new_run(path) as parent:
                with history.Inventory(directory, batch, parent_fd=parent) as owner:
                    self.assertEqual(os.fstat(owner.parent).st_ino, os.fstat(parent).st_ino)
                with self.assertRaises(BlockingIOError):
                    with history.Inventory(directory, batch): pass
            with history.Inventory(directory, batch): pass

    def test_budget_exhaustion_and_lost_custody_never_publish_partial_inventory(self):
        for kind in ('disk', 'work', 'time', 'custody', 'unverified'):
            with self.subTest(kind=kind), setup() as (directory, batch):
                save(directory, batch)
                if kind == 'disk': batch.budget.disk_bytes = 1
                elif kind == 'work': batch.budget.work_steps = 1
                elif kind == 'time': batch.budget.deadline = 0
                elif kind == 'custody': batch.corrupt = True
                else: batch.verified = False
                with self.assertRaises(ValueError):
                    with history.Inventory(directory, batch) as owner: owner.page()

    def test_current_binding_must_survive_admission_and_publication(self):
        with setup() as (directory, batch):
            owner = history.Inventory(directory, batch).__enter__()
            batch.inventory = 'f' * 64
            try:
                with self.assertRaises(ValueError): owner.page()
                with self.assertRaises(ValueError): owner.admit_new(scope_for(batch, 1))
            finally: owner.close()

    def test_maximum_history_is_paged_without_truncation_and_refuses_another_effect(self):
        with setup() as (directory, batch):
            for sequence in range(history.MAX_RECORDS): save(directory, batch, sequence=sequence)
            batch.budget = Budget()
            with history.Inventory(directory, batch) as owner:
                page = owner.page(limit=64)
                total = len(page['rows'])
                while page['continuation']:
                    page = owner.page(limit=64, continuation=page['continuation'])
                    self.assertLessEqual(len(canonical(page)), history.MAX_OUTPUT)
                    total += len(page['rows'])
                self.assertEqual(total, 256)
                with self.assertRaises(ValueError): owner.admit_new(scope_for(batch, 256))
            save(directory, batch, sequence=256)
            with self.assertRaises(ValueError):
                with history.Inventory(directory, batch): pass

    def test_new_effect_may_add_only_its_three_names_without_changing_prior_records(self):
        with setup() as (directory, batch):
            save(directory, batch)
            fresh = scope_for(batch, 1)
            path = directory / ('progress-' + fresh.digest + '.run')
            with outcomes.new_run(path) as parent:
                with history.Inventory(directory, batch, parent_fd=parent) as owner:
                    owner.admit_new(fresh)
                    with ScopeStore(path, parent, batch, fresh.digest, create=fresh):
                        owner.check()
                        with clock.capsule(path, fresh.intent()): owner.check()
                        with outcomes.OutcomeStore(path, parent_fd=parent) as saved:
                            saved.retain(record_for(fresh)); owner.check()
            with history.Inventory(directory, batch) as owner:
                self.assertEqual(owner.page()['counts']['records'], 2)
                self.assertTrue(owner.page()['directory_history_reconciled'])

    def test_close_releases_descriptors_even_after_failed_entry(self):
        with setup() as (directory, batch):
            count = lambda: len(os.listdir('/proc/self/fd'))
            baseline = count()
            for _ in range(30):
                owner = history.Inventory(directory, batch)
                with owner: owner.page()
                with self.assertRaises(ValueError): owner.__enter__()
                owner.close()
            self.assertEqual(count(), baseline)


if __name__ == '__main__':
    unittest.main()
