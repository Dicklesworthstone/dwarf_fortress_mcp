#!/usr/bin/env python3
"""Compile/run the actual excavation-run plugin against explicit SDK/protobuf doubles.

Independent Python struct/hashlib vectors check canonical C++ output. This is not
real protobuf serialization, SDK ABI, plugin manager, live-game or MCP execution.
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
from test_excavation_run_engine import FLAGS, ROOT

PROFILE = 'bridge/dfhack-excavation-run-v1_18/'
FILES = ('bridge/common/bounded_run.h', 'bridge/common/bounded_run_wire.h',
         'bridge/common/retained_snapshot.h', 'bridge/common/excavation_run.h',
         'bridge/common/excavation_run_wire.h', PROFILE + 'native_capture.h',
         PROFILE + 'dfmcp_excavation_run_v1_18.cpp', PROFILE + 'DfmcpExcavationRunV1_18.proto',
         'tests/native/excavation_run/stubs.h', 'tests/native/excavation_run/handler.cpp')
HEADERS = ('Core.h', 'Export.h', 'PluginManager.h', 'RemoteServer.h', 'VersionInfo.h',
           'TileTypes.h', 'modules/Maps.h', 'modules/World.h', 'df/map_block.h',
           'df/tile_designation.h', 'df/tiletype_shape.h')


def protobuf_double(proto: str) -> str:
    """Generate only the declared accessor/reflection API, never a wire runtime."""
    out = ['#pragma once', '#include "sdk.h"', '#include <algorithm>',
           'namespace dfmcp { namespace excavation_run { namespace v1_18 {',
           'struct Unknown { int count = 0; int field_count() const { return count; } };',
           'struct Reflection { template<class T> const Unknown &GetUnknownFields(const T &v) const { return v.unknown; } };',
           'inline std::size_t varsize(std::uint64_t v) { std::size_t n=1; while(v>=128){v>>=7;++n;} return n; }']
    for name, body in re.findall(r'message (\w+)\s*\{([^}]+)\}', proto):
        fields = re.findall(r'(required|optional) (\w+) (\w+) = (\d+);', body)
        out += ['class ' + name + ' { public:', 'Unknown unknown; std::size_t size_override = 0;',
                'const Reflection *GetReflection() const { static Reflection r; return &r; }',
                'void Clear() { *this = ' + name + '{}; }']
        sizes, required = [], []
        for cardinality, kind, field, number in fields:
            ctype = {'bytes': 'std::string', 'string': 'std::string', 'bool': 'bool',
                     'uint32': 'std::uint32_t', 'uint64': 'std::uint64_t'}[kind]
            string = kind in ('string', 'bytes')
            hook = 'if(fake::throw_reply){fake::throw_reply=false;throw std::bad_alloc();}' if field == 'effect_record' else ''
            out += [f'{ctype} v_{field}{{}}; bool p_{field}=false;',
                    f'bool has_{field}() const {{ return p_{field}; }}',
                    f'{"const std::string &" if string else ctype} {field}() const {{ return v_{field}; }}',
                    f'void set_{field}({"const std::string &" if string else ctype} v) {{ {hook}v_{field}=v; p_{field}=true; }}',
                    f'void clear_{field}() {{ v_{field}={{}}; p_{field}=false; }}']
            value_size = f'varsize(v_{field}.size())+v_{field}.size()' if string else f'varsize(v_{field})'
            sizes.append(f'(p_{field} ? varsize({int(number)*8 + (2 if string else 0)})+{value_size} : 0)')
            if cardinality == 'required':
                required.append('p_' + field)
        out += ['bool IsInitialized() const { return ' + ' && '.join(required) + '; }',
                'std::size_t ByteSizeLong() const { return size_override ? size_override : ' + '+'.join(sizes) + '; }', '};']
    out.append('}}}')
    return '\n'.join(out) + '\n'


def build(root: Path, work: Path, compiler: str) -> subprocess.CompletedProcess:
    work.mkdir(parents=True, exist_ok=True)
    (work / 'sdk.h').write_bytes((root / 'tests/native/excavation_run/stubs.h').read_bytes())
    for header in HEADERS:
        path = work / header; path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text('#include "sdk.h"\n')
    (work / 'DfmcpExcavationRunV1_18.pb.h').write_text(protobuf_double((root / FILES[7]).read_text()))
    subprocess.run([compiler, *FLAGS, '-I', str(work), '-I', str(root), str(root / FILES[-1]),
                    '-o', str(work / 'handler')], check=True, capture_output=True, text=True, timeout=60)
    return subprocess.run([str(work / 'handler')], capture_output=True, text=True, timeout=20)


def reference_vectors(vectors: dict) -> None:
    def digest(domain, data): return hashlib.sha256(domain + b'\0' + data).digest()
    def text(value): return struct.pack('>H', len(value)) + value
    def capture(tick, sequence, paused, cells):
        clock = b'DFMRO013' + struct.pack('>QQQBBB', 41, sequence, tick, 1, 1, paused)
        return (b'DFMEC018' + clock + struct.pack('>IIII', 2, 64, 64, 8) + text(b'region1')
                + struct.pack('>IIIIIH', 15, 15, 2, 2, 2, 4) + cells * 4)
    before = capture(806500, 3, 1, bytes([2, 2, 0, 1]))
    after = capture(806504, 4, 0, bytes([2, 3, 0, 0]))
    spec = struct.pack('>IIIIII', 100, 1000, 2, 2, 1, 10)
    plan = digest(b'dfmcp-excavation-run-plan/1', spec + before)
    key = text(b'golden'); token = digest(b'dfmcp-excavation-run-token/1', key + plan)[:16]
    prefix = b'DFMER018' + key + spec + text(before) + plan + token
    prepared = prefix + bytes(5) + struct.pack('>QBIQQQB', 0, 0, 0, 0, 806500, 806500, 0)
    stopped = prefix + bytes([3, 3, 1, 1, 1]) + struct.pack('>QBIQQQB', 806504, 1, 2, 806501, 806504, 806504, 1) + text(after)
    for name, value in {'capture': before, 'plan': plan, 'token': token,
                        'prepared': prepared + digest(b'dfmcp-excavation-run-receipt/1', prepared),
                        'stopped': stopped + digest(b'dfmcp-excavation-run-receipt/1', stopped)}.items():
        if vectors[name] != value.hex():
            raise AssertionError('independent native vector mismatch: ' + name)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', default='g++')
    parser.add_argument('--mutations', action='store_true')
    parser.add_argument('--report', type=Path)
    args = parser.parse_args(); compiler = shutil.which(args.compiler)
    if compiler is None: parser.error('requested compiler unavailable')
    with tempfile.TemporaryDirectory(prefix='dfmcp-excavation-bridge-') as directory:
        work = Path(directory); result = build(ROOT, work / 'normal', compiler)
        if result.returncode: raise RuntimeError(result.stderr or result.stdout)
        output = json.loads(result.stdout); reference_vectors(output.pop('vectors'))
        report = {**output, 'independent_vectors': 5, 'real_dfhack_sdk': False, 'real_protobuf': False,
                  'live_fortress': False, 'rust_mcp': False,
                  'compiler': subprocess.check_output([compiler, '--version'], text=True).splitlines()[0],
                  'flags': FLAGS, 'source_sha256': {f: hashlib.sha256((ROOT/f).read_bytes()).hexdigest() for f in
                      (*FILES, 'scripts/test_excavation_run_bridge.py', 'scripts/test_excavation_run_engine.py')},
                  'rejected_mutants': []}
        if args.mutations:
            mutants = {
                'hidden_payload_read': (PROFILE+'native_capture.h', 'if (bits.hidden) { cell.presence = 1; continue; }', 'if (false) { cell.presence = 1; continue; }'),
                'unknown_fields_accepted': (PROFILE+'dfmcp_excavation_run_v1_18.cpp', '&& in.GetReflection()->GetUnknownFields(in).field_count() == 0', ''),
                'clock_gate_removed': (PROFILE+'dfmcp_excavation_run_v1_18.cpp', 'return opted_in() && value && std::string_view(value) == "1";', '(void)value; return opted_in();'),
                'replacement_setter': (PROFILE+'native_capture.h', 'run::require(identity(generation) == expected, 4);', '(void)generation; (void)expected;'),
            }
            for name, (path, old, new) in mutants.items():
                root = work / name
                for f in FILES:
                    destination = root/f; destination.parent.mkdir(parents=True, exist_ok=True)
                    destination.write_bytes((ROOT/f).read_bytes())
                original = (root/path).read_text()
                if original.count(old) != 1: raise RuntimeError('mutation target changed: '+name)
                (root/path).write_text(original.replace(old, new))
                result = build(root, work/(name+'-build'), compiler)
                if result.returncode == 0: raise RuntimeError('mutant survived: '+name)
                report['rejected_mutants'].append(name)
    rendered = json.dumps(report, indent=2, sort_keys=True)+'\n'
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True); args.report.write_text(rendered)
    print(rendered, end='')


if __name__ == '__main__':
    main()
