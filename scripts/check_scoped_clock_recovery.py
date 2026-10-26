#!/usr/bin/env python3
"""Execute exact-source scoped recovery tests, with optional assertion mutants."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
FILES = tuple('scripts/' + name for name in (
    'bounded_run_client.py', 'bounded_run_outcomes.py', 'furniture_plan.py',
    'furniture_allocation.py', 'route_furniture_allocation.py', 'route_furniture_review.py',
    'route_furniture_clock_context.py', 'recover_route_furniture_clock.py',
    'test_scoped_clock_recovery.py', 'scoped_clock_cancel.py',
    'test_scoped_clock_cancel.py', 'check_scoped_clock_recovery.py',
    'scoped_clock_inventory.py', 'test_scoped_clock_inventory.py'))
PREFIX = 'test_scoped_clock_recovery.'
MUTATIONS = (
    ('scope_identity_omitted', 'recover_route_furniture_clock.py',
     "self.scope.digest == self.confirmation and value['batch_id'] == self.batch_id,",
     'True,', 'RecoveryFileTests.test_scope_batch_confirmation_and_intent_are_independently_checked'),
    ('intent_binding_omitted', 'recover_route_furniture_clock.py',
     'clock.require(self.store.intent == self.scope.intent(),',
     'clock.require(True,', 'RecoveryFileTests.test_scope_batch_confirmation_and_intent_are_independently_checked'),
    ('scope_custody_omitted', 'recover_route_furniture_clock.py',
     "storage._fingerprint(os.fstat(self.fd)) == self.stamp\n                          and storage._read(self.fd, self.store.parent, self.name) == self.data,",
     'True,', 'RecoveryTCPTests.test_scope_replacement_during_reply_fences_before_retention'),
    ('old_generation_accepted', 'recover_route_furniture_clock.py',
     "clock.require(client.manifest['generation'] >= original,",
     'clock.require(True,', 'RecoveryTCPTests.test_source_loss_is_retained_but_not_resolved_and_old_generation_is_refused'),
    ('outcome_sync_omitted', 'bounded_run_outcomes.py',
     '            os.fsync(fd)\n            os.fsync(self.parent)\n',
     '            # Deliberately omitted outcome synchronization.\n',
     'RecoveryTCPTests.test_failed_sync_and_torn_publication_do_not_acknowledge_or_repair'),
    ('authority_recheck_omitted', 'recover_route_furniture_clock.py',
     "clock.require(Authority.load() == self, 'clock recovery authority changed')",
     "clock.require(True, 'clock recovery authority changed')",
     'RecoveryFileTests.test_clock_authority_is_only_loaded_for_online_query'),
)

MUTATIONS += (
    ('cancel_attempt_reused', 'scoped_clock_cancel.py',
     'not self.cancel_attempted and fields == clock.intent_fields(self.intent, True)',
     'fields == clock.intent_fields(self.intent, True)',
     'test_scoped_clock_cancel.CancellationTests.test_cancellation_connection_has_one_attempt_even_after_success'),
    ('cancel_identity_omitted', 'scoped_clock_cancel.py',
     'not self.cancel_attempted and fields == clock.intent_fields(self.intent, True)',
     'not self.cancel_attempted',
     'test_scoped_clock_cancel.CancellationTests.test_key_plan_and_prepare_token_must_be_the_original_exact_tuple'),
    ('cancel_generation_widened', 'scoped_clock_cancel.py',
     "result['manifest']['generation'] == self.original['generation']",
     "result['manifest']['generation'] >= self.original['generation']",
     'test_scoped_clock_cancel.CancellationTests.test_changed_or_regressed_native_generation_refuses_before_cancel'),
    ('cancel_prepared_accepted', 'scoped_clock_cancel.py',
     "result.get('record', {}).get('phase') in\n                              ('stopping', 'stopped', 'refused', 'source_lost')",
     'True',
     'test_scoped_clock_cancel.CancellationTests.test_contradictory_prepared_or_running_reply_does_not_acknowledge_stop'),
)


MUTATIONS += (
    ('directory_identity_history_omitted', 'scoped_clock_inventory.py',
     'name, stamp, hashlib.sha256(data).hexdigest()',
     'name, None, hashlib.sha256(data).hexdigest()',
     'test_scoped_clock_inventory.InventoryTests.test_same_bytes_replacement_and_membership_changes_fence'),
    ('inventory_revalidation_omitted', 'scoped_clock_inventory.py',
     'clock.require(self._scan() == self.entries,', 'clock.require(True,',
     'test_scoped_clock_inventory.InventoryTests.test_same_bytes_replacement_and_membership_changes_fence'),
    ('page_query_binding_omitted', 'scoped_clock_inventory.py',
     'match is not None and match[1] == query,', 'match is not None,',
     'test_scoped_clock_inventory.InventoryTests.test_pages_cover_each_record_once_and_bind_query_parameters'),
)


def inventory():
    out = {}
    for name in FILES:
        raw = (ROOT / name).read_bytes()
        out[name] = {'bytes': len(raw), 'sha256': hashlib.sha256(raw).hexdigest(),
                     'git_blob': hashlib.sha1(f'blob {len(raw)}\0'.encode() + raw).hexdigest()}
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mutations', action='store_true')
    args = parser.parse_args()
    before = inventory()
    suite = unittest.defaultTestLoader.loadTestsFromNames(
        ('test_scoped_clock_recovery', 'test_scoped_clock_cancel', 'test_scoped_clock_inventory'))
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    passed = result.wasSuccessful() and not result.skipped
    report = {'schema': 'dfmcp.scoped-clock-recovery-evidence/1',
        'tests_run': result.testsRun, 'failures': len(result.failures), 'errors': len(result.errors),
        'skipped': len(result.skipped), 'python': sys.version.split()[0], 'sources': before,
        'mutations': [], 'scope': {'original_clock_files': 'actual private POSIX files',
            'native_io': 'actual TCP client to joined independently encoded peers',
            'scope_and_intent_codecs': 'unchanged production sources',
            'cli': 'actual parser/output and subprocess-to-peer checks',
            'batch_opened': False, 'furniture_mutation': False, 'real_dfhack': False,
            'rust_mcp': False, 'full_workspace': False, 'production_admission': False}}
    if args.mutations and passed:
        for name, path, old, new, test in MUTATIONS:
            with tempfile.TemporaryDirectory(prefix='scoped-clock-mutant-') as directory:
                base = Path(directory)
                (base / 'scripts').mkdir()
                for relative in FILES:
                    shutil.copyfile(ROOT / relative, base / relative)
                source = base / 'scripts' / path
                text = source.read_text()
                if text.count(old) != 1:
                    raise ValueError('mutation site not unique: ' + name)
                source.write_text(text.replace(old, new))
                env = dict(os.environ, PYTHONPATH=str(base / 'scripts'), PYTHONDONTWRITEBYTECODE='1')
                run = subprocess.run([sys.executable, '-B', '-m', 'unittest',
                     test if test.startswith('test_') else PREFIX + test, '-v'],
                    cwd=base, env=env, capture_output=True, text=True, timeout=15, check=False)
                killed = (run.returncode != 0 and 'FAIL:' in run.stderr
                          and 'AssertionError' in run.stderr and 'ERROR:' not in run.stderr)
                report['mutations'].append({'name': name, 'assertion_rejected': killed,
                                           'returncode': run.returncode})
                print(name + ': ' + ('assertion rejected' if killed else 'NOT REJECTED'), file=sys.stderr)
                if not killed:
                    print(run.stderr, file=sys.stderr)
                passed = passed and killed
    report['source_bytes_unchanged'] = before == inventory()
    report['passed'] = passed and report['source_bytes_unchanged']
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
