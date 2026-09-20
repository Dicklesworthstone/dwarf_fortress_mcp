#!/usr/bin/env python3
"""Compile actual order-run/1.14 plugin code against explicit SDK/protobuf doubles.

No real SDK/ABI, protobuf runtime, plugin-manager or game qualification. All
build products/logs are retained. Independent Python checks the C++ wire bytes.
"""
from pathlib import Path
import argparse
import hashlib
import re
import shutil
import struct
import subprocess
import tempfile


def protobuf_double(schema):
    lines = ['#pragma once', '#include <string>', '#include <cstdint>', '#include <new>',
             'namespace dfmcp::order_run::v1_14 {']
    for name, body in re.findall(r'message (\w+)\s*\{([^}]+)\}', schema):
        fields = re.findall(r'(required|optional) (\w+) (\w+) = (\d+);', body)
        lines += [f'struct {name} {{', 'inline static int fail_record = 0;',
                  'bool unknown = false; std::size_t forced_size = 0;']
        for _, kind, field, _ in fields:
            cpp = {'bytes': 'std::string', 'string': 'std::string', 'uint32': 'std::uint32_t',
                   'uint64': 'std::uint64_t', 'bool': 'bool'}[kind]
            lines += [f'{cpp} v_{field}{{}}; bool p_{field} = false;',
                      f'const {cpp} &{field}() const {{ return v_{field}; }}',
                      f'bool has_{field}() const {{ return p_{field}; }}']
            fault = 'if (fail_record) { --fail_record; throw std::bad_alloc(); }' if field == 'effect_record' else ''
            lines += [f'void set_{field}(const {cpp} &v) {{ {fault} v_{field}=v; p_{field}=true; }}']
        initialized = ' && '.join(f'p_{f}' for mode, _, f, _ in fields if mode == 'required')
        lines += [f'bool IsInitialized() const {{ return {initialized}; }}',
                  'std::size_t ByteSizeLong() const { return forced_size ? forced_size : 256' +
                  ''.join(f' + v_{f}.size()' for _, k, f, _ in fields if k in ('bytes', 'string')) + '; }',
                  f'void Clear() {{ *this = {name}{{}}; }}',
                  'struct Unknown { int count; int field_count() const { return count; } };',
                  f'struct Reflection {{ Unknown GetUnknownFields(const {name} &v) const {{ return {{v.unknown ? 1 : 0}}; }} }};',
                  'const Reflection *GetReflection() const { static Reflection r; return &r; }', '};']
    return '\n'.join(lines + ['}'])


