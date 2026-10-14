#!/usr/bin/env python3
"""Exercise bounded-run ownership, private custody and placement TCP to a test peer.

No real map/inventory acquisition, DFHack, Rust or admission evidence is produced.
"""
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
MODULES = ('test_route_furniture_run', 'test_route_furniture_run_integration', 'test_route_furniture_run_tcp')
PRODUCTION = (
    'route_furniture_run.py', 'route_furniture_batch.py', 'route_furniture_batch_store.py',
    'build_placement_client.py', 'build_placement_rpc.py', 'build_placement_store.py',
    'build_placement_reconciliation.py', 'build_placement_wire.py',
    'furniture_plan.py', 'furniture_allocation.py', 'route_furniture_allocation.py',
    'route_furniture_review.py', 'route_furniture_frontier.py', 'route_furniture_suffix.py',
    'route_furniture_sequence.py',
)
FILES = tuple('scripts/' + name for name in (
    *PRODUCTION, *(name + '.py' for name in MODULES), 'check_route_furniture_run.py'))
MUTATIONS = (
    ('consent_ignored', 'route_furniture_run.py',
     "require(actual == confirmed, 'run confirmation differs from current original prefix or limits')",
     "require(True, 'run confirmation differs from current original prefix or limits')",
     'test_route_furniture_run.RunTests.test_confirmation_binds_count_timeout_review_inventory_and_native_frontier'),
    ('step_cap_ignored', 'route_furniture_run.py',
     'batch.plan.ordered[count:count + limits.max_steps]', 'batch.plan.ordered[count:]',
     'test_route_furniture_run.RunTests.test_explicit_limit_stops_and_fresh_consent_resumes_only_original_suffix'),
    ('budget_refilled', 'route_furniture_run.py',
     'for index, name in enumerate(approved):\n        _limits(batch, limits)',
     'for index, name in enumerate(approved):\n        batch.budget = make_budget(limits, _steps=steps)\n        _limits(batch, limits)',
     'test_route_furniture_run.RunTests.test_complete_bounded_prefix_and_one_budget_for_every_step'),
    ('output_reservation_omitted', 'route_furniture_run.py',
     '        _reserve(steps, current)\n', '        # Removed pre-effect output reservation.\n',
     'test_route_furniture_run.RunTests.test_complete_output_is_reserved_before_any_child'),
    ('final_route_gate_ignored', 'route_furniture_batch.py',
     "require(fresh['status'] == 'ready' and fresh['evidence_digest'] == first['evidence_digest'],",
     'require(True,',
     'test_route_furniture_run_integration.JournalRunTests.test_native_final_route_gate_still_blocks_after_dispatch_sync'),
    ('child_append_sync_omitted', 'build_placement_store.py',
     'write_all(self.fd, new[len(self.raw):])\n            os.fsync(self.fd)',
     'write_all(self.fd, new[len(self.raw):])\n            # Removed child-file sync.',
     'test_route_furniture_run_integration.JournalRunTests.test_complete_dispatch_bytes_are_synced_before_each_writer'),
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
    suite = unittest.defaultTestLoader.loadTestsFromNames(MODULES)
    result = unittest.TextTestRunner(stream=sys.stderr, verbosity=2).run(suite)
    passed = result.wasSuccessful() and not result.skipped
    report = {'schema': 'dfmcp.route-batch-run-evidence/1', 'python': sys.version.split()[0],
              'tests_run': result.testsRun, 'failures': len(result.failures), 'errors': len(result.errors),
              'skipped': len(result.skipped), 'mutations': [], 'sources': before,
              'scope': {'run_owner': 'actual source', 'private_files': 'actual POSIX files and sync calls',
                        'placement_coordinator': 'actual source',
                        'placement_tcp': 'actual client to joined, independently encoded test peers',
                        'map_inventory_acquisition': 'injected; production frontier and suffix still execute',
                        'native_values': 'strict codecs with synthetic game data',
                        'cli': 'actual parser/main/stdio functions, including alternate entry module identity',
                        'complete_32_step_private_run': 'not qualified by this suite',
                        'real_dfhack': False, 'rust_mcp': False, 'full_workspace': False,
                        'production_admitted': False}}
    if args.mutations and passed:
        for name, path, old, new, test in MUTATIONS:
            with tempfile.TemporaryDirectory(prefix='dfmcp-run-mutant-') as directory:
                base = Path(directory)
                (base / 'scripts').mkdir()
                for relative in FILES:
                    shutil.copyfile(ROOT / relative, base / relative)
                source = base / 'scripts' / path
                text = source.read_text()
                if text.count(old) != 1:
                    raise ValueError('mutation target no longer unique: ' + name)
                source.write_text(text.replace(old, new))
                env = dict(os.environ, PYTHONPATH=str(base / 'scripts'), PYTHONDONTWRITEBYTECODE='1')
                run = subprocess.run([sys.executable, '-B', '-m', 'unittest', test, '-v'],
                                     cwd=base, env=env, capture_output=True, text=True,
                                     timeout=30, check=False)
                killed = (run.returncode != 0 and 'FAIL:' in run.stderr
                          and 'AssertionError' in run.stderr and 'ERROR:' not in run.stderr)
                report['mutations'].append({'name': name, 'path': path, 'test': test,
                                             'assertion_rejected': killed, 'returncode': run.returncode})
                print(name + ': ' + ('assertion rejected' if killed else 'NOT REJECTED'), file=sys.stderr)
                if not killed:
                    print(run.stderr, file=sys.stderr)
                passed = passed and killed
    after = inventory()
    report['source_bytes_unchanged'] = before == after
    report['passed'] = passed and before == after
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
