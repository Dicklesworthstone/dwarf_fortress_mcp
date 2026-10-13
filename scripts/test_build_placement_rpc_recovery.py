#!/usr/bin/env python3
"""Execute original-placement recovery through actual RPC handlers with SDK doubles.

Beads: df-dfhack-bridge-plane-c-pic.4/.5. No real SDK, protobuf serialization,
live fortress or production qualification is claimed by this component runner.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
HANDLER = 'bridge/dfhack-build-v1_19/dfmcp_build_v1_19.cpp'
ENGINE = 'bridge/common/build_placement.h'
TEST = 'tests/native/build_placement/receipt_recovery.cpp'
INPUTS = (HANDLER, ENGINE, 'bridge/common/retained_snapshot.h', TEST,
          'tests/native/build_placement/stubs.h', 'scripts/test_build_placement_rpc_recovery.py')
FLAGS = ('-std=c++17', '-Wall', '-Wextra', '-Werror', '-pedantic',
         '-fsanitize=undefined', '-fno-sanitize-recover=all')


def execute(compiler: str, root: Path, work: Path) -> subprocess.CompletedProcess:
    include = work / 'include'
    include.mkdir(parents=True, exist_ok=True)
    for name in re.findall(r'#include "([^"\n]+)"', (root / HANDLER).read_text()):
        if not name.startswith('..'):
            header = include / name
            header.parent.mkdir(parents=True, exist_ok=True)
            header.write_text('#pragma once\n')
    binary = work / 'receipt-recovery'
    subprocess.run([compiler, *FLAGS, f'-I{include}', str(root / TEST), '-o', str(binary)],
                   check=True, capture_output=True, text=True, timeout=90)
    return subprocess.run([str(binary)], capture_output=True, text=True, timeout=30, check=False)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', default='g++')
    parser.add_argument('--mutations', action='store_true')
    parser.add_argument('--evidence', type=Path)
    args = parser.parse_args()
    compiler = shutil.which(args.compiler)
    if compiler is None:
        raise RuntimeError('requested compiler unavailable')
    hashes = {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in INPUTS}
    with tempfile.TemporaryDirectory(prefix='dfmcp-rpc-recovery-') as directory:
        work = Path(directory)
        result = execute(compiler, ROOT, work)
        if result.returncode:
            raise RuntimeError(result.stderr)
        report = json.loads(result.stdout)
        killed = []
        if args.mutations:
            changes = {
                'query_does_not_reconcile': (HANDLER,
                    'engine.reconcile(in->idempotency_key(), in->plan_digest(),\n            capture, verify)',
                    'engine.query(in->idempotency_key(), in->plan_digest())'),
                'missing_final_capture': (ENGINE,
                    ' || inspect(r.before.selection, read).encode() != expected', ''),
                'missing_source_frontier_fence': (ENGINE,
                    ' || generation_ != r.before.generation\n            || sequence_ != r.before.sequence + 1', ''),
            }
            for name, (path, before, after) in changes.items():
                clone = work / name
                for relative in INPUTS:
                    destination = clone / relative
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(ROOT / relative, destination)
                source = clone / path
                text = source.read_text()
                if text.count(before) != 1:
                    raise RuntimeError('mutation no longer applies exactly: ' + name)
                source.write_text(text.replace(before, after))
                mutant = execute(compiler, clone, clone)
                if mutant.returncode == 0 or 'check failed:' not in mutant.stderr:
                    raise RuntimeError('compiled mutant escaped regression: ' + name)
                killed.append(name)
        if hashes != {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in INPUTS}:
            raise RuntimeError('source changed during execution')
    report.update(source_sha256=hashes, flags=FLAGS, rejected_compiled_mutants=killed,
                  compiler=subprocess.run([compiler, '--version'], check=True, capture_output=True,
                                          text=True, timeout=10).stdout.splitlines()[0],
                  real_dfhack_sdk=False, live_fortress=False, protobuf_serialization=False)
    text = json.dumps(report, indent=2, sort_keys=True) + '\n'
    if args.evidence:
        args.evidence.write_text(text)
    print(text, end='')


if __name__ == '__main__':
    main()
