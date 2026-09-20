#!/usr/bin/env python3
"""Compile real dig/1.16 engine and handler with explicit SDK/protobuf doubles.

This does not qualify DFHack ABI, protobuf serialization or a running fortress.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
ENGINE = 'bridge/common/dig_designation_v1_16.h'
HANDLER = 'bridge/dfhack-dig-v1_16/dfmcp_dig_v1_16.cpp'
INPUTS = [ENGINE, HANDLER, 'bridge/common/retained_snapshot.h',
          'bridge/dfhack-dig-v1_16/DfmcpDigV1_16.proto',
          'tests/native/dig_designation/engine.cpp',
          'tests/native/dig_designation/handler.cpp',
          'tests/native/dig_designation/stubs.h', 'scripts/test_dig_designation_native.py']


def sha(domain: bytes, data: bytes) -> bytes:
    return hashlib.sha256(domain + b'\0' + data).digest()


def text(data: bytes) -> bytes:
    return struct.pack('>H', len(data)) + data


def reference_vectors() -> dict[str, bytes]:
    region = struct.pack('>IIIII', 15, 15, 2, 2, 2)
    def observation(after: bool) -> bytes:
        out = b'DFMDG016' + struct.pack('>QQQIIII', 7, int(after), 12345, 1, 64, 64, 8)
        out += region + b'\1' + text(b'region1') + struct.pack('>H', 48)
        for z in range(1, 4):
            for y in range(14, 18):
                for x in range(14, 18):
                    target = z == 2 and x in (15, 16) and y in (15, 16)
                    block = after and z == 2  # all four blocks touch this selection
                    out += b'\2' + struct.pack('>IIIIIIHHBBB', 42, 512, 0,
                        4000 if target and after else 0, 0 if block else 100, 8,
                        10015, 10015, int(target and after), 0, 17 if block else 1)
        return out
    before, after = observation(False), observation(True)
    witness = hashlib.sha256(before).digest()
    plan = sha(b'dfmcp-dig-designation-plan/1', region + b'\0' + witness)
    key = text(b'dig-001')
    token = sha(b'dfmcp-dig-designation-token/1', struct.pack('>Q', 7) + key + plan)[:16]
    def effect(state: int, reason: int) -> bytes:
        known = int(state == 2)
        suffix = struct.pack('>BBBI', state, reason, known, 4 if known else 0)
        suffix += hashlib.sha256(after).digest() if known else bytes(32)
        proof = sha(b'dfmcp-dig-designation-receipt/1', struct.pack('>Q', 7) + key + plan + token + suffix)
        return (b'DFMDGE16' + struct.pack('>QQQ', 7, 0, 12345) + region + b'\0'
                + witness + plan + token + suffix + (proof if state in (2, 4) else bytes(32)) + key)
    return {'observation': before, 'prepared': effect(0, 0),
            'designated': effect(2, 0), 'cancelled': effect(4, 2)}


def run(command: list[str], *, success: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(command, capture_output=True, text=True, timeout=35)
    if success and result.returncode:
        raise RuntimeError(f'{command[0]} failed: {result.stderr[:4000]}')
    return result


def compile_and_run(compiler: str, root: Path, work: Path, name: str) -> dict:
    include = work / 'include'
    include.mkdir(exist_ok=True)
    for header in re.findall(r'#include "([^"\n]+)"', (root / HANDLER).read_text()):
        if header.startswith('..'):
            continue
        target = include / header
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text('#pragma once\n')
    binary = work / name
    run([compiler, '-std=c++17', '-Wall', '-Wextra', '-Werror', '-pedantic',
         '-fsanitize=undefined', '-fno-sanitize-recover=all', f'-I{include}',
         str(root / f'tests/native/dig_designation/{name}.cpp'), '-o', str(binary)])
    return {'binary': str(binary), 'result': run([str(binary)], success=False)}


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--compiler', default='g++')
    p.add_argument('--mutations', action='store_true')
    args = p.parse_args()
    compiler = shutil.which(args.compiler)
    if compiler is None:
        raise SystemExit('requested C++ compiler unavailable')
    reports, rejected_mutants = {}, []
    with tempfile.TemporaryDirectory(prefix='dfmcp-dig-native-') as directory:
        work = Path(directory)
        for name in ('engine', 'handler'):
            result = compile_and_run(compiler, ROOT, work, name)
            if result['result'].returncode:
                raise RuntimeError(result['result'].stderr)
            reports[name] = json.loads(result['result'].stdout)
        emitted = run([str(work / 'engine'), '--vectors']).stdout.splitlines()
        vectors = {name: bytes.fromhex(data) for name, data in (line.split() for line in emitted)}
        if vectors != reference_vectors():
            raise AssertionError('actual C++ encoder differs from independent hashlib/struct reconstruction')
        for name, raw in vectors.items():
            path = ROOT / f'tests/native/dig_designation/vectors/{name}.hex'
            if bytes.fromhex(path.read_text()) != raw:
                raise AssertionError(f'checked-in vector mismatch: {name}')
        if args.mutations:
            mutations = [
                ('witness', ENGINE, 'current.witness() == r.witness &&', 'true &&', 'engine'),
                ('replay', ENGINE, 'if (r.state != State::Prepared) return r;', 'if (false) return r;', 'engine'),
                ('priority-readback', ENGINE, 'cell.priority = PRIORITY;', 'cell.priority = 0;', 'engine'),
                ('cross-key-uncertainty', ENGINE, 'require(!unresolved(),8);', '', 'engine'),
                ('hidden-redaction', HANDLER, 'if (des.bits.hidden)', 'if (false)', 'handler'),
                ('scheduling', HANDLER, 'block->dsgn_check_cooldown=0;', '/* omitted cooldown reset */', 'handler'),
            ]
            for label, path, old, new, entry in mutations:
                copy = work / label
                for relative in INPUTS:
                    target = copy / relative
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(ROOT / relative, target)
                original = (copy / path).read_text()
                if old not in original:
                    raise AssertionError(f'mutation no longer applies: {label}')
                (copy / path).write_text(original.replace(old, new))
                result = compile_and_run(compiler, copy, copy, entry)['result']
                if result.returncode == 0 or 'line ' not in result.stderr and 'expected rejection' not in result.stderr:
                    raise AssertionError(f'mutant not rejected by an assertion: {label}: {result.stderr}')
                rejected_mutants.append(label)
    report = {
        'schema': 'dfmcp.dig-native-evidence/1', 'status': 'passed_sdk_doubles_only',
        'compiler': run([compiler, '--version']).stdout.splitlines()[0],
        'engine': reports['engine'], 'actual_handler': reports['handler'],
        'independent_native_vectors': len(vectors), 'rejected_compiled_mutants': rejected_mutants,
        'warning_denied': True, 'ubsan': True, 'real_dfhack_sdk': False,
        'protobuf_serialization': False, 'live_fortress': False, 'rust_executed': False,
        'source_sha256': {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in INPUTS},
    }
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == '__main__':
    main()
