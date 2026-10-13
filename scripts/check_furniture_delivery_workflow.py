#!/usr/bin/env python3
"""Run exact-source delivery workflow/component regressions and optional mutants."""
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
    'furniture_plan.py', 'furniture_allocation.py', 'build_placement_wire.py',
    'route_furniture_allocation.py', 'route_furniture_review.py', 'route_furniture_frontier.py',
    'route_furniture_sequence.py', 'route_furniture_review_runtime.py', 'review_route_furniture.py',
    'test_cross_level_furniture.py', 'test_cross_level_furniture_review.py',
    'test_cross_level_furniture_frontier.py', 'test_furniture_delivery_workflow.py',
    'check_furniture_delivery_workflow.py',
))
SUITES = ('test_cross_level_furniture', 'test_cross_level_furniture_review',
          'test_cross_level_furniture_frontier', 'test_furniture_delivery_workflow')
MUTATIONS = (
    ('sequencing_not_called', 'route_furniture_review_runtime.py',
     '    if sequence_deliveries:\n', '    if False:\n',
     'DeliveryWorkflowTests.test_opt_in_exports_effective_review_and_complete_order_report'),
    ('final_authority_not_checked', 'route_furniture_review_runtime.py',
     '    budget.remaining()\n    authority.guard()\n    return output',
     '    budget.remaining()\n    return output',
     'DeliveryWorkflowTests.test_final_authority_and_output_failure_cannot_publish_cached_review'),
    ('check_allowed_to_reorder', 'review_route_furniture.py',
     "        require(creating or not args.sequence_deliveries, 'check cannot reorder an original review')",
     "        require(True, 'check cannot reorder an original review')",
     'DeliveryWorkflowTests.test_cli_check_rejects_reordering_before_files_or_native_contact'),
    ('prior_targets_not_blocked', 'route_furniture_sequence.py',
     '        blocked.add(targets[name])\n        done.add(name)',
     '        done.add(name)',
     'SequenceOracleTests.test_corridor_reorders_without_reallocation_or_constraint_loss'),
)


def hashes():
    out = {}
    for name in FILES:
        raw = (ROOT / name).read_bytes()
        out[name] = {'sha256': hashlib.sha256(raw).hexdigest(),
                     'git_blob': hashlib.sha1(f'blob {len(raw)}\0'.encode() + raw).hexdigest()}
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mutations', action='store_true')
    args = parser.parse_args()
    before = hashes()
    suite = unittest.defaultTestLoader.loadTestsFromNames(SUITES)
    result = unittest.TextTestRunner(stream=sys.stderr, verbosity=2).run(suite)
    report = {'schema': 'dfmcp.furniture-delivery-workflow-evidence/1',
              'python': sys.version.split()[0], 'tests': result.testsRun,
              'failures': len(result.failures), 'errors': len(result.errors),
              'skipped': len(result.skipped), 'mutations': [], 'sources': before,
              'scope': 'real selector/sequence/review and complete workflow/CLI source with injected acquisition, budget, outer packet; real input files',
              'native_tcp_executed': False, 'private_batch_executed': False,
              'rust_mcp_executed': False, 'live_fortress': False}
    if args.mutations and result.wasSuccessful():
        for name, filename, old, new, test in MUTATIONS:
            source = (ROOT / 'scripts' / filename).read_text()
            if source.count(old) != 1:
                raise ValueError('mutation no longer identifies one source site: ' + name)
            with tempfile.TemporaryDirectory(prefix='dfmcp-delivery-test-') as directory:
                target = Path(directory)
                (target / 'scripts').mkdir()
                for path in FILES:
                    shutil.copyfile(ROOT / path, target / path)
                (target / 'scripts' / filename).write_text(source.replace(old, new))
                environment = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
                environment.update(PYTHONPATH=str(target / 'scripts'), PYTHONDONTWRITEBYTECODE='1')
                run = subprocess.run([sys.executable, '-m', 'unittest',
                    'test_furniture_delivery_workflow.' + test, '-v'],
                    cwd=target, env=environment, capture_output=True, text=True, timeout=20)
                killed = (run.returncode != 0 and 'FAIL:' in run.stderr
                          and 'AssertionError' in run.stderr and 'ERROR:' not in run.stderr)
                report['mutations'].append({'name': name, 'assertion_rejected': killed})
                print(name + ': ' + ('rejected' if killed else 'NOT rejected'), file=sys.stderr)
    report['sources_unchanged'] = before == hashes()
    report['passed'] = (result.wasSuccessful() and not result.skipped and report['sources_unchanged']
        and (not args.mutations or len(report['mutations']) == len(MUTATIONS)
             and all(v['assertion_rejected'] for v in report['mutations'])))
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
