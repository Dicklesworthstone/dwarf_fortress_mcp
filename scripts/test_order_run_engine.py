#!/usr/bin/env python3
"""Execute the actual order-run engine with deterministic source/setter doubles.

Not a DFHack SDK, plugin-manager, real game, Rust or MCP qualification. Build
products and logs are retained. No runtime dependencies or external downloads.
"""
from pathlib import Path
import argparse
import shutil
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', action='append')
    parser.add_argument('--build-dir', type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    build = args.build_dir or Path(tempfile.mkdtemp(prefix='dfmcp-order-run-'))
    build.mkdir(parents=True, exist_ok=True)
    compilers = args.compiler or [c for c in ('g++', 'clang++') if shutil.which(c)]
    if not compilers:
        parser.error('a C++17 compiler is required')
    for i, compiler in enumerate(compilers):
        binary = build / f'engine-{i}'
        subprocess.run([compiler, '-std=c++17', '-Wall', '-Wextra', '-Werror', '-pedantic',
                        '-fsanitize=undefined', '-fno-sanitize-recover=all', '-g', '-I', str(root),
                        str(root / 'tests/native/order_run/engine.cpp'), '-o', str(binary)], check=True, timeout=90)
        result = subprocess.run([str(binary)], check=False, capture_output=True, text=True, timeout=20)
        (build / f'engine-{i}.log').write_text(result.stdout + result.stderr)
        if result.returncode:
            raise RuntimeError(result.stderr)
        print(f'{compiler}: {result.stdout.strip()}')
    print(f'Retained artifacts: {build}')


if __name__ == '__main__':
    main()