def vectors(output):
    values = dict(line.split('=', 1) for line in output.splitlines() if '=' in line)
    observation = (b'DFMOR014' + struct.pack('>QQQIH', 41, 0, 100, 7, 4) + b'fort'
                   + struct.pack('>B I I B B i i I', 1, 9, 10, 1, 1, 10, 10, 0))
    plan = b'DFMOP014' + struct.pack('>IIBIIIH', 100, 1000, 1, 0, 1, 1, len(observation)) + observation
    digest = hashlib.sha256(b'dfmcp-order-run-plan/1\0' + plan).digest()
    key = struct.pack('>H', 6) + b'vector'
    token = hashlib.sha256(b'dfmcp-order-run-token/1\0' + key + digest).digest()[:16]
    record = (b'DFMOE014' + key + struct.pack('>H', len(plan)) + plan + digest + token + bytes(6)
              + struct.pack('>QIQH', 0, 0, 100, 0))
    record += hashlib.sha256(b'dfmcp-order-run-receipt/1\0' + record).digest()
    for label, expected in [('capture', observation), ('plan', plan), ('record', record)]:
        if bytes.fromhex(values[label]) != expected:
            raise AssertionError(f'cross-language {label} differs')
    return values


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', action='append')
    parser.add_argument('--build-dir', type=Path)
    parser.add_argument('--mutations', action='store_true', help='compile three separately mutated native variants')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    build = args.build_dir or Path(tempfile.mkdtemp(prefix='dfmcp-order-run-bridge-'))
    build.mkdir(parents=True, exist_ok=True)
    stubs = root / 'tests/native/order_run/stubs.h'
    for name in ('Core.h', 'Export.h', 'PluginManager.h', 'RemoteServer.h', 'VersionInfo.h', 'modules/World.h',
                 'df/global_objects.h', 'df/item_type.h', 'df/job_type.h', 'df/manager_order.h', 'df/workquota_frequency_type.h', 'df/world.h'):
        dest = build / name; dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(f'#include "{stubs}"\n')
    schema = root / 'bridge/dfhack-order-run-v1_14/DfmcpOrderRunV1_14.proto'
    (build / 'DfmcpOrderRunV1_14.pb.h').write_text(protobuf_double(schema.read_text()))
    compilers = args.compiler or [c for c in ('g++', 'clang++') if shutil.which(c)]
    if not compilers:
        parser.error('C++17 compiler required')
    for index, compiler in enumerate(compilers):
        binary = build / f'bridge-{index}'
        command = [compiler, '-std=c++17', '-Wall', '-Wextra', '-Werror', '-pedantic', '-fsanitize=undefined',
                   '-fno-sanitize-recover=all', '-g', '-I', str(root), '-I', str(build),
                   str(root / 'tests/native/order_run/bridge.cpp'), '-o', str(binary)]
        subprocess.run(command, check=True, timeout=90)
        result = subprocess.run([str(binary)], capture_output=True, text=True, timeout=20)
        (build / f'bridge-{index}.log').write_text(result.stdout + result.stderr)
        if result.returncode:
            raise RuntimeError(result.stderr)
        vectors(result.stdout)
        print(f'{compiler}: {result.stdout.splitlines()[-1]}; 3 independent wire vectors passed')
    if args.mutations:
        mutations = [
            ('setter-scope', 'bridge/dfhack-order-run-v1_14/native_capture.h',
             'run::require(identity(generation) == expected, 4);',
             'run::require(identity(generation).generation == expected.generation, 4);'),
            ('template-recognition', 'bridge/dfhack-order-run-v1_14/native_capture.h',
             'return matches ? code : 0;', 'return (matches || a.amount_total > 0) ? code : 0;'),
            ('same-tick-samples', 'bridge/common/order_run.h',
             'current.clock.tick >= record.counted_tick + record.goal.interval',
             'current.clock.tick >= record.counted_tick'),
        ]
        sources = ['bridge/common/order_run.h', 'bridge/common/bounded_run.h',
                   'bridge/common/order_run_wire.h', 'bridge/common/retained_snapshot.h',
                   'bridge/dfhack-order-run-v1_14/native_capture.h',
                   'bridge/dfhack-order-run-v1_14/dfmcp_order_run_v1_14.cpp']
        for label, changed, old, new in mutations:
            mutant = build / label
            for path in sources:
                target = mutant / path; target.parent.mkdir(parents=True, exist_ok=True)
                contents = (root / path).read_text()
                if path == changed:
                    if contents.count(old) != 1:
                        raise AssertionError('mutation target does not occur exactly once')
                    contents = contents.replace(old, new)
                target.write_text(contents)
            for index, compiler in enumerate(compilers):
                binary = mutant / f'bridge-{index}'
                subprocess.run([compiler, '-std=c++17', '-Wall', '-Wextra', '-Werror', '-pedantic',
                                '-fsanitize=undefined', '-fno-sanitize-recover=all', '-I', str(mutant), '-I', str(build),
                                str(root / 'tests/native/order_run/bridge.cpp'), '-o', str(binary)], check=True, timeout=90)
                result = subprocess.run([str(binary)], capture_output=True, text=True, timeout=20)
                (mutant / f'bridge-{index}.log').write_text(result.stdout + result.stderr)
                if result.returncode == 0:
                    raise AssertionError(f'{label}: mutant survived')
                print(f'{compiler}: {label} rejected by executable assertions')
    print(f'Retained artifacts: {build}')


if __name__ == '__main__':
    main()
