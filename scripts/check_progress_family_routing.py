#!/usr/bin/env python3
"""Run complete progress-owner routing tests with explicitly mocked infrastructure."""
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
SUITES = ('test_progress_family_routing', 'test_progress_script_entrypoint', 'test_progress_final_observe')
FILES = ('route_furniture_progress.py', 'route_furniture_final_progress.py',
         *(name + '.py' for name in SUITES), 'check_progress_family_routing.py')
MUTATIONS = (
    ('recover_by_current_status', 'route_furniture_progress.py',
     '    if type(scope) is FinalScope:', "    if batch.audit()['status'] == 'all_placed':",
     'test_progress_family_routing.FamilyRoutingTests.test_legacy_query_after_final_placement_is_not_reinterpreted'),
    ('rewrite_final_key', 'route_furniture_final_progress.py',
     "'clock_key': scope.clock_key, 'clock_record_id': scope.digest,",
     "'clock_key': 'frp-' + scope.digest, 'clock_record_id': scope.digest,",
     'test_progress_family_routing.FamilyRoutingTests.test_final_offline_query_preserves_family_key_and_final_report'),
    ('omit_final_read_permission_check', 'route_furniture_progress.py',
     "    encoded('observe-final', state)\n    authority.guard('QueryPlacement')\n    batch.check()",
     "    encoded('observe-final', state)\n    batch.check()",
     'test_progress_final_observe.FinalObserveTests.test_read_permission_revoked_during_observation_refuses_publication'),
    ('claim_stable_completion', 'route_furniture_final_progress.py',
     "'placement_retry_permitted': False, 'construction_completion_proven': False}",
     "'placement_retry_permitted': False, 'construction_completion_proven': True}",
     'test_progress_family_routing.FamilyRoutingTests.test_final_offline_query_preserves_family_key_and_final_report'),
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
    report = {'schema': 'dfmcp.progress-family-routing-evidence/1',
              'tests': result.testsRun, 'failures': len(result.failures),
              'errors': len(result.errors), 'skips': len(result.skipped),
              'scope': 'Complete progress/final-progress source and CLI; infrastructure and codecs are explicit doubles',
              'native_tcp': False, 'private_file_custody': False, 'live_game': False,
              'rust_or_full_workspace': False, 'sources': before, 'mutations': []}
    if args.mutations and result.wasSuccessful():
        for name, filename, old, new, test in MUTATIONS:
            source = (ROOT / filename).read_text()
            if source.count(old) != 1:
                raise ValueError('ambiguous mutation: ' + name)
            with tempfile.TemporaryDirectory(prefix='dfmcp-family-test-') as directory:
                target = Path(directory)
                for path in FILES:
                    shutil.copyfile(ROOT / path, target / path)
                (target / filename).write_text(source.replace(old, new))
                env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
                env.update(PYTHONPATH=str(target), PYTHONDONTWRITEBYTECODE='1')
                checked = subprocess.run([sys.executable, '-m', 'unittest', test, '-v'],
                    cwd=target, env=env, capture_output=True, text=True, timeout=20)
                killed = (checked.returncode != 0 and 'FAIL:' in checked.stderr
                          and 'AssertionError' in checked.stderr and 'ERROR:' not in checked.stderr)
                report['mutations'].append({'name': name, 'assertion_rejected': killed})
    report['sources_unchanged'] = hashes() == before
    report['passed'] = (result.wasSuccessful() and not result.skipped and report['sources_unchanged']
        and (not args.mutations or len(report['mutations']) == len(MUTATIONS)
             and all(m['assertion_rejected'] for m in report['mutations'])))
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
