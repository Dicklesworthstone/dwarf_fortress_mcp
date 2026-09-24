#!/usr/bin/env python3
"""Execute actual excavation-run C++ with injected deterministic callbacks.

No DFHack SDK, native game, Rust/MCP, or production qualification is implied.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
FILES = ('bridge/common/bounded_run.h', 'bridge/common/excavation_run.h',
         'tests/native/excavation_run/engine.cpp')
FLAGS = ['-std=c++17', '-Wall', '-Wextra', '-Werror', '-pedantic',
         '-fsanitize=undefined', '-fno-sanitize-recover=all']


def execute(root: Path, output: Path, compiler: str) -> subprocess.CompletedProcess:
    subprocess.run([compiler, *FLAGS, '-I', str(root),
                    str(root / FILES[-1]), '-o', str(output)], check=True,
                   capture_output=True, text=True, timeout=60)
    return subprocess.run([str(output)], capture_output=True, text=True, timeout=20)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', default='g++')
    parser.add_argument('--mutations', action='store_true')
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    compiler = shutil.which(args.compiler)
    if compiler is None:
        parser.error('requested compiler unavailable')
    report = {'scope': 'actual C++ engine; deterministic injected effects',
              'real_dfhack_sdk': False, 'live_fortress': False, 'rust_mcp': False,
              'compiler': subprocess.check_output([compiler, '--version'], text=True).splitlines()[0],
              'flags': FLAGS,
              'source_sha256': {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in FILES},
              'rejected_mutants': []}
    with tempfile.TemporaryDirectory(prefix='dfmcp-excavation-run-') as directory:
        scratch = Path(directory)
        result = execute(ROOT, scratch / 'engine', compiler)
        if result.returncode:
            raise RuntimeError(result.stderr or result.stdout)
        report.update(json.loads(result.stdout))
        if args.mutations:
            original = (ROOT / FILES[1]).read_text()
            mutants = {
                'same_tick_samples': ('&& tick > record.last_tick', ''),
                'gap_streak_survives': ('tick - record.last_tick > record.goal.max_gap', 'false'),
                'wall_is_floor': ('shape == 3 && !liquid && !dig', 'shape == 2 && !liquid && !dig'),
                'stale_capture_accepted': ('require(current == record.before, 6);', '(void)current;'),
            }
            for name, (old, new) in mutants.items():
                if original.count(old) != 1:
                    raise RuntimeError('mutation target changed: ' + name)
                root = scratch / name
                for path in FILES:
                    destination = root / path
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    destination.write_bytes((ROOT / path).read_bytes())
                (root / FILES[1]).write_text(original.replace(old, new))
                result = execute(root, scratch / name / 'engine', compiler)
                if result.returncode == 0:
                    raise RuntimeError('mutant survived: ' + name)
                report['rejected_mutants'].append(name)
    rendered = json.dumps(report, indent=2, sort_keys=True) + '\n'
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(rendered)
    print(rendered, end='')


if __name__ == '__main__':
    main()
