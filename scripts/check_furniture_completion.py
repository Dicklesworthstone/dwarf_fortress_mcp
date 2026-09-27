#!/usr/bin/env python3
"""Execute original furnishing-plan completion and its placement/monitor regressions.

Reports exact Python/POSIX/TCP source evidence, without native or MCP admission.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
TESTS = (
    'test_furniture_plan', 'test_furniture_batch',
    'test_construction_receipt', 'test_construction_monitor_rpc', 'test_construction_monitor_store',
    'test_construction_plan', 'test_construction_plan_rpc', 'test_construction_plan_store',
    'test_track_construction_plan', 'test_furniture_completion',
    'test_furniture_completion_store', 'test_track_furniture_batch',
)
SOURCES = (
    'furniture_plan', 'furniture_batch', 'build_placement_wire', 'build_placement_rpc',
    'build_placement_store', 'build_placement_client', 'construction_receipt',
    'construction_monitor_rpc', 'construction_monitor_store', 'track_construction',
    'construction_plan', 'construction_plan_rpc', 'construction_plan_store',
    'track_construction_plan', 'furniture_completion', 'furniture_completion_store',
    'track_furniture_batch', 'check_furniture_completion',
)
INPUTS = (
    *(f'scripts/{name}.py' for name in (*SOURCES, *TESTS)),
    'architecture/furniture_completion_v1.json',
    'bridge/common/tests/fixtures/build_placement_v1_19.json',
)


def check() -> dict:
    hashes = {path: hashlib.sha256((ROOT / path).read_bytes()).hexdigest() for path in INPUTS}
    environment = dict(os.environ, PYTHONPATH=str(ROOT / 'scripts'), PYTHONDONTWRITEBYTECODE='1')
    run = subprocess.run([sys.executable, '-m', 'unittest', *TESTS, '-q'], cwd=ROOT,
                         env=environment, text=True, capture_output=True, timeout=240)
    if run.returncode:
        raise RuntimeError(run.stdout + run.stderr)
    import furniture_completion as core
    import furniture_completion_store as store
    import construction_plan_rpc as rpc
    import track_furniture_batch as cli
    contract = json.loads((ROOT / 'architecture/furniture_completion_v1.json').read_bytes())
    expected = {
        'steps': [1, 32], 'origin_bytes': core.MAX_ORIGIN, 'goal_bytes': core.MAX_GOAL,
        'sample_bytes': core.MAX_SAMPLE, 'operations_capture_bytes': 16 * 1024 * 1024,
        'rpc_calls': rpc.MAX_RPC_CALLS, 'journal_bytes': store.MAX_FILE,
        'journal_frames': store.MAX_FRAMES, 'observations': 512, 'output_bytes': cli.MAX_OUTPUT,
        'timeout_ms': [1, 60000],
    }
    assert contract['bounds'] == expected, 'completion contract and executable bounds differ'
    assert contract['goal_policy'] == core.POLICY
    for field, value in (('origin_magic', core.ORIGIN_MAGIC), ('goal_magic', core.GOAL_MAGIC),
                         ('journal_magic', store.MAGIC)):
        assert contract['custody'][field].encode() == value
    assert contract['custody']['journal_checksum_domain'].encode() + b'\0' == store.DOMAIN
    counts = {name: unittest.defaultTestLoader.loadTestsFromName(name).countTestCases() for name in TESTS}
    assert hashes == {path: hashlib.sha256((ROOT / path).read_bytes()).hexdigest() for path in INPUTS}, \
        'source changed during furnishing completion validation'
    return {
        'schema': 'dfmcp.furniture-completion-evidence/1',
        'status': 'passed_python_posix_loopback_only', 'python': platform.python_version(),
        'test_functions': sum(counts.values()), 'test_functions_by_module': counts,
        'execution_metrics': run.stdout.splitlines(), 'machine_contract_checked': True,
        'source_sha256': hashes,
        'actual_tcp_and_subprocesses_executed': True, 'original_placement_workflow_executed': True,
        'native_plugin_executed': False, 'live_game': False, 'physical_power_loss': False,
        'rust_mcp_executed': False, 'full_repository_qualification': False, 'production_admitted': False,
    }


if __name__ == '__main__':
    print(json.dumps(check(), indent=2, sort_keys=True))
