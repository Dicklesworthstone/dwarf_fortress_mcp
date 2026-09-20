#!/usr/bin/env python3
"""Execute the actual run/1.13 plugin with explicit SDK/protobuf API doubles.

This is NOT an actual DFHack SDK/protobuf ABI build or live-game qualification.
The schema generates the API double; independent Python verifies canonical hashes.
Build artifacts and all reports are retained; no repository files are removed.
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
INPUTS = ["bridge/common/bounded_run.h", "bridge/common/bounded_run_wire.h",
          "bridge/common/retained_snapshot.h", "bridge/dfhack-run-v1_13/dfmcp_run_v1_13.cpp",
          "bridge/dfhack-run-v1_13/DfmcpRunV1_13.proto", "tests/native/bounded_run/bridge.cpp",
          "scripts/test_bounded_run_bridge.py"]


def generate_doubles(build: Path) -> None:
    source = (ROOT / INPUTS[4]).read_text()
    package = re.search(r"package ([\w.]+);", source)
    if package is None:
        raise ValueError("protobuf package missing")
    result = ['#pragma once', '#include <cstdint>', '#include <string>', '#include <new>',
              'namespace fake_proto { inline bool fail_effect = false; }',
              'namespace ' + package[1].replace('.', '::') + ' {',
              'struct Unknown { int count; int field_count() const { return count; } };',
              'struct Reflection { template<class T> Unknown GetUnknownFields(const T &v) const { return {v.unknown_count}; } };']
    for name, body in re.findall(r"message (\w+)\s*\{([^}]+)\}", source):
        fields = re.findall(r"(required|optional) (\w+) (\w+) = (\d+);", body)
        result += [f'class {name} {{ public:', 'int unknown_count = 0;',
                   f'void Clear() {{ *this = {name}(); }}',
                   'const Reflection *GetReflection() const { static Reflection r; return &r; }']
        required, sizes = [], []
        for necessity, typ, field, _ in fields:
            native = {'bytes': 'std::string', 'string': 'std::string', 'uint32': 'std::uint32_t',
                      'uint64': 'std::uint64_t', 'bool': 'bool'}[typ]
            argument = 'const std::string &' if native == 'std::string' else native
            special = ('if (fake_proto::fail_effect) { fake_proto::fail_effect = false; throw std::bad_alloc(); } '
                       if field == 'effect_record' else '')
            result += [f'private: {native} {field}_{{}}; bool has_{field}_ = false;', 'public:',
                       f'bool has_{field}() const {{ return has_{field}_; }}',
                       f'{argument} {field}() const {{ return {field}_; }}',
                       f'void set_{field}({argument} value) {{ {special}{field}_ = value; has_{field}_ = true; }}',
                       f'void clear_{field}() {{ {field}_ = {{}}; has_{field}_ = false; }}']
            if necessity == 'required':
                required.append(f'has_{field}_')
            sizes.append(f'(has_{field}_ ? {field}_.size() + 4 : 0)' if native == 'std::string'
                         else f'(has_{field}_ ? 11 : 0)')
        result += ['bool IsInitialized() const { return ' + ' && '.join(required) + '; }',
                   'std::size_t ByteSizeLong() const { return ' + ' + '.join(sizes) + '; }', '};']
    result += ['}']
    (build / 'DfmcpRunV1_13.pb.h').write_text('\n'.join(result) + '\n')
    sdk = r'''#pragma once
#include <cstdint>
#include <functional>
#include <stdexcept>
#include <string>
#include <vector>
namespace fake {
inline int depth = 0, year = 2, pauses = 0, unpauses = 0;
inline std::uint32_t tick = 100;
inline bool world_loaded = true, map_loaded = true, fortress = true, paused = true;
inline bool fail_pause = false, fail_after_unpause = false, ignore_unpause = false;
inline std::function<void()> on_unpause;
inline void suspended() { if (depth <= 0) throw std::logic_error("native access without suspension"); }
}
#define DFHACK_PLUGIN(name) static_assert(true, name)
#define DFhackCExport extern "C"
namespace DFHack {
enum command_result { CR_OK = 0, CR_FAILURE = 1 };
enum state_change_event { SC_WORLD_LOADED, SC_WORLD_UNLOADED, SC_MAP_LOADED, SC_MAP_UNLOADED, SC_BEGIN_UNLOAD };
struct color_ostream {};
struct PluginCommand {};
struct VersionInfo { std::string getVersion() const { return "fake-df"; } };
struct Core {
    VersionInfo info; VersionInfo *vinfo = &info;
    static Core &getInstance() { static Core c; return c; }
    bool isWorldLoaded() const { fake::suspended(); return fake::world_loaded; }
    bool isMapLoaded() const { fake::suspended(); return fake::map_loaded; }
};
struct CoreSuspender { CoreSuspender() { ++fake::depth; } ~CoreSuspender() { --fake::depth; } };
namespace Version { inline const char *dfhack_version() { return "fake-dfhack"; } }
namespace World {
inline bool isFortressMode() { fake::suspended(); return fake::fortress; }
inline bool ReadPauseState() { fake::suspended(); return fake::paused; }
inline std::int32_t ReadCurrentYear() { fake::suspended(); return fake::year; }
inline std::uint32_t ReadCurrentTick() { fake::suspended(); return fake::tick; }
inline void SetPauseState(bool pause) {
    fake::suspended();
    if (pause) {
        ++fake::pauses;
        if (fake::fail_pause) throw std::runtime_error("pause failed");
        fake::paused = true;
    } else {
        ++fake::unpauses;
        if (fake::on_unpause) fake::on_unpause();
        if (!fake::ignore_unpause) fake::paused = false;
        if (fake::fail_after_unpause) throw std::runtime_error("unpause acknowledgement lost");
    }
}
}
struct RPCService {
    std::vector<std::string> names;
    template<class F> void addFunction(const char *name, F, int flags) {
        if (flags != 0) throw std::logic_error("unsuspended RPC registration");
        names.emplace_back(name);
    }
};
}
'''
    (build / 'sdk_double.h').write_text(sdk)
    for name in ['Core.h', 'Export.h', 'PluginManager.h', 'RemoteServer.h', 'VersionInfo.h', 'modules/World.h']:
        path = build / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text('#include "sdk_double.h"\n')


def check_vectors(output: str) -> int:
    values = dict(line.split('=', 1) for line in output.splitlines() if '=' in line)
    snapshot = b'DFMRO013' + struct.pack('>QQQBBB', 41, 3, 806500, 1, 1, 1)
    spec = struct.pack('>II', 10, 1000)
    plan = hashlib.sha256(b'dfmcp-bounded-run-plan/1\0' + spec + snapshot).digest()
    key = struct.pack('>H', 6) + b'golden'
    token = hashlib.sha256(b'dfmcp-bounded-run-token/1\0' + key + plan).digest()[:16]
    record = b'DFMRE013' + key + spec + snapshot + plan + token + bytes(5) + bytes(8)
    record += hashlib.sha256(b'dfmcp-bounded-run-receipt/1\0' + record).digest()
    expected = {'snapshot': snapshot.hex(), 'plan': plan.hex(), 'record': record.hex()}
    for name, value in expected.items():
        if values.get(name) != value:
            raise ValueError(f'C++/Python canonical {name} mismatch')
    return int(values['bridge_assertions'])


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', action='append')
    parser.add_argument('--build-dir', type=Path)
    args = parser.parse_args()
    build = args.build_dir or Path(tempfile.mkdtemp(prefix='dfmcp-run-bridge-'))
    build.mkdir(parents=True, exist_ok=True)
    generate_doubles(build)
    compilers = args.compiler or [name for name in ('g++', 'clang++') if shutil.which(name)]
    if not compilers:
        parser.error('no C++ compiler found')
    report = {'evidence_class': 'native_plugin_sdk_and_protobuf_api_doubles', 'live_game_qualified': False,
              'source_sha256': {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in INPUTS},
              'runs': []}
    for index, compiler in enumerate(compilers):
        binary = build / f'bridge-{index}'
        subprocess.run([compiler, '-std=c++17', '-Wall', '-Wextra', '-Werror', '-pedantic',
                        '-fsanitize=undefined', '-fno-sanitize-recover=all', '-g', '-I', str(ROOT),
                        '-I', str(build), str(ROOT / 'tests/native/bounded_run/bridge.cpp'), '-o', str(binary)],
                       check=True, timeout=90)
        process = subprocess.run([str(binary)], text=True, capture_output=True, check=True, timeout=15)
        count = check_vectors(process.stdout)
        (build / f'bridge-{index}.log').write_text(process.stdout + process.stderr)
        report['runs'].append({'compiler': compiler, 'assertions': count, 'independent_hash_vectors': 3})
        print(f'{compiler}: {count} plugin assertions and 3 independent canonical hash vectors passed')
    (build / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(f'Retained SDK-double artifacts: {build}')


if __name__ == '__main__':
    main()
