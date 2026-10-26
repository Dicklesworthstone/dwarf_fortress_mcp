#!/usr/bin/env python3
"""Run measured original-completion clock tests and assertion-based mutants."""
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
SUITES = ('test_completion_clock_plan', 'test_completion_clock_scope', 'test_completion_clock_workflow')
FILES = ('completion_clock_plan.py', 'completion_clock_scope.py', 'route_furniture_clock_context.py',
         'route_furniture_clock_inventory.py', 'scoped_clock_inventory.py',
         'verify_route_furniture_completion.py', 'check_completion_clock.py',
         *(name + '.py' for name in SUITES))
MUTANTS = (
    ('exclusive_deadline_lost', 'completion_clock_plan.py',
     'finish >= window.deadline', 'finish > window.deadline',
     'test_completion_clock_plan.CadenceTests.test_exhaustive_small_schedules_and_selected_first_step_are_sound'),
    ('stable_span_ignored', 'completion_clock_plan.py',
     'span_end = window.first_tick + window.stable_span', 'span_end = window.first_tick',
     'test_completion_clock_plan.CadenceTests.test_last_needed_sample_waits_for_span_without_renewing_policy'),
    ('monitor_head_recheck_omitted', 'verify_route_furniture_completion.py',
     'witness(owner, batch, expected.goal_digest) == expected', 'True',
     'test_completion_clock_workflow.CompletionWorkflowTests.test_monitor_change_or_revocation_after_prepare_prevents_commit'),
    ('history_admission_omitted', 'verify_route_furniture_completion.py',
     '            history.admit_new(scope)\n', '            pass\n',
     'test_completion_clock_workflow.CompletionWorkflowTests.test_history_blocks_new_effect_for_verification_too'),
    ('monitor_readonly_lost', 'verify_route_furniture_completion.py',
     'type(owner) is Journal and not owner.writable and not batch.writable', 'type(owner) is Journal',
     'test_completion_clock_workflow.CompletionWorkflowTests.test_shared_original_owner_budget_and_readonly_custody_are_required'),
    ('clock_duration_unbound', 'completion_clock_scope.py',
     "and decision['game_ticks'] == value['game_ticks']", 'and True',
     'test_completion_clock_scope.CompletionScopeTests.test_mismatched_capture_tick_interval_and_foreign_scope_refuse'),
    ('verification_mislabeled', 'scoped_clock_inventory.py',
     "scope.get('phase', 'between_placements')",
     "'final_construction' if type(owner.scope) is FinalScope else 'between_placements'",
     'test_completion_clock_scope.CompletionScopeTests.test_independent_recovery_discovery_preserves_the_verification_phase_and_key'),
)


def inventory():
    out = {}
    for name in FILES:
        raw = (ROOT / name).read_bytes()
        out['scripts/' + name] = {'bytes': len(raw), 'sha256': hashlib.sha256(raw).hexdigest(),
            'git_blob': hashlib.sha1(b'blob ' + str(len(raw)).encode() + b'\0' + raw).hexdigest()}
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mutations', action='store_true')
    args = parser.parse_args()
    before = inventory()
    tests = unittest.TestSuite(unittest.defaultTestLoader.loadTestsFromName(name) for name in SUITES)
    result = unittest.TextTestRunner(verbosity=1).run(tests)
    mutated = []
    if args.mutations:
        for name, file, original, replacement, target in MUTANTS:
            with tempfile.TemporaryDirectory() as temporary:
                destination = Path(temporary)
                for source in FILES:
                    shutil.copy2(ROOT / source, destination / source)
                path = destination / file
                text = path.read_text()
                if text.count(original) != 1:
                    raise RuntimeError('mutation site missing or ambiguous: ' + name)
                path.write_text(text.replace(original, replacement, 1))
                checked = subprocess.run([sys.executable, '-m', 'unittest', target, '-v'],
                    cwd=destination, env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'},
                    capture_output=True, text=True, timeout=15)
                log = checked.stdout + checked.stderr
                rejected = checked.returncode != 0 and 'FAIL:' in log and 'AssertionError' in log and 'ERROR:' not in log
                mutated.append({'name': name, 'assertion_rejected': rejected})
                if not rejected:
                    print(log, file=sys.stderr)
    stable = before == inventory()
    passed = result.wasSuccessful() and not result.skipped and stable and all(x['assertion_rejected'] for x in mutated)
    print(json.dumps({'ok': passed, 'tests_run': result.testsRun, 'failures': len(result.failures),
        'errors': len(result.errors), 'skipped': len(result.skipped), 'unchanged_inputs': stable,
        'mutants': mutated, 'inputs': before,
        'evidence_scope': 'actual scheduler and scope/workflow sources; explicit codec, custody and native I/O ports; no native/live/full-workspace claim'},
        sort_keys=True, indent=2))
    return 0 if passed else 1


if __name__ == '__main__':
    raise SystemExit(main())
