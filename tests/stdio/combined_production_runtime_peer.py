#!/usr/bin/env python3
"""Run the actual Rust Runtime against the existing native TCP peer doubles.

This driver supplies only test environment and native peers. It does not create
an MCP server, replace runtime code, or claim live-game/production evidence.
"""
from __future__ import annotations

import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True


def main() -> int:
    executable, test_name, case = sys.argv[1:]
    assert case in ('growth', 'refresh')
    fixture_path = Path(__file__).with_name('combined_production.py')
    spec = importlib.util.spec_from_file_location('combined_runtime_native_fixture', fixture_path)
    fixture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixture)
    with tempfile.TemporaryDirectory(prefix='dfmcp-runtime-preflight-') as temporary:
        directory = Path(temporary).resolve()
        peer = fixture.Peer(directory)
        try:
            peer.inventory_stack = 7
            peer.after_operations_read = lambda: setattr(peer, 'inventory_stack', 11)
            environment = fixture.environment(directory, peer.endpoint)
            environment['COMBINED_RUNTIME_PREFLIGHT_TEST_CASE'] = case
            environment['COMBINED_RUNTIME_NATIVE_LOG'] = str(directory / 'native-requests.jsonl')
            environment['PYTHONDONTWRITEBYTECODE'] = '1'
            result = subprocess.run(
                [executable, '--exact', test_name, '--nocapture'],
                env=environment, timeout=180, check=False,
            )
            assert not peer.workforce_applied and not peer.order_created
            assert peer.workforce_plan is None and peer.order_plan is None
            assert not peer.clock_commits and not peer.inventory_clock_commits
            assert not any(method.startswith(('Prepare', 'Commit', 'Cancel'))
                           for _, method in peer.calls)
            return result.returncode
        finally:
            peer.close()


if __name__ == '__main__':
    sys.exit(main())
