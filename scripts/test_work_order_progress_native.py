#!/usr/bin/env python3
"""Compile the real read-only handler against explicit SDK/protobuf test doubles."""
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
HEADERS = ['Core.h', 'Export.h', 'MiscUtils.h', 'PluginManager.h', 'RemoteServer.h',
           'VersionInfo.h', 'modules/World.h', 'df/global_objects.h', 'df/item_type.h',
           'df/job_type.h', 'df/manager_order.h', 'df/workquota_frequency_type.h', 'df/world.h',
           'DfmcpWorkOrderProgressV1_12.pb.h']


def text(value: bytes) -> bytes:
    return struct.pack('>H', len(value)) + value


def vector() -> bytes:
    header = b'DFMWP012' + struct.pack('>QQQIIIB', 7, 1, 12345, 1, 10, 1, 1)
    header += text(b'region1') + struct.pack('>I', 2)
    row = struct.pack('>IBiBiiIiiiiiII', 3, 1, 69, 1, 3, 5, 3, 0, -1, 1, -1, -1, 0, 0)
    return header + row + text(b'ConstructBed') + text(b'') + struct.pack('>IB', 8, 0)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument('--compiler', default='g++')
    parser.add_argument('--ubsan', action='store_true')
    parser.add_argument('--mutations', action='store_true')
    args = parser.parse_args()
    compiler = shutil.which(args.compiler)
    if compiler is None:
        raise SystemExit('C++ compiler unavailable')
    with tempfile.TemporaryDirectory(prefix='dfmcp-progress-') as tmp:
        includes = Path(tmp) / 'include'
        for header in HEADERS:
            target = includes / header
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text('#pragma once\n#include "stubs.h"\n', encoding='utf-8')
        binary = Path(tmp) / 'handler'
        command = [compiler, '-std=c++17', '-O1', '-Wall', '-Wextra', '-Werror', '-pedantic',
                   '-I', str(includes), '-I', str(ROOT / 'tests/native/work_order_progress')]
        if args.ubsan:
            command += ['-fsanitize=undefined', '-fno-sanitize-recover=all']
        command += [str(ROOT / 'tests/native/work_order_progress/handler.cpp'), '-o', str(binary)]
        subprocess.run(command, check=True, capture_output=True, text=True, timeout=90)
        output = subprocess.run([str(binary)], check=True, capture_output=True, text=True, timeout=20).stdout
        rejected_mutants = []
        if args.mutations:
            relative = 'bridge/dfhack-work-order-progress-v1_12/dfmcp_work_order_progress_v1_12.cpp'
            original = (ROOT / relative).read_text()
            changes = {
                'complete_queue_uniqueness': ('wp::require(index.emplace(static_cast<std::uint32_t>(order->id), order).second, 5);',
                                              'index.emplace(static_cast<std::uint32_t>(order->id), order);'),
                'static_material_recognition': ('a.material_category.whole == wood.whole', '(static_cast<void>(wood), true)'),
                'unknown_request_field_gate': ('in->IsInitialized() && in->GetReflection()->GetUnknownFields(*in).empty()', 'in->IsInitialized()'),
            }
            for name, (old, new) in changes.items():
                if original.count(old) != 1:
                    raise AssertionError('mutation no longer matches exactly one source location')
                overlay = Path(tmp) / name
                for source in ['bridge/common/work_order_progress.h', relative,
                               'tests/native/work_order_progress/handler.cpp', 'tests/native/work_order_progress/stubs.h']:
                    target = overlay / source
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes((ROOT / source).read_bytes())
                (overlay / relative).write_text(original.replace(old, new), encoding='utf-8')
                mutant = overlay / 'handler'
                build = [str(overlay / 'tests/native/work_order_progress') if v == str(ROOT / 'tests/native/work_order_progress') else v for v in command[:-3]]
                build += [str(overlay / 'tests/native/work_order_progress/handler.cpp'), '-o', str(mutant)]
                subprocess.run(build, check=True, capture_output=True, text=True, timeout=90)
                result = subprocess.run([str(mutant)], capture_output=True, text=True, timeout=20)
                if result.returncode == 0 or 'failed check' not in result.stderr:
                    raise AssertionError(f'mutant {name} was not rejected by an assertion: {result.stderr}')
                rejected_mutants.append(name)
    emitted = re.search(r'^VECTOR ([0-9a-f]+)$', output, re.M)
    summary = re.search(r'^PASS (\d+) assertions (\d+) groups$', output, re.M)
    if emitted is None or summary is None or bytes.fromhex(emitted[1]) != vector():
        raise AssertionError('native fixture or assertion report mismatch')
    files = ['bridge/common/work_order_progress.h',
             'bridge/dfhack-work-order-progress-v1_12/dfmcp_work_order_progress_v1_12.cpp',
             'bridge/dfhack-work-order-progress-v1_12/DfmcpWorkOrderProgressV1_12.proto',
             'tests/native/work_order_progress/handler.cpp', 'tests/native/work_order_progress/stubs.h',
             'scripts/test_work_order_progress_native.py']
    print(json.dumps({'schema': 'dfmcp.work-order-progress-native/1', 'status': 'passed_sdk_doubles_only',
                     'assertions': int(summary[1]), 'groups': int(summary[2]), 'ubsan': args.ubsan,
                     'separately_compiled_mutants_rejected': rejected_mutants,
                     'compiler': subprocess.run([compiler, '--version'], capture_output=True, text=True, check=True).stdout.splitlines()[0],
                     'native_vector_hex': emitted[1], 'native_vector_sha256': hashlib.sha256(vector()).hexdigest(),
                     'real_dfhack_or_protobuf_or_game_executed': False,
                     'source_sha256': {f: hashlib.sha256((ROOT / f).read_bytes()).hexdigest() for f in files}}, indent=2, sort_keys=True))


if __name__ == '__main__':
    main()
