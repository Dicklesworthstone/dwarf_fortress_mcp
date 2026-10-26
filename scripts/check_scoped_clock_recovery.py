#!/usr/bin/env python3
"""Run exact-source query-only clock recovery, private-file/TCP/CLI regressions."""
from __future__ import annotations

import argparse
import ast
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent
SUITES = ('test_route_furniture_clock_inventory', 'test_scoped_clock_recovery',
          'test_scoped_clock_recovery_cli')
FILES = ('bounded_run_client.py', 'bounded_run_outcomes.py', 'furniture_plan.py',
         'furniture_allocation.py', 'route_furniture_allocation.py', 'route_furniture_review.py',
         'route_furniture_clock_context.py', 'route_furniture_clock_inventory.py',
         'test_route_furniture_clock_inventory_workflow.py', 'scoped_clock_recovery.py',
         'recover_scoped_clocks.py', 'check_scoped_clock_recovery.py',
         *(name + '.py' for name in SUITES))
MUTATIONS = (
    ('stale_inventory_accepted', 'scoped_clock_recovery.py',
     "require(self.digest == expected, 'clock inventory changed; inspect the original history again')",
     "require(True, 'clock inventory changed; inspect the original history again')",
     'test_scoped_clock_recovery.RecoveryTests.test_stale_inventory_refuses_before_any_socket_or_new_outcome'),
    ('query_cap_ignored', 'scoped_clock_recovery.py',
     "targets = [dict(row) for row in self._rows if row['query_supported']][:maximum]",
     "targets = [dict(row) for row in self._rows if row['query_supported']]",
     'test_scoped_clock_recovery.RecoveryTests.test_exact_limits_through_sixteen_queries_and_no_quota_reset'),
    ('source_loss_resolved', 'scoped_clock_recovery.py',
     "resolved=record['phase'] in ('stopped', 'refused'), query_supported=False,",
     "resolved=record['phase'] in ('stopped', 'refused', 'source_lost'), query_supported=False,",
     'test_scoped_clock_recovery.RecoveryTests.test_absent_prepared_running_and_source_lost_remain_unresolved'),
    ('queries_continue_after_failure', 'scoped_clock_recovery.py',
     "                        stop, faulted = 'query_failed', True\n                        break",
     "                        stop, faulted = 'query_failed', True\n                        continue",
     'test_scoped_clock_recovery.RecoveryTests.test_failed_second_query_retains_first_and_does_not_retry_or_query_third'),
    ('ambiguous_publication_not_fenced', 'scoped_clock_recovery.py',
     '            self.fenced = True\n', '            self.fenced = False\n',
     'test_scoped_clock_recovery.RecoveryTests.test_outcome_sync_failure_fences_owner_and_preserves_ambiguous_bytes'),
    ('output_reservation_omitted', 'scoped_clock_recovery.py',
     '        render(reservation)\n', '        pass\n',
     'test_scoped_clock_recovery.RecoveryTests.test_complete_result_reserved_before_contact'),
    ('clock_lock_released_before_output', 'recover_scoped_clocks.py',
     '        output_attempted = True\n        result_code = 0 if publish(raw)',
     '        history.close()\n        output_attempted = True\n        result_code = 0 if publish(raw)',
     'test_scoped_clock_recovery_cli.RecoveryCliTests.test_lock_and_original_batch_held_through_single_output_attempt'),
    ('failed_pass_exit_success', 'recover_scoped_clocks.py',
     "result_code = 0 if publish(raw) and not report['faulted'] else 2",
     'result_code = 0 if publish(raw) else 2',
     'test_scoped_clock_recovery_cli.RecoveryCliTests.test_failed_second_query_returns_first_durable_result_with_exit_two'),
)


def hashes():
    result = {}
    for name in FILES:
        raw = (ROOT / name).read_bytes()
        ast.parse(raw, filename=name)
        result['scripts/' + name] = {'bytes': len(raw), 'sha256': hashlib.sha256(raw).hexdigest(),
            'git_blob': hashlib.sha1(f'blob {len(raw)}\0'.encode() + raw).hexdigest()}
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mutations', action='store_true')
    args = parser.parse_args()
    before = hashes()
    result = unittest.TextTestRunner(stream=sys.stderr, verbosity=2).run(
        unittest.defaultTestLoader.loadTestsFromNames(SUITES))
    evidence = {'schema': 'dfmcp.scoped-clock-recovery-evidence/1',
        'python': sys.version.split()[0], 'tests': result.testsRun, 'failures': len(result.failures),
        'errors': len(result.errors), 'skipped': len(result.skipped), 'syntax_checked': True,
        'sources': before, 'mutations': [],
        'scope': 'actual Python clock TCP to joined synthetic peers, strict scope/intent/receipt codecs, private clock files and locks, complete recovery CLI and output; original route-batch custody/budget are explicit ports',
        'live_fortress': False, 'real_private_route_batch': False,
        'rust_mcp_executed': False, 'production_admission_changed': False}
    if args.mutations and result.wasSuccessful():
        for name, filename, old, new, test in MUTATIONS:
            source = (ROOT / filename).read_text()
            if source.count(old) != 1:
                raise ValueError('mutation site must identify exact source: ' + name)
            with tempfile.TemporaryDirectory(prefix='dfmcp-recovery-mutant-') as directory:
                target = Path(directory)
                for filename_to_copy in FILES:
                    shutil.copyfile(ROOT / filename_to_copy, target / filename_to_copy)
                (target / filename).write_text(source.replace(old, new))
                environment = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
                environment.update(PYTHONPATH=str(target), PYTHONDONTWRITEBYTECODE='1')
                try:
                    run = subprocess.run([sys.executable, '-m', 'unittest', test, '-v'],
                        cwd=target, env=environment, capture_output=True, text=True, timeout=20)
                    killed = (run.returncode != 0 and 'FAIL:' in run.stderr
                              and 'AssertionError' in run.stderr and 'ERROR:' not in run.stderr)
                except subprocess.TimeoutExpired:
                    killed = False
                evidence['mutations'].append({'name': name, 'assertion_rejected': killed})
                print(name + ': ' + ('rejected' if killed else 'NOT rejected'), file=sys.stderr)
    evidence['sources_unchanged'] = before == hashes()
    evidence['passed'] = (result.wasSuccessful() and not result.skipped and evidence['sources_unchanged']
        and (not args.mutations or len(evidence['mutations']) == len(MUTATIONS)
             and all(row['assertion_rejected'] for row in evidence['mutations'])))
    print(json.dumps(evidence, indent=2, sort_keys=True))
    return 0 if evidence['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
