"""Actual bounded directory custody and CLI discovery, with no native source."""
from __future__ import annotations

import hashlib
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

import bounded_run_client as clock
import bounded_run_outcomes as storage
import route_furniture_clock_context as context
import recover_route_furniture_clock as recovery
import scoped_clock_inventory as census
from test_scoped_clock_recovery import BATCH, Fixture, private_write, scope_for


class ManyFixture(Fixture):
    def __init__(self, root, index, *, batch_id=BATCH, final=False, address=('127.0.0.1', 5000)):
        self.root = Path(root)
        self.root.chmod(0o700)
        value = scope_for(address, final).value()
        value.update(batch_id=batch_id, inventory_digest=f'{index:064x}')
        self.scope = (context.FinalScope if final else context.Scope).create(value)
        self.intent = self.scope.intent()
        self.path = recovery.record_path(str(self.root), self.scope.digest)
        self.sidecar = Path(str(self.path) + context.SCOPE_SUFFIX)
        self.outcome = Path(str(self.path) + storage.SUFFIX)
        private_write(self.path, clock.canonical({'intent': self.intent,
            'sha256': hashlib.sha256(clock.canonical(self.intent)).hexdigest()}) + b'\n')
        private_write(self.sidecar, self.scope.raw + b'\n')


def snapshot(directory):
    return {p.name: (p.read_bytes(), storage._fingerprint(p.stat()))
            for p in Path(directory).iterdir() if p.is_file()}


