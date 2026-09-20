#!/usr/bin/env python3
"""Execute the real bounded-dig engine, not a translated state-machine model."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SOURCES = ['bridge/common/dig_designation.h', 'bridge/common/retained_snapshot.h',
           'tests/native/dig/engine.cpp', 'scripts/test_bounded_dig_engine.py']


def h(domain: str, value: bytes) -> bytes:
    return hashlib.sha256(domain.encode('ascii') + b'\0' + value).digest()


def text(value: bytes) -> bytes:
    return struct.pack('>H', len(value)) + value


def reference_vectors() -> dict[str, bytes]:
    region = struct.pack('>IIIII', 15, 15, 2, 2, 2)
    def observation(after: bool) -> bytes:
        out = b'DFMDG015' + struct.pack('>QQQIIII', 7, int(after), 12345, 1, 64, 64, 8)
        out += region + b'\1' + text(b'region1') + struct.pack('>I', 48)
        for z in range(1, 4):
            for y in range(14, 18):
                for x in range(14, 18):
                    target = z == 2 and 15 <= x <= 16 and 15 <= y <= 16
                    out += struct.pack('>BBBBBBBIIIIHH', 2, 1, 1, int(after and target), 0, 0,
                                       int(after and z == 2), 42, 0, 0, 0, 10015, 10015)
        return out
    before = observation(False)
    witness = hashlib.sha256(before).digest()
    plan = h('dfmcp-dig-plan/1', witness)
    key = text(b'mine-001')
    token = h('dfmcp-dig-token/1', struct.pack('>Q', 7) + key + plan)[:16]
    identity = b'DFMDGE15' + struct.pack('>QQQ', 7, 0, 12345) + region + witness + plan + token
    out = {'observation': before}
    for name, state in [('prepared', 0), ('designated', 2), ('refused', 4)]:
        prefix = identity + struct.pack('>BI', state, 4 if state == 2 else 0)
        prefix += hashlib.sha256(observation(True)).digest() if state == 2 else bytes(32)
        prefix += key
        out[name] = prefix + (h('dfmcp-dig-receipt/1', prefix) if state in (2, 4) else bytes(32))
    return out


def execute(command: list[str], *, ok: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(command, text=True, capture_output=True, timeout=90, check=False)
    if ok and result.returncode:
        raise RuntimeError(f'{command[0]} failed ({result.returncode}): {result.stderr[-4000:]}')
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', default='g++')
    parser.add_argument('--mutations', action='store_true')
    args = parser.parse_args()
    flags = ['-std=c++17', '-Wall', '-Wextra', '-Werror', '-pedantic',
             '-fsanitize=undefined', '-fno-sanitize-recover=all']
    mutations = {
        'commit_witness_removed': ('current.witness() == record.witness', 'true'),
        'terminal_replay_fence_removed': ('if (record.state != State::Prepared) return record;',
                                        'if (record.state == State::Refused) return record;'),
        'readback_equality_removed': ('if (after != expected) return record;',
                                     'if (false && after != expected) return record;'),
        'uncertainty_fence_removed': ('if (entry.second.state == State::Unknown)',
                                     'if (entry.second.state == State::Refused)'),
    }
    with tempfile.TemporaryDirectory(prefix='dfmcp-dig-engine-') as directory:
        root = Path(directory)
        for name in SOURCES[:3]:
            target = root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)
        def compile_run(name: str, ok: bool = True) -> subprocess.CompletedProcess[str]:
            binary = root / name
            execute([args.compiler, *flags, str(root / SOURCES[2]), '-o', str(binary)])
            return execute([str(binary)], ok=ok)
        result = json.loads(compile_run('engine').stdout)
        for name, expected in reference_vectors().items():
            if bytes.fromhex(result['vectors'][name]) != expected:
                raise AssertionError(f'actual C++ vector differs from independent struct/hashlib reference: {name}')
        killed = []
        original = (ROOT / SOURCES[0]).read_text()
        if args.mutations:
            for name, (old, new) in mutations.items():
                if original.count(old) != 1:
                    raise AssertionError(f'mutation no longer targets exactly one gate: {name}')
                (root / SOURCES[0]).write_text(original.replace(old, new))
                mutant = compile_run(name, ok=False)
                if mutant.returncode != 1 or 'CHECK failed:' not in mutant.stderr:
                    raise AssertionError(f'mutant not rejected by an executed assertion: {name}: {mutant.stderr}')
                killed.append(name)
        report = {'schema': 'dfmcp.bounded-dig-engine-evidence/1', 'status': 'passed_native_engine_only',
                  'compiler': execute([args.compiler, '--version']).stdout.splitlines()[0],
                  'flags': flags, 'groups': result['groups'], 'assertions': result['assertions'],
                  'independent_vectors': len(result['vectors']), 'mutants_rejected': killed,
                  'rust_executed': False, 'dfhack_handler_executed': False, 'live_game_executed': False,
                  'source_sha256': {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in SOURCES}}
        print(json.dumps(report, sort_keys=True, indent=2))


if __name__ == '__main__':
    main()
