#!/usr/bin/env python3
"""Execute cross-level allocation, retained-review and prefix regressions.

This runs production Python modules with explicit map/receipt values. It does
not attest native acquisition, private batch custody, Rust/MCP or a live game.
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

ROOT = Path(__file__).resolve().parents[1]
MODULES = ('test_cross_level_furniture', 'test_cross_level_furniture_review',
           'test_cross_level_furniture_frontier')
FILES = ('furniture_plan.py', 'build_placement_wire.py', 'furniture_allocation.py',
         'route_furniture_allocation.py', 'route_furniture_review.py', 'route_furniture_frontier.py',
         *(name + '.py' for name in MODULES), 'check_cross_level_furniture.py')
MUTATIONS = (
    ('vertical_limit_ignored', 'route_furniture_allocation.py',
     'if vertical > slot.max_vertical_distance:', 'if False:',
     'test_cross_level_furniture.CrossLevelTests.test_default_refuses_other_floors_even_with_complete_stairs'),
    ('vertical_distance_not_charged', 'route_furniture_allocation.py',
     'horizontal + vertical > slot.max_distance', 'horizontal > slot.max_distance',
     'test_cross_level_furniture.CrossLevelTests.test_geometric_level_and_route_limits_are_independent_and_inclusive'),
    ('complementary_stairs_not_required', 'route_furniture_allocation.py',
     'if lower not in (6, 8) or upper not in (7, 8):', 'if False:',
     'test_cross_level_furniture.CrossLevelTests.test_all_stair_pair_orientations_require_a_connected_observed_path'),
    ('manhattan_allocator_silently_accepts_opt_in', 'furniture_allocation.py',
     'require(not any(slot.max_vertical_distance for slot in request.slots),', 'require(True,',
     'test_cross_level_furniture.CrossLevelTests.test_nonroute_allocator_rejects_vertical_opt_in'),
    ('review_skips_original_distance_limit', 'route_furniture_review.py',
     'elif distance(slot, item) is None:', 'elif False:',
     'test_cross_level_furniture_review.CrossLevelReviewTests.test_moved_original_item_can_use_another_allowed_level_but_never_exceed_bound'),
    ('native_binding_skips_original_limit', 'route_furniture_frontier.py',
     'and distance(slot, current) is not None,', ',',
     'test_cross_level_furniture_frontier.CrossLevelFrontierTests.test_native_before_cannot_relax_level_material_or_identity'),
)


def hashes():
    return {name: hashlib.sha256((ROOT / 'scripts' / name).read_bytes()).hexdigest() for name in FILES}


def execute(directory, names):
    # Only local Python code is executed, with no native credentials or network.
    environment = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
    environment.update(PYTHONPATH=str(directory), PYTHONDONTWRITEBYTECODE='1')
    return subprocess.run([sys.executable, '-m', 'unittest', *names, '-v'],
                          cwd=directory, env=environment, capture_output=True,
                          text=True, timeout=30, check=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--evidence', type=Path)
    parser.add_argument('--mutations', action='store_true')
    args = parser.parse_args()
    before = hashes()
    result = execute(ROOT / 'scripts', MODULES)
    sys.stderr.write(result.stderr)
    if result.returncode:
        raise RuntimeError('cross-level implementation regression failed')
    killed = []
    if args.mutations:
        with tempfile.TemporaryDirectory(prefix='dfmcp-cross-level-') as temporary:
            for label, filename, old, new, test in MUTATIONS:
                directory = Path(temporary) / label
                directory.mkdir()
                for name in FILES:
                    shutil.copyfile(ROOT / 'scripts' / name, directory / name)
                target = directory / filename
                source = target.read_text(encoding='utf-8')
                if source.count(old) != 1:
                    raise RuntimeError('mutation no longer has one exact location: ' + label)
                target.write_text(source.replace(old, new), encoding='utf-8')
                mutant = execute(directory, [test])
                # Broken imports or syntax do not qualify as an assertion kill.
                if (mutant.returncode != 1 or 'FAIL:' not in mutant.stderr
                        or 'AssertionError' not in mutant.stderr or 'ERROR:' in mutant.stderr):
                    raise RuntimeError('mutant escaped expected assertion: ' + label + '\n' + mutant.stderr)
                killed.append(label)
                print(label + ': assertion rejected', file=sys.stderr)
    if hashes() != before:
        raise RuntimeError('production or test source changed during execution')
    report = {'schema': 'dfmcp.cross-level-furniture-evidence/1', 'status': 'passed_python_components',
              'python': sys.version.split()[0], 'unittest_functions': 26,
              'stair_pair_cases': 81, 'random_unpruned_assignment_cases': 180,
              'maximum_slots': 32, 'maximum_endpoint_level_difference': 127,
              'rejected_mutants': killed, 'source_sha256': before,
              'actual_python_modules_executed': True, 'real_dfhack_sdk': False,
              'native_transport_executed': False, 'private_batch_parent_executed': False,
              'rust_mcp_executed': False, 'live_fortress': False,
              'full_repository_qualification': False, 'production_admitted': False}
    text = json.dumps(report, indent=2, sort_keys=True) + '\n'
    if args.evidence is not None:
        args.evidence.write_text(text, encoding='utf-8')
    print(text, end='')


if __name__ == '__main__':
    main()
