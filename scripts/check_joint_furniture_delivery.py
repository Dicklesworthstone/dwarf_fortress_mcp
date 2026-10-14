#!/usr/bin/env python3
"""Execute actual joint-delivery code, bounded oracles and semantic mutants."""
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
MODULES = ('test_joint_furniture_delivery', 'test_cross_level_furniture',
           'test_cross_level_furniture_review', 'test_cross_level_furniture_frontier',
           'test_furniture_delivery_workflow', 'test_joint_furniture_workflow')
FILES = ('furniture_plan.py', 'furniture_allocation.py', 'build_placement_wire.py',
         'route_furniture_allocation.py', 'route_furniture_sequence.py', 'route_furniture_review.py',
         'route_furniture_frontier.py', 'route_furniture_suffix.py',
         'route_furniture_review_runtime.py', 'review_route_furniture.py',
         'route_furniture_joint.py', 'check_joint_furniture_delivery.py',
         *(name + '.py' for name in MODULES))
MUTATIONS = (
    ('skip_alternative_search', "if result['status'] == 'delivery_order_blocked':", 'if False:',
     'test_alternative_item_rescues_mandatory_blocking_dependency'),
    ('greedy_single_item', 'if len(heap) < len(remaining):', 'if len(heap) < 1:',
     'test_backtracking_does_not_steal_a_material_critical_item'),
    ('premature_inventory_trimming', 'for item in candidates:\n        work()\n        if item.id in used',
     'for item in candidates[:len(remaining)]:\n        work()\n        if item.id in used',
     'test_full_inventory_scanned_after_blocked_routing_not_initial_best_n'),
    ('exhaustion_is_infeasibility', 'search_complete=not cut_off, infeasibility_proven=not cut_off',
     'search_complete=True, infeasibility_proven=True',
     'test_node_exhaustion_never_claims_infeasibility_or_exports_partial_plan'),
    ('unbound_search_policy', "'max_search_nodes': max_search_nodes", "'max_search_nodes': MAX_SEARCH_NODES",
     'test_deterministic_input_permutations_and_bound_identity'),
)


def hashes():
    return {name: hashlib.sha256((ROOT / 'scripts' / name).read_bytes()).hexdigest() for name in FILES}


def main():
    before = hashes()
    result = unittest.TextTestRunner(verbosity=2, stream=sys.stderr).run(
        unittest.defaultTestLoader.loadTestsFromNames(MODULES))
    mutants = []
    if result.wasSuccessful() and not result.skipped:
        original = (ROOT / 'scripts/route_furniture_joint.py').read_text()
        for name, old, new, case in MUTATIONS:
            if original.count(old) != 1:
                raise ValueError('mutation site must be unique: ' + name)
            with tempfile.TemporaryDirectory(prefix='dfmcp-joint-test-') as directory:
                target = Path(directory)
                for file in FILES:
                    shutil.copyfile(ROOT / 'scripts' / file, target / file)
                (target / 'route_furniture_joint.py').write_text(original.replace(old, new))
                env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
                env.update(PYTHONDONTWRITEBYTECODE='1', PYTHONPATH=str(target))
                run = subprocess.run([sys.executable, '-m', 'unittest',
                    'test_joint_furniture_delivery.JointDeliveryTests.' + case], cwd=target,
                    env=env, capture_output=True, text=True, timeout=20, check=False)
                killed = (run.returncode == 1 and 'FAIL:' in run.stderr
                          and 'AssertionError' in run.stderr and 'ERROR:' not in run.stderr)
                mutants.append({'name': name, 'assertion_rejected': killed})
    unchanged = hashes() == before
    passed = result.wasSuccessful() and not result.skipped and unchanged and len(mutants) == len(MUTATIONS) and all(m['assertion_rejected'] for m in mutants)
    print(json.dumps({'schema': 'dfmcp.joint-furniture-delivery-tests/1', 'passed': passed,
        'tests': result.testsRun, 'failures': len(result.failures), 'errors': len(result.errors),
        'skipped': len(result.skipped), 'mutations': mutants, 'sources_unchanged': unchanged,
        'sha256': before, 'python': sys.version.split()[0], 'new_obstacle_masks': 512,
        'new_assignment_order_oracles': 300, 'native_acquisition_executed': False,
        'private_batch_executed': False, 'rust_mcp_executed': False, 'live_game': False,
        'scope': 'actual Python core; existing workflow suite has explicit inventory/packet doubles'},
        sort_keys=True, indent=2))
    return 0 if passed else 1


if __name__ == '__main__':
    raise SystemExit(main())
