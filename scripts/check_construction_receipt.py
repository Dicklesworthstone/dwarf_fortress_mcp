#!/usr/bin/env python3
"""Run real receipt-monitor tests and independently weakened implementations."""
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
INPUTS = ('scripts/construction_receipt.py', 'scripts/test_construction_receipt.py',
          'scripts/build_placement_wire.py', 'scripts/check_construction_receipt.py',
          'bridge/common/tests/fixtures/build_placement_v1_19.json')
MUTANTS = {
    'job_disappearance_is_completion': (
        'if building.stage != p.max_stage:', 'if False:',
        'OperationsTests.test_building_footprint_stage_missing_and_reused_id'),
    'item_substitution_allowed': (
        "return replace(outcome, status='item_identity_mismatch')", 'pass',
        'OperationsTests.test_all_item_flag_words_and_exact_item_identity'),
    'paused_replays_count': (
        'if state.last_tick != observed.tick and (current.counted_tick is None\n                or observed.tick - current.counted_tick >= goal.interval):',
        'if True:', 'LinkedGoalTests.test_distinct_advancing_samples_not_paused_replays'),
    'interrupted_streak_survives': (
        'if state.reading:', 'if False:',
        'LinkedGoalTests.test_negative_same_tick_gap_and_interruption_reset_stability'),
}


def run(root: Path, test: str = 'test_construction_receipt') -> subprocess.CompletedProcess:
    environment = dict(os.environ, PYTHONPATH=str(root / 'scripts'), PYTHONDONTWRITEBYTECODE='1')
    return subprocess.run([sys.executable, '-m', 'unittest', test, '-v'],
                          cwd=root, env=environment, capture_output=True, text=True, timeout=90)


def check(mutations: bool) -> dict:
    hashes = {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in INPUTS}
    result = run(ROOT)
    if result.returncode:
        raise RuntimeError(result.stderr)
    killed = []
    if mutations:
        with tempfile.TemporaryDirectory(prefix='dfmcp-construction-regressions-') as directory:
            root = Path(directory)
            for path in INPUTS:
                destination = root / path
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / path, destination)
            source = (ROOT / INPUTS[0]).read_text()
            for name, (before, after, test) in MUTANTS.items():
                if source.count(before) != 1:
                    raise AssertionError('mutation point changed: ' + name)
                (root / INPUTS[0]).write_text(source.replace(before, after))
                result = run(root, 'test_construction_receipt.' + test)
                if not result.returncode or 'FAIL:' not in result.stderr or 'ERROR:' in result.stderr:
                    raise AssertionError('mutant not rejected by a regression assertion: ' + name + '\n' + result.stderr)
                killed.append(name)
    if hashes != {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in INPUTS}:
        raise RuntimeError('source changed during evidence capture')
    return {'schema': 'dfmcp.construction-receipt-check/1', 'status': 'passed_python_only',
            'test_functions': 16, 'python_implementation_executed': True,
            'rejected_mutants': killed, 'source_sha256': hashes,
            'native_plugin_executed': False, 'live_game': False,
            'rust_mcp_executed': False, 'production_admitted': False}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mutations', action='store_true')
    args = parser.parse_args()
    print(json.dumps(check(args.mutations), indent=2, sort_keys=True))
