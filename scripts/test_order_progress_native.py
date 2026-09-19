#!/usr/bin/env python3
"""Compile the actual progress handler with explicit SDK/protobuf doubles."""
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
SOURCE = Path('bridge/dfhack-order-progress-v1_11/dfmcp_order_progress_v1_11.cpp')
HEADER = Path('bridge/common/order_progress.h')
HARNESS = Path('tests/native/order_progress/handler.cpp')
STUB = Path('tests/native/order_progress/stubs.h')


def reference_vectors() -> dict[str, bytes]:
    def record(sequence: int, tick: int, present: bool, left: int, status: int) -> bytes:
        folder = b'region1'
        raw = b'DFMOP011' + struct.pack('>QQQIIIBBH', 7, sequence, tick, 10, 11, 1, 1, present, len(folder)) + folder
        if present:
            raw += struct.pack('>iIIIiB', 69, left, 5, status, 0, 1)
        return raw
    return {name: record(seq,tick,present,left,status) for name,seq,tick,present,left,status in (
        ('pending',1,12345,True,5,0), ('active',2,12346,True,2,3),
        ('zero',3,12347,True,0,3), ('missing',4,12348,False,0,0))}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', choices=('g++','clang++'), default='g++')
    parser.add_argument('--mutations', action='store_true')
    args = parser.parse_args()
    compiler = shutil.which(args.compiler)
    if compiler is None:
        raise SystemExit('requested compiler not installed')
    source_text = (ROOT / SOURCE).read_text()
    mutations = {
        'missing-duplicate-check': ('op::require(std::adjacent_find(ids.begin(),ids.end()) == ids.end(),5);', ''),
        'missing-condition-check': ('|| !a.item_conditions.empty() || !a.order_conditions.empty() || a.items', ''),
        'wrong-remaining-counter': ('out.left = static_cast<std::uint32_t>(found->amount_left);', 'out.left = static_cast<std::uint32_t>(found->amount_total);'),
    }
    sources = [SOURCE,HEADER,HARNESS,STUB,Path('tests/native/work_orders/stubs.h'),Path('scripts/test_order_progress_native.py'),
               Path('bridge/dfhack-order-progress-v1_11/DfmcpOrderProgressV1_11.proto')]
    report = {'schema':'dfmcp.order-progress-native-tests/1', 'scope':'actual native handler with explicit SDK/protobuf doubles',
              'real_dfhack_or_generated_protobuf':False, 'rust_or_mcp_executed':False, 'live_game_executed':False,
              'source_sha256':{str(p):hashlib.sha256((ROOT/p).read_bytes()).hexdigest() for p in sources}}
    with tempfile.TemporaryDirectory(prefix='dfmcp-order-progress-') as temp:
        build = Path(temp); includes = build / 'include'; includes.mkdir()
        for line in source_text.splitlines():
            if line.startswith('#include "'):
                name = line.split('"')[1]
                if name.startswith('../common/'): continue
                p = includes/name; p.parent.mkdir(parents=True,exist_ok=True); p.write_text('#pragma once\n')
        def run(text: str, name: str) -> subprocess.CompletedProcess[str]:
            tree = build/name; cpp = tree/SOURCE; cpp.parent.mkdir(parents=True)
            header = tree/HEADER; header.parent.mkdir(parents=True,exist_ok=True); shutil.copyfile(ROOT/HEADER,header)
            cpp.write_text(text)
            wrapper = tree/'handler.cpp'
            wrapper.write_text((ROOT/HARNESS).read_text().replace('"stubs.h"', json.dumps(str(ROOT/STUB)))
                .replace('"../../../'+str(SOURCE)+'"',json.dumps(str(cpp))))
            binary = tree/'handler'
            command = [compiler,'-std=c++17','-Wall','-Wextra','-Werror','-pedantic','-O1',
                       '-fsanitize=undefined','-fno-sanitize-recover=all','-I',str(includes),str(wrapper),'-o',str(binary)]
            built = subprocess.run(command,capture_output=True,text=True,timeout=60)
            if built.returncode:
                raise RuntimeError('native test compilation failed\n'+built.stderr)
            return subprocess.run([str(binary)],capture_output=True,text=True,timeout=20)
        actual = run(source_text,'baseline')
        if actual.returncode: raise RuntimeError('baseline failed: '+actual.stderr)
        values = dict(line.split('=',1) for line in actual.stdout.splitlines())
        for name,raw in reference_vectors().items():
            if bytes.fromhex(values[name]) != raw: raise AssertionError('native/Python mismatch: '+name)
            fixture = ROOT/'crates/dfmcp-adapter/tests/fixtures'/f'order_progress_{name}_v1_11.hex'
            if bytes.fromhex(fixture.read_text()) != raw: raise AssertionError('fixture mismatch: '+name)
        report.update(status='passed_with_test_doubles', assertions=int(values['assertions']),groups=int(values['groups']),
                      compiler=subprocess.check_output([compiler,'--version'],text=True).splitlines()[0],
                      independent_native_vectors=4, mutation_tests=[])
        if args.mutations:
            for name,(before,after) in mutations.items():
                if source_text.count(before) != 1: raise AssertionError('ambiguous mutation target')
                result = run(source_text.replace(before,after),name)
                if result.returncode == 0: raise AssertionError('mutant survived: '+name)
                if 'assertion ' not in result.stderr: raise AssertionError('mutant did not fail an assertion: '+result.stderr)
                report['mutation_tests'].append({'name':name,'rejected':True})
    print(json.dumps(report,indent=2,sort_keys=True))


if __name__ == '__main__':
    main()
