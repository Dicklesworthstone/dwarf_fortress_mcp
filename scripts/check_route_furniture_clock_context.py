#!/usr/bin/env python3
"""Execute scoped clock progress and original runner tests with exact input hashes."""
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

ROOT = Path(__file__).resolve().parent
SUITES = ('test_route_furniture_run', 'test_route_furniture_run_readiness',
          'test_route_furniture_clock_context', 'test_route_furniture_progress_context',
          'test_route_furniture_progress')
FILES = ('furniture_plan.py', 'furniture_allocation.py', 'route_furniture_allocation.py',
         'route_furniture_review.py', 'route_furniture_run.py', 'bounded_run_client.py',
         'bounded_run_outcomes.py', 'route_furniture_clock_context.py', 'route_furniture_progress.py',
         *(name + '.py' for name in SUITES), 'check_route_furniture_clock_context.py')
MUTATIONS = (
    ('batch_association_ignored', 'route_furniture_clock_context.py',
     "require(value['batch_id'] == batch.id and value['review_digest'] == batch.review.digest\n                and value['plan_digest'] == batch.plan.digest,",
     "require(True,",
     'test_route_furniture_clock_context.ClockContextTests.test_foreign_batch_or_missing_scope_cannot_adopt_clock_recovery'),
    ('authority_ignored_at_native_boundary', 'route_furniture_clock_context.py',
     '        self.authority.clock_guard()\n', '        pass\n',
     'test_route_furniture_clock_context.ClockContextTests.test_authority_revocation_after_prepare_prevents_commit_and_preserves_intent'),
    ('native_call_budget_not_charged', 'route_furniture_clock_context.py',
     "        self.budget.reserve('calls', 1)\n", '',
     'test_route_furniture_progress_context.ProgressContextTests.test_actual_request_allowance_includes_queries_not_only_constant_reservation'),
    ('scope_sync_omitted', 'route_furniture_clock_context.py',
     '                os.fsync(self.fd)\n', '',
     'test_route_furniture_progress_context.ProgressContextTests.test_scope_and_intent_are_synced_before_prepare_and_lock_never_released'),
    ('intent_identity_replacement_accepted', 'route_furniture_clock_context.py',
     'require(self.intent_stamp is None or stamp == self.intent_stamp,', 'require(True,',
     'test_route_furniture_clock_context.ClockContextTests.test_every_effect_rechecks_original_scope_and_intent_bytes'),
    ('post_stop_clock_floor_ignored', 'route_furniture_progress.py',
     "require(current.tick >= tick, 'post-clock route observation predates the stopped clock effect')",
     "require(True, 'post-clock route observation predates the stopped clock effect')",
     'test_route_furniture_progress_context.ProgressContextTests.test_post_stop_route_cannot_regress_behind_the_clock_receipt'),
)


def hashes():
    result = {}
    for name in FILES:
        raw = (ROOT / name).read_bytes()
        compile(raw, str(ROOT / name), 'exec')
        result['scripts/' + name] = {'bytes': len(raw), 'sha256': hashlib.sha256(raw).hexdigest(),
            'git_blob': hashlib.sha1(f'blob {len(raw)}\0'.encode() + raw).hexdigest()}
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--mutations', action='store_true')
    mode.add_argument('--mutations-only', action='store_true')
    args = parser.parse_args()
    before = hashes()
    result = None if args.mutations_only else unittest.TextTestRunner(stream=sys.stderr, verbosity=2).run(
        unittest.defaultTestLoader.loadTestsFromNames(SUITES))
    tests_passed = result is None or result.wasSuccessful() and not result.skipped
    report = {'schema': 'dfmcp.scoped-clock-progress-test-report/1',
        'python': sys.version.split()[0], 'tests_executed': result is not None,
        'tests': 0 if result is None else result.testsRun,
        'errors': 0 if result is None else len(result.errors),
        'failures': 0 if result is None else len(result.failures),
        'skipped': 0 if result is None else len(result.skipped), 'sources': before,
        'mutations': [], 'clock_native_tcp_executed': True, 'clock_private_files_executed': True,
        'scope': 'actual clock client/codecs/start/recover, scope/intent/outcome files and progress CLI; route observation, batch custody and outer packet are explicit ports',
        'complete_route_acquisition_executed': False, 'batch_private_store_executed': False,
        'cli_functions_executed': True, 'cli_subprocess_to_game_executed': False,
        'rust_mcp_executed': False, 'live_dfhack_executed': False, 'production_admission_changed': False}
    if (args.mutations or args.mutations_only) and tests_passed:
        for name, file, old, new, test in MUTATIONS:
            source = (ROOT / file).read_text()
            if source.count(old) != 1:
                raise ValueError('mutation source site changed: ' + name)
            with tempfile.TemporaryDirectory(prefix='dfmcp-clock-mutation-') as folder:
                for input_name in FILES:
                    shutil.copyfile(ROOT / input_name, Path(folder) / input_name)
                (Path(folder) / file).write_text(source.replace(old, new))
                environment = {key: value for key, value in os.environ.items() if not key.startswith('DFMCP_')}
                environment.update(PYTHONDONTWRITEBYTECODE='1', PYTHONPATH=folder)
                run = subprocess.run([sys.executable, '-m', 'unittest', test, '-v'],
                    cwd=folder, env=environment, capture_output=True, text=True, timeout=30)
                rejected = run.returncode != 0 and 'FAIL:' in run.stderr and 'AssertionError' in run.stderr and 'ERROR:' not in run.stderr
                report['mutations'].append({'name': name, 'assertion_rejected': rejected})
                print(name + ': ' + ('assertion rejected' if rejected else 'NOT rejected'), file=sys.stderr)
                if not rejected:
                    print(run.stderr, file=sys.stderr)
    report['sources_unchanged'] = before == hashes()
    report['passed'] = (tests_passed and report['sources_unchanged']
        and (not (args.mutations or args.mutations_only) or len(report['mutations']) == len(MUTATIONS)
             and all(row['assertion_rejected'] for row in report['mutations'])))
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
