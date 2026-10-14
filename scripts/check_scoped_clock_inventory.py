#!/usr/bin/env python3
"""Execute exact-source scoped clock-history regressions and optional mutants."""
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
SUITES = ('test_route_furniture_clock_inventory', 'test_route_furniture_clock_inventory_workflow')
FILES = ('bounded_run_client.py', 'bounded_run_outcomes.py', 'furniture_plan.py',
         'furniture_allocation.py', 'route_furniture_allocation.py', 'route_furniture_review.py',
         'route_furniture_clock_context.py', 'route_furniture_progress.py',
         'route_furniture_clock_inventory.py', 'check_scoped_clock_inventory.py',
         *(name + '.py' for name in SUITES))
MUTATIONS = (
    ('source_loss_resolved', 'route_furniture_clock_inventory.py',
     "phase in ('stopped', 'refused')", "phase in ('stopped', 'refused', 'source_lost')",
     'test_route_furniture_clock_inventory.InventoryTests.test_all_unresolved_variants_prevent_replacement_effect'),
    ('original_binding_omitted', 'route_furniture_clock_inventory.py',
     '                        scope.bind(self.batch)\n', '                        pass\n',
     'test_route_furniture_clock_inventory.InventoryTests.test_mismatched_original_review_plan_source_and_slot_are_refused'),
    ('native_scope_binding_omitted', 'route_furniture_clock_inventory.py',
     "require(store.intent == scope.intent(), 'native intent differs from original scope')",
     "require(True, 'native intent differs from original scope')",
     'test_route_furniture_clock_inventory.InventoryTests.test_valid_native_intent_with_same_key_still_must_match_scope_bytes'),
    ('unresolved_history_not_fenced', 'route_furniture_clock_inventory.py',
     "self._groups < MAX_RECORDS and all(r['resolved'] for r in self._rows)",
     'self._groups < MAX_RECORDS',
     'test_route_furniture_clock_inventory_workflow.WorkflowTests.test_unknown_prior_effect_blocks_fresh_confirmation_before_new_scope_or_native_start'),
    ('file_identity_ignored', 'route_furniture_clock_inventory.py',
     "require(current == expected, 'original clock history changed')",
     "require(current.sha256 == expected.sha256, 'original clock history changed')",
     'test_route_furniture_clock_inventory.InventoryTests.test_checks_detect_new_removed_and_byte_identical_replaced_history'),
    ('final_membership_check_omitted', 'route_furniture_clock_inventory.py',
     "require(frozenset(self._names()) == names\n                and self._binding() == self._bound, 'clock history changed before publication')",
     "require(True, 'clock history changed before publication')",
     'test_route_furniture_clock_inventory.InventoryTests.test_membership_change_during_last_file_read_is_detected'),
    ('effect_history_check_omitted', 'route_furniture_progress.py',
     '                def effect_guard(intent):\n                    history.check()\n                    binding.verify_intent(intent)\n                    history.check()\n',
     '                def effect_guard(intent):\n                    binding.verify_intent(intent)\n',
     'test_route_furniture_clock_inventory_workflow.WorkflowTests.test_prior_file_replacement_after_prepare_ack_prevents_commit'),
    ('clock_custody_released_before_output', 'route_furniture_progress.py',
     '        if history is not None:\n            history.check()\n',
     '        if history is not None:\n            history.close()\n',
     'test_route_furniture_clock_inventory_workflow.WorkflowTests.test_inventory_cli_needs_no_confirmation_or_credentials_and_holds_both_owners'),
)


def hashes():
    result = {}
    for name in FILES:
        raw = (ROOT / 'scripts' / name).read_bytes()
        result['scripts/' + name] = {'sha256': hashlib.sha256(raw).hexdigest(),
                                    'git_blob': hashlib.sha1(f'blob {len(raw)}\0'.encode() + raw).hexdigest()}
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mutations', action='store_true')
    args = parser.parse_args()
    before = hashes()
    result = unittest.TextTestRunner(stream=sys.stderr, verbosity=2).run(
        unittest.defaultTestLoader.loadTestsFromNames(SUITES))
    report = {'schema': 'dfmcp.scoped-clock-inventory-evidence/1',
              'python': sys.version.split()[0], 'tests': result.testsRun,
              'failures': len(result.failures), 'errors': len(result.errors), 'skipped': len(result.skipped),
              'sources': before, 'mutations': [],
              'scope': 'actual scope/source/intent/receipt codecs, native clock TCP to joined test peers, private clock files and locks, complete inventory and progress/CLI source',
              'test_ports': ['original route acquisition', 'original batch custody/budget', 'outer route packet'],
              'live_dfhack': False, 'rust_mcp': False, 'full_workspace_qualified': False}
    if args.mutations and result.wasSuccessful():
        for name, filename, old, new, test in MUTATIONS:
            source = (ROOT / 'scripts' / filename).read_text()
            if source.count(old) != 1:
                raise ValueError('mutation no longer identifies one source site: ' + name)
            with tempfile.TemporaryDirectory(prefix='dfmcp-clock-history-mutant-') as directory:
                target = Path(directory) / 'scripts'; target.mkdir()
                for file in FILES: shutil.copyfile(ROOT / 'scripts' / file, target / file)
                (target / filename).write_text(source.replace(old, new))
                env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
                env.update(PYTHONPATH=str(target), PYTHONDONTWRITEBYTECODE='1')
                check = subprocess.run([sys.executable, '-m', 'unittest', test, '-v'],
                                       cwd=target.parent, env=env, text=True, capture_output=True, timeout=15)
                rejected = (check.returncode != 0 and 'FAIL:' in check.stderr
                            and 'AssertionError' in check.stderr and 'ERROR:' not in check.stderr)
                report['mutations'].append({'name': name, 'assertion_rejected': rejected})
                print(name + ': ' + ('rejected' if rejected else 'NOT rejected'), file=sys.stderr)
                if not rejected: print(check.stderr, file=sys.stderr)
    report['sources_unchanged'] = before == hashes()
    report['passed'] = (result.wasSuccessful() and not result.skipped and report['sources_unchanged']
                       and (not args.mutations or len(report['mutations']) == len(MUTATIONS)
                            and all(row['assertion_rejected'] for row in report['mutations'])))
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
