#!/usr/bin/env python3
"""Execute allocation regressions and semantic mutation checks; emit exact evidence."""
from __future__ import annotations

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
FILES = (
    'scripts/furniture_plan.py', 'scripts/build_placement_wire.py',
    'scripts/construction_receipt.py', 'scripts/construction_monitor_rpc.py',
    'scripts/furniture_allocation.py', 'scripts/furniture_inventory.py',
    'scripts/allocate_furniture.py', 'scripts/test_furniture_allocation.py',
    'scripts/test_furniture_inventory.py', 'scripts/check_furniture_allocation.py',
)
MUTATIONS = (
    ('distance_ignored', 'scripts/furniture_allocation.py',
     'value * scale + identity * base**(n - row - 1)', 'identity * base**(n - row - 1)',
     'test_furniture_allocation.AllocationTests.test_sum_distance_precedes_id_vector_and_lexical_slots_break_ties'),
    ('greedy_candidate_reduction', 'scripts/furniture_allocation.py',
     'if len(heaps[row]) < n:', 'if len(heaps[row]) < 1:',
     'test_furniture_allocation.AllocationTests.test_global_assignment_avoids_greedy_material_starvation'),
    ('cross_level_candidates', 'scripts/furniture_allocation.py',
     'slot.kind != item.kind or slot.target[2] != item.position[2]', 'slot.kind != item.kind',
     'test_furniture_allocation.AllocationTests.test_same_level_distance_subtype_material_kind_and_exclusions'),
    ('partial_plan_on_shortage', 'scripts/furniture_allocation.py',
     "result['shortage'] = {'slots':", "result['plan'] = {'schema': 'dfmcp.furniture-plan/1', 'steps': []}\n        result['shortage'] = {'slots':",
     'test_furniture_allocation.AllocationTests.test_hall_shortage_not_individual_counts_and_no_partial_plan'),
    ('projected_flags_ignored', 'scripts/furniture_inventory.py',
     'elif value.flags != (1 << 6):', 'elif False:',
     'test_furniture_inventory.ProjectionTests.test_all_512_native_flag_words_and_candidate_policy'),
    ('job_attachment_ignored', 'scripts/furniture_inventory.py',
     'or value.id in linked or value.id in containing', 'or value.id in containing',
     'test_furniture_inventory.ProjectionTests.test_job_attachment_beats_apparently_free_flags'),
    ('foreign_world_adopted', 'scripts/furniture_inventory.py',
     'require((observed.folder, observed.site) == (request.folder, request.site),', 'require(True,',
     'test_furniture_inventory.ProjectionTests.test_foreign_fortress_refuses_instead_of_relabeling_inventory'),
    ('release_token_not_verified', 'scripts/construction_monitor_rpc.py',
     "require(release[10] == token, 'native release acknowledgment changed token')",
     "require(True, 'native release acknowledgment changed token')",
     'test_furniture_inventory.TransportTests.test_complete_bytes_without_verified_release_never_publish'),
    ('capture_digest_not_verified', 'scripts/construction_monitor_rpc.py',
     "require(hashlib.sha256(raw).digest() == identity[2], 'whole native capture digest mismatch')",
     "require(True, 'whole native capture digest mismatch')",
     'test_furniture_inventory.TransportTests.test_bad_binding_handshake_and_page_shapes_never_return_plan'),
    ('post_read_revocation_ignored', 'scripts/furniture_inventory.py',
     '# Cached allocation facts must not bypass a revoked read configuration.\n        authority.guard()',
     '# Weakened publication deliberately omits the final authority check.',
     'test_furniture_inventory.TransportTests.test_revocation_after_native_read_cannot_publish_cached_allocation'),
)


def main() -> int:
    suite = unittest.defaultTestLoader.loadTestsFromNames(('test_furniture_allocation', 'test_furniture_inventory'))
    result = unittest.TextTestRunner(verbosity=2, stream=sys.stderr).run(suite)
    report = {'schema': 'dfmcp.furniture-allocation-integration-evidence/1',
              'scope': 'executed Python, exact existing dependencies, independent wire encoder, joined TCP test peer and real CLI subprocess; not DFHack',
              'python': sys.version.split()[0], 'tests_run': result.testsRun,
              'failures': len(result.failures), 'errors': len(result.errors), 'skipped': len(result.skipped),
              'exhaustive_graphs': 4096, 'random_unpruned_oracles': 150,
              'native_flag_words_checked': 512, 'maximum_inventory_items_executed': 65536,
              'maximum_slots_executed': 32, 'mutations': [], 'sources': {},
              'rust_mcp_executed': False, 'real_dfhack_sdk': False, 'live_fortress': False,
              'whole_workspace_qualified': False, 'production_admitted': False}
    for name in FILES:
        raw = (ROOT / name).read_bytes()
        report['sources'][name] = {'sha256': hashlib.sha256(raw).hexdigest(),
                                   'git_blob': hashlib.sha1(f'blob {len(raw)}\0'.encode() + raw).hexdigest()}
    from test_furniture_inventory import MEASUREMENTS
    report['measured'] = dict(MEASUREMENTS)
    if result.wasSuccessful() and not result.skipped:
        for name, file, before, after, test in MUTATIONS:
            source = (ROOT / file).read_text()
            if source.count(before) != 1:
                raise RuntimeError('mutation source changed: ' + name)
            with tempfile.TemporaryDirectory(prefix='dfmcp-allocation-mutant-') as directory:
                target = Path(directory)
                (target / 'scripts').mkdir()
                for item in FILES:
                    shutil.copyfile(ROOT / item, target / item)
                (target / file).write_text(source.replace(before, after))
                env = {key: value for key, value in os.environ.items() if not key.startswith('DFMCP_')}
                env.update(PYTHONPATH=str(target / 'scripts'), PYTHONDONTWRITEBYTECODE='1')
                run = subprocess.run([sys.executable, '-m', 'unittest', test, '-v'], cwd=target,
                                     env=env, capture_output=True, text=True, timeout=20, check=False)
                # A syntax/import/crash error is not a semantic regression kill.
                rejected = (run.returncode != 0 and 'FAIL:' in run.stderr
                            and 'AssertionError' in run.stderr and 'ERROR:' not in run.stderr)
                report['mutations'].append({'name': name, 'source': file, 'test': test,
                                             'assertion_rejected': rejected, 'returncode': run.returncode})
                print(f'{name}: {"assertion rejected" if rejected else "NOT REJECTED"}', file=sys.stderr)
    passed = (result.wasSuccessful() and not result.skipped and len(report['mutations']) == len(MUTATIONS)
              and all(value['assertion_rejected'] for value in report['mutations']))
    report['passed'] = passed
    print(json.dumps(report, indent=2))
    return 0 if passed else 1


if __name__ == '__main__':
    raise SystemExit(main())
