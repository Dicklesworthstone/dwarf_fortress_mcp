#!/usr/bin/env python3
"""Run actual construction workflow tests and weakened-publication regressions.

This source-bound report is Python/POSIX/loopback evidence only. It neither builds
nor qualifies native DFHack, Rust/MCP, a live fortress or physical power loss.
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

ROOT = Path(__file__).resolve().parents[1]
INPUTS = (
    'architecture/construction_monitor_v1.json',
    'scripts/construction_receipt.py', 'scripts/construction_monitor_rpc.py',
    'scripts/construction_monitor_store.py', 'scripts/track_construction.py',
    'scripts/build_placement_wire.py', 'scripts/test_construction_receipt.py',
    'scripts/test_construction_monitor_rpc.py', 'scripts/test_construction_monitor_store.py',
    'scripts/check_construction_monitor.py', 'scripts/check_construction_receipt.py',
    'bridge/common/tests/fixtures/build_placement_v1_19.json',
)
TESTS = ('test_construction_receipt', 'test_construction_monitor_rpc', 'test_construction_monitor_store')
MUTANTS = {
    'missing_pre_read_sync': (
        '            write_all(self.fd, raw, self.budget)\n            os.fsync(self.fd)\n            os.fsync(self.directory)',
        '            write_all(self.fd, raw, self.budget)',
        'CustodyTests.test_file_parent_sync_failure_preserves_uncertainty'),
    'reopened_read_mints_publication': (
        "require(self.read_owned, 'reopened or unowned read cannot publish a sample')",
        "require(True, 'reopened or unowned read cannot publish a sample')",
        'CustodyTests.test_reopened_read_cannot_publish_and_interrupt_resets_streak'),
    'old_bytes_not_reverified': (
        ' and digest.digest() == self._digest.digest()', '',
        'CustodyTests.test_same_size_substitution_and_named_file_replacement_fence_owner'),
    'publication_before_output_reservation': (
        '            render(candidate)\n            self.budget.remaining()',
        '            self.budget.remaining()',
        'CustodyTests.test_failed_result_reservation_never_publishes_sample'),
}


def execute(root: Path, tests: tuple[str, ...]) -> subprocess.CompletedProcess:
    environment = dict(os.environ, PYTHONPATH=str(root / 'scripts'), PYTHONDONTWRITEBYTECODE='1')
    return subprocess.run([sys.executable, '-m', 'unittest', *tests, '-v'], cwd=root,
                          env=environment, text=True, capture_output=True, timeout=90)


def check(mutations: bool) -> dict:
    hashes = {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in INPUTS}
    result = execute(ROOT, TESTS)
    if result.returncode:
        raise RuntimeError(result.stderr)
    import unittest
    count = unittest.defaultTestLoader.loadTestsFromNames(TESTS).countTestCases()
    # The machine contract uses the actual implementation's fixed bounds.
    import construction_monitor_rpc as rpc
    import construction_monitor_store as store
    import track_construction as cli
    from construction_receipt import MAX_CAPTURE
    contract = json.loads((ROOT / INPUTS[0]).read_bytes())
    assert contract['native_bindings'] == [[rpc.PROFILES[family][0], name] for family, name in rpc.BINDINGS]
    assert contract['client_environment'] == list(rpc.ENVIRONMENT)
    assert contract['journal']['magic'] == store.MAGIC.decode('ascii')
    assert contract['bounds']['journal_bytes'] == store.MAX_FILE
    assert contract['bounds']['journal_frames'] == store.MAX_FRAMES
    assert contract['bounds']['complete_capture_bytes'] == MAX_CAPTURE
    assert contract['bounds']['output_bytes'] == cli.MAX_OUTPUT
    killed = []
    if mutations:
        with tempfile.TemporaryDirectory(prefix='dfmcp-construction-publication-') as directory:
            root = Path(directory)
            for relative in INPUTS:
                target = root / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / relative, target)
            source_path = 'scripts/construction_monitor_store.py'
            source = (ROOT / source_path).read_text()
            for name, (before, after, test) in MUTANTS.items():
                if source.count(before) != 1:
                    raise AssertionError('publication mutation point changed: ' + name)
                (root / source_path).write_text(source.replace(before, after))
                result = execute(root, ('test_construction_monitor_store.' + test,))
                if not result.returncode or 'FAIL:' not in result.stderr or 'ERROR:' in result.stderr:
                    raise AssertionError('publication mutant not rejected by assertion: ' + name + '\n' + result.stderr)
                killed.append(name)
    if hashes != {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in INPUTS}:
        raise RuntimeError('source changed during workflow checks')
    return {'schema': 'dfmcp.construction-monitor-evidence/1', 'status': 'passed_python_posix_loopback_only',
            'python': platform.python_version(), 'actual_python_implementation_executed': True,
            'actual_tcp_and_subprocesses_executed': True, 'test_functions': count,
            'rejected_publication_mutants': killed, 'source_sha256': hashes,
            'native_plugin_executed': False, 'live_game': False, 'physical_power_loss': False,
            'rust_mcp_executed': False, 'full_repository_qualification': False, 'production_admitted': False}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mutations', action='store_true')
    args = parser.parse_args()
    print(json.dumps(check(args.mutations), indent=2, sort_keys=True))