class InventoryTests(unittest.TestCase):
    def test_complete_mixed_inventory_without_batch_or_authority(self):
        with tempfile.TemporaryDirectory() as directory:
            fixtures = [ManyFixture(directory, n, batch_id=BATCH if n < 2 else 'f' * 64,
                                    final=n % 2 == 0) for n in range(4)]
            for fixture, phase in zip(fixtures[1:], ('stopped', 'refused', 'source_lost')):
                fixture.saved(phase)
            before = snapshot(directory)
            with patch.object(recovery.Authority, 'load', side_effect=AssertionError('no authority')):
                with census.Inventory(directory, census.Budget()) as owner:
                    out = owner.page()
                    self.assertEqual(out['directory_records'], 4)
                    self.assertEqual(out['counts'], {'total': 4, 'query_required': 1,
                        'operator_attention': 1, 'unresolved': 2, 'terminal_resolved': 2})
                    self.assertEqual([r['confirmation'] for r in out['rows']], sorted(f.scope.digest for f in fixtures))
                    self.assertEqual({r['phase'] for r in out['rows']}, {'final_construction', 'between_placements'})
                    self.assertTrue(out['complete_clock_inventory_verified'])
                    self.assertFalse(out['original_batch_custody_verified'])
                    self.assertFalse(out['native_contacted'])
                    self.assertTrue(out['current_pause_unproved'])
                    self.assertFalse(out['construction_completion_proven'])
                    for selection, count in (('pending', 1), ('terminal', 3), ('attention', 1)):
                        self.assertEqual(owner.page(selection=selection)['selected_total'], count)
                    filtered = owner.page(batch_id=BATCH)
                    self.assertEqual(filtered['counts']['total'], 2)
                    self.assertEqual(filtered['directory_records'], 4)
            self.assertEqual(before, snapshot(directory))

    def test_empty_and_unmatched_batch_are_only_directory_absence(self):
        with tempfile.TemporaryDirectory() as directory:
            with census.Inventory(directory, census.Budget()) as owner:
                out = owner.page()
                self.assertEqual(out['rows'], [])
                self.assertEqual(out['counts']['unresolved'], 0)
                self.assertEqual(out['inventory_scope'], 'selected_private_directory_only')
                self.assertIsNone(out['continuation'])
            ManyFixture(directory, 0)
            with census.Inventory(directory, census.Budget()) as owner:
                out = owner.page(batch_id='f' * 64)
                self.assertEqual(out['rows'], [])
                self.assertEqual(out['directory_records'], 1)

    def test_pages_cover_each_record_once_and_bind_query_parameters(self):
        with tempfile.TemporaryDirectory() as directory:
            fixtures = [ManyFixture(directory, i) for i in range(9)]
            with census.Inventory(directory, census.Budget()) as owner:
                first = owner.page(limit=4)
                second = owner.page(limit=4, continuation=first['continuation'])
                third = owner.page(limit=4, continuation=second['continuation'])
                self.assertEqual([r['confirmation'] for p in (first, second, third) for r in p['rows']],
                                 sorted(f.scope.digest for f in fixtures))
                self.assertEqual([p['offset'] for p in (first, second, third)], [0, 4, 8])
                self.assertIsNone(third['continuation'])
                for kwargs in ({'limit': 3}, {'limit': 4, 'selection': 'pending'},
                               {'limit': 4, 'batch_id': BATCH}):
                    with self.assertRaises(ValueError):
                        owner.page(continuation=first['continuation'], **kwargs)
                root = first['continuation'].split(':')[0]
                for token in (root + ':0', root + ':04', root + ':5', root + ':12', 'x' * 100,
                              '0' * 64 + ':4', root + ':4\n'):
                    with self.subTest(token=token), self.assertRaises(ValueError):
                        owner.page(limit=4, continuation=token)

    def test_continuation_refuses_new_outcome_and_directory_copy(self):
        with tempfile.TemporaryDirectory() as directory, tempfile.TemporaryDirectory() as copy:
            fixtures = [ManyFixture(directory, i) for i in range(2)]
            with census.Inventory(directory, census.Budget()) as owner:
                first = owner.page(limit=1)
            for path in Path(directory).iterdir():
                private_write(Path(copy) / path.name, path.read_bytes())
            with census.Inventory(copy, census.Budget()) as owner:
                with self.assertRaises(ValueError):
                    owner.page(limit=1, continuation=first['continuation'])
            fixtures[0].saved()
            with census.Inventory(directory, census.Budget()) as owner:
                with self.assertRaises(ValueError):
                    owner.page(limit=1, continuation=first['continuation'])

    def test_same_bytes_replacement_and_membership_changes_fence(self):
        for change in ('replace', 'remove', 'add', 'outcome'):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                fixture = ManyFixture(directory, 0)
                with census.Inventory(directory, census.Budget()) as owner:
                    if change == 'replace':
                        original = fixture.path.read_bytes()
                        alternate = fixture.path.with_suffix('.new')
                        private_write(alternate, original)
                        os.replace(alternate, fixture.path)
                    elif change == 'remove':
                        fixture.path.unlink()
                        fixture.sidecar.unlink()
                    elif change == 'add':
                        ManyFixture(directory, 1)
                    else:
                        # Publish through the already borrowed directory lock.
                        with recovery.Recovery(directory, BATCH, fixture.scope.digest, owner.budget,
                                               parent_fd=owner.parent) as selected:
                            from test_scoped_clock_recovery import record
                            selected.store.retain({'manifest': fixture.scope.value()['clock_manifest'],
                                'record': clock.decode_record(record(fixture.intent, 'stopped'))})
                    with self.assertRaises(ValueError):
                        owner.check()
                    self.assertTrue(owner.fenced)
                    with self.assertRaises(ValueError):
                        owner.page()

    def test_membership_change_mid_scan_is_not_a_complete_snapshot(self):
        with tempfile.TemporaryDirectory() as directory:
            ManyFixture(directory, 0)
            original = census._entry
            def add(owner):
                out = original(owner)
                ManyFixture(directory, 1)
                return out
            with patch.object(census, '_entry', side_effect=add), self.assertRaises(ValueError):
                with census.Inventory(directory, census.Budget()):
                    self.fail('changed census was accepted')

    def test_orphan_foreign_and_corrupt_entries_never_return_partial_inventory(self):
        for fault in ('scope_only', 'intent_only', 'outcome_only', 'foreign', 'bad_scope', 'bad_intent',
                      'bad_outcome', 'wrong_name'):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as directory:
                ManyFixture(directory, 0)
                fixture = ManyFixture(directory, 1)
                if fault == 'scope_only':
                    fixture.path.unlink()
                elif fault == 'intent_only':
                    fixture.sidecar.unlink()
                elif fault == 'outcome_only':
                    fixture.saved()
                    fixture.path.unlink()
                    fixture.sidecar.unlink()
                elif fault == 'foreign':
                    private_write(Path(directory) / 'unscoped.run', b'x')
                elif fault.startswith('bad_'):
                    target = {'bad_scope': fixture.sidecar, 'bad_intent': fixture.path,
                              'bad_outcome': fixture.outcome}[fault]
                    private_write(target, b'{"broken":true}\n')
                else:
                    alternate = recovery.record_path(directory, 'f' * 64)
                    fixture.path.rename(alternate)
                    fixture.sidecar.rename(Path(str(alternate) + context.SCOPE_SUFFIX))
                before = snapshot(directory)
                with self.assertRaises((OSError, ValueError)):
                    with census.Inventory(directory, census.Budget()) as owner:
                        owner.page()
                self.assertEqual(before, snapshot(directory))

    def test_private_regular_single_link_no_follow_files(self):
        for fault in ('symlink', 'hardlink', 'fifo', 'directory', 'mode', 'oversize'):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as directory:
                fixture = ManyFixture(directory, 0)
                if fault == 'mode':
                    fixture.path.chmod(0o644)
                elif fault == 'oversize':
                    fixture.path.write_bytes(b'x' * (storage.MAX_FILE + 1))
                else:
                    data = fixture.path.read_bytes()
                    fixture.path.unlink()
                    if fault in ('symlink', 'hardlink'):
                        elsewhere = Path(directory).parent / (Path(directory).name + '-target')
                        private_write(elsewhere, data)
                        self.addCleanup(elsewhere.unlink)
                        if fault == 'symlink':
                            fixture.path.symlink_to(elsewhere)
                        else:
                            os.link(elsewhere, fixture.path)
                    elif fault == 'fifo':
                        os.mkfifo(fixture.path, 0o600)
                    else:
                        fixture.path.mkdir(mode=0o700)
                with self.assertRaises((OSError, ValueError)):
                    with census.Inventory(directory, census.Budget()):
                        self.fail('unsafe custody accepted')

    def test_directory_custody_is_exclusive_and_borrowing_retains_lock(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = ManyFixture(directory, 0)
            with census.Inventory(directory, census.Budget()) as owner:
                for _ in range(2):
                    with recovery.Recovery(directory, BATCH, fixture.scope.digest, owner.budget,
                                           parent_fd=owner.parent) as selected:
                        self.assertEqual(selected.inspect()['confirmation'], fixture.scope.digest)
                    with self.assertRaises(BlockingIOError):
                        with census.Inventory(directory, census.Budget()):
                            self.fail('directory lock released by borrowed reader')
            with census.Inventory(directory, census.Budget()) as owner:
                self.assertEqual(owner.page()['directory_records'], 1)

    def test_foreign_borrowed_directory_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory, tempfile.TemporaryDirectory() as other:
            fixture = ManyFixture(directory, 0)
            ManyFixture(other, 0)
            with census.Inventory(other, census.Budget()) as owner:
                with self.assertRaises(ValueError):
                    with recovery.Recovery(directory, BATCH, fixture.scope.digest, owner.budget,
                                           parent_fd=owner.parent):
                        self.fail('foreign borrowed parent accepted')

    def test_all_256_records_fit_without_descriptor_growth_or_oversized_pages(self):
        with tempfile.TemporaryDirectory() as directory:
            for i in range(census.MAX_RECORDS):
                ManyFixture(directory, i, final=i % 2 == 0).saved()
            real_open, real_close = os.open, os.close
            opened, peak = set(), [0]
            def counting_open(*args, **kwargs):
                fd = real_open(*args, **kwargs)
                opened.add(fd)
                peak[0] = max(peak[0], len(opened))
                return fd
            def counting_close(fd):
                opened.discard(fd)
                return real_close(fd)
            budget = census.Budget(60000)
            with patch.object(os, 'open', side_effect=counting_open), patch.object(os, 'close', side_effect=counting_close):
                with census.Inventory(directory, budget) as owner:
                    out = owner.page(limit=census.MAX_PAGE)
                    raw = census.encoded('inventory', out)
                    self.assertEqual(out['counts']['terminal_resolved'], census.MAX_RECORDS)
                    self.assertEqual(len(out['rows']), census.MAX_PAGE)
                    self.assertLessEqual(len(raw), census.MAX_OUTPUT)
                    owner.check()  # Same prepublication check as the actual CLI.
            self.assertFalse(opened)
            self.assertLessEqual(peak[0], 10)
            self.assertGreater(budget.disk_bytes, 0)
            self.assertEqual(budget.calls, 32)

    def test_257_records_and_entry_overflow_refuse(self):
        with tempfile.TemporaryDirectory() as directory:
            for i in range(census.MAX_RECORDS + 1):
                ManyFixture(directory, i)
            with self.assertRaises(ValueError):
                with census.Inventory(directory, census.Budget(60000)):
                    self.fail('oversized catalogue accepted')
        with tempfile.TemporaryDirectory() as directory:
            for i in range(3 * census.MAX_RECORDS + 1):
                private_write(Path(directory) / ('progress-' + f'{i:064x}' + '.run'), b'x')
            with self.assertRaises(ValueError):
                with census.Inventory(directory, census.Budget()):
                    self.fail('entry overflow accepted')

    def test_budget_and_path_refusals_leave_files_unchanged(self):
        with tempfile.TemporaryDirectory() as directory:
            ManyFixture(directory, 0)
            before = snapshot(directory)
            for kind in ('disk_bytes', 'work_steps'):
                budget = census.Budget()
                setattr(budget, kind, 0)
                with self.assertRaises(ValueError):
                    with census.Inventory(directory, budget):
                        self.fail('exhausted allowance accepted')
            budget = census.Budget()
            with patch.object(recovery.time, 'monotonic', return_value=budget.deadline + 1):
                with self.assertRaises(ValueError):
                    with census.Inventory(directory, budget):
                        self.fail('expired deadline accepted')
            for path in ('relative', directory + '/', directory + '/../' + Path(directory).name):
                with self.assertRaises(ValueError):
                    census.Inventory(path, census.Budget())
            self.assertEqual(before, snapshot(directory))

    def test_returned_rows_cannot_mutate_pinned_inventory(self):
        with tempfile.TemporaryDirectory() as directory:
            ManyFixture(directory, 0)
            with census.Inventory(directory, census.Budget()) as owner:
                first = owner.page()
                first['rows'][0]['query_required'] = False
                self.assertTrue(owner.page()['rows'][0]['query_required'])

    def test_actual_cli_without_saved_identifiers_or_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            ManyFixture(directory, 0)
            environment = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
            run = subprocess.run([sys.executable, '-B', str(Path(recovery.__file__)), 'inventory',
                '--clock-directory', directory, '--selection', 'pending'],
                env=environment, capture_output=True, timeout=10)
            self.assertEqual(run.returncode, 0, run.stderr)
            self.assertEqual(run.stderr, b'')
            packet = __import__('json').loads(run.stdout)
            self.assertTrue(packet['ok'])
            self.assertEqual(packet['result']['counts']['query_required'], 1)
            self.assertEqual(run.stdout.count(b'\n'), 1)
            self.assertFalse(packet['clock_start_dispatched'])

    def test_cli_rejects_wrong_options_and_attempts_only_one_output_under_lock(self):
        with tempfile.TemporaryDirectory() as directory:
            ManyFixture(directory, 0)
            out = io.BytesIO()
            with patch.object(recovery.sys, 'stdout', types.SimpleNamespace(buffer=out)):
                self.assertEqual(recovery.main(['inventory', '--clock-directory', directory, '--wait-ms', '1']), 2)
            self.assertEqual(out.getvalue().count(b'\n'), 1)
            writes = []
            class Sink:
                def write(inner, raw):
                    writes.append(raw)
                    with self.assertRaises(BlockingIOError):
                        with census.Inventory(directory, census.Budget()):
                            self.fail('output published without directory custody')
                    return len(raw) - 1
                def flush(inner):
                    self.fail('short output should not flush or retry')
            with patch.object(recovery.sys, 'stdout', types.SimpleNamespace(buffer=Sink())):
                self.assertEqual(recovery.main(['inventory', '--clock-directory', directory]), 2)
            self.assertEqual(len(writes), 1)


if __name__ == '__main__':
    unittest.main()
