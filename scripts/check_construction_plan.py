#!/usr/bin/env python3
"""Execute whole-plan construction, original-profile and fault regressions.

The source-bound result describes Python, real POSIX files and joined TCP peers.
It grants no native DFHack, live-game, Rust/MCP or production qualification.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
TESTS = (
    'test_construction_receipt', 'test_construction_monitor_rpc', 'test_construction_monitor_store',
    'test_construction_plan', 'test_construction_plan_rpc', 'test_construction_plan_store',
    'test_track_construction_plan',
)
INPUTS = (
    'architecture/construction_plan_monitor_v1.json',
    'architecture/construction_monitor_v1.json',
    'scripts/build_placement_wire.py',
    'scripts/construction_receipt.py', 'scripts/construction_monitor_rpc.py',
    'scripts/construction_monitor_store.py', 'scripts/track_construction.py',
    'scripts/construction_plan.py', 'scripts/construction_plan_rpc.py',
    'scripts/construction_plan_store.py', 'scripts/track_construction_plan.py',
    *(f'scripts/{name}.py' for name in TESTS),
    'scripts/check_construction_plan.py',
    'bridge/common/tests/fixtures/build_placement_v1_19.json',
)
MUTANTS = {
    'trailing_original_receipts_not_verified': (
        'scripts/construction_plan.py',
        'self.before_records == self.after_records == goal.receipts',
        'self.before_records == goal.receipts',
        'test_construction_plan.WholeSelectionTests.test_every_original_receipt_and_bracket_must_match'),
    'later_targets_not_evaluated': (
        'scripts/construction_plan.py',
        'for child in goal.goals:\n        guard()',
        'for child in goal.goals[:1]:\n        guard()',
        'test_construction_plan.WholeSelectionTests.test_late_target_failure_is_not_hidden_by_first_pending_or_satisfied_row'),
    'trailing_native_queries_not_executed': (
        'scripts/construction_plan_rpc.py',
        'after_records = tuple(self._member_receipt(goal) for goal in self.member_goals)',
        'after_records = before_records',
        'test_construction_plan_rpc.PlanTransportTests.test_all_originals_bracket_one_capture_on_one_connection'),
    'reopened_read_mints_plan_publication': (
        'scripts/construction_monitor_store.py',
        "require(self.read_owned, 'reopened or unowned read cannot publish a sample')",
        "require(True, 'reopened or unowned read cannot publish a sample')",
        'test_construction_plan_store.PlanCustodyTests.test_reopened_read_has_no_publication_permit_and_resets_entire_streak'),
    'plan_publication_before_result_reservation': (
        'scripts/construction_monitor_store.py',
        '            render(candidate)\n            self.budget.remaining()',
        '            self.budget.remaining()',
        'test_construction_plan_store.PlanCustodyTests.test_failed_whole_result_reservation_retains_only_unknown_read'),
}


def execute(root: Path, tests: tuple[str, ...]) -> subprocess.CompletedProcess:
    environment = dict(os.environ, PYTHONPATH=str(root / 'scripts'), PYTHONDONTWRITEBYTECODE='1')
    return subprocess.run([sys.executable, '-m', 'unittest', *tests, '-q'], cwd=root,
                          env=environment, text=True, capture_output=True, timeout=240)


def check(mutations: bool) -> dict:
    hashes = {path: hashlib.sha256((ROOT / path).read_bytes()).hexdigest() for path in INPUTS}
    result = execute(ROOT, TESTS)
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)
    counts = {name: unittest.defaultTestLoader.loadTestsFromName(name).countTestCases() for name in TESTS}
    import construction_plan as core
    import construction_plan_rpc as rpc
    import construction_plan_store as store
    import construction_monitor_rpc as original
    import track_construction_plan as cli
    contract = json.loads((ROOT / INPUTS[0]).read_bytes())
    bounds = contract['bounds']
    budget = rpc.Budget(10000)
    expected = {
        'targets': [1, core.MAX_TARGETS], 'canonical_goal_bytes': core.MAX_GOAL,
        'canonical_sample_bytes': core.MAX_SAMPLE, 'complete_capture_bytes': core.MAX_CAPTURE,
        'receipt_input_bytes': cli.MAX_RECEIPT_INPUT, 'output_bytes': cli.MAX_OUTPUT,
        'rpc_calls': rpc.MAX_RPC_CALLS, 'journal_bytes': store.MAX_FILE,
        'journal_frames': store.MAX_FRAMES, 'journal_body_bytes': store.MAX_BODY,
        'goal_observations': core.MAX_OBSERVATIONS, 'page_bytes': original.PAGE,
        'pages': core.MAX_CAPTURE // original.PAGE, 'network_bytes': budget.network_bytes,
        'cooperative_work_steps': budget.work_steps, 'custody_bytes': budget.disk_bytes,
    }
    assert all(bounds[name] == value for name, value in expected.items()), 'contract bounds differ from execution'
    assert budget.calls == rpc.MAX_RPC_CALLS
    assert contract['native_bindings'] == [[original.PROFILES[f][0], n] for f, n in original.BINDINGS]
    assert contract['client_environment'] == list(original.ENVIRONMENT)
    assert contract['condition_policy'] == core.POLICY
    assert contract['receipt_bundle']['schema'] == cli.RECEIPT_SCHEMA
    assert contract['journal']['magic'] == store.MAGIC.decode('ascii')
    assert contract['journal']['checksum_domain'].encode() + b'\0' == store.DOMAIN
    killed = []
    if mutations:
        with tempfile.TemporaryDirectory(prefix='dfmcp-construction-plan-regressions-') as directory:
            root = Path(directory)
            for path in INPUTS:
                target = root / path
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / path, target)
            for name, (path, before, after, test) in MUTANTS.items():
                source = (ROOT / path).read_text()
                if source.count(before) != 1:
                    raise AssertionError('mutation location changed: ' + name)
                (root / path).write_text(source.replace(before, after))
                result = execute(root, (test,))
                (root / path).write_text(source)
                if not result.returncode or 'FAIL:' not in result.stderr or 'ERROR:' in result.stderr:
                    raise AssertionError('mutant was not rejected by assertions: ' + name + '\n' + result.stderr)
                killed.append(name)
    if hashes != {path: hashlib.sha256((ROOT / path).read_bytes()).hexdigest() for path in INPUTS}:
        raise RuntimeError('source changed during construction-plan validation')
    return {
        'schema': 'dfmcp.construction-plan-monitor-evidence/1',
        'status': 'passed_python_posix_loopback_only', 'python': platform.python_version(),
        'actual_python_implementation_executed': True,
        'actual_tcp_and_subprocesses_executed': True,
        'test_functions': sum(counts.values()), 'test_functions_by_module': counts,
        'machine_contract_bounds_checked': True, 'rejected_mutants': killed,
        'source_sha256': hashes,
        'native_plugin_executed': False, 'live_game': False, 'physical_power_loss': False,
        'rust_mcp_executed': False, 'full_repository_qualification': False, 'production_admitted': False,
    }


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mutations', action='store_true')
    args = parser.parse_args()
    print(json.dumps(check(args.mutations), indent=2, sort_keys=True))
