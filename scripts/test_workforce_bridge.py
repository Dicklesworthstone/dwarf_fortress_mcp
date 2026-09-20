#!/usr/bin/env python3
"""Build the real workforce plugin translation unit against explicit API doubles."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parents[1]
PLUGIN='bridge/dfhack-workforce-v1_17/dfmcp_workforce_v1_17.cpp'
FILES=('bridge/common/retained_snapshot.h','bridge/common/workforce_control.h',PLUGIN,
       'bridge/dfhack-workforce-v1_17/DfmcpWorkforceV1_17.proto',
       'tests/native/workforce/stubs.h','tests/native/workforce/handler.cpp')

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--compiler',action='append');p.add_argument('--mutations',action='store_true');args=p.parse_args()
    compilers=args.compiler or [name for name in ('g++','clang++') if shutil.which(name)]
    if not compilers:p.error('C++ compiler required')
    build=Path(tempfile.mkdtemp(prefix='dfmcp-workforce-bridge-'));stub=build/'include';stub.mkdir()
    schema=(ROOT/FILES[3]).read_text()
    request=schema.split('message Request {',1)[1].split('}',1)[0]
    actual=re.findall(r'(required|optional|repeated)\s+(\w+)\s+(\w+)\s*=\s*(\d+)',request)
    expected=[('required','bytes','bearer_token','1'),('required','bytes','client_nonce','2'),
              ('required','uint32','protocol_major','3'),('required','uint32','protocol_minor','4'),
              ('optional','string','idempotency_key','5'),('optional','uint32','detail_index','6'),
              ('optional','bool','assigned','7'),('optional','bytes','expected_witness','8'),
              ('optional','bytes','plan_digest','9'),('optional','bytes','prepare_token','10'),
              ('repeated','uint32','unit_ids','11')]
    if actual!=expected:raise AssertionError('protobuf/API-double request field drift')
    includes=re.findall(r'#include "([^"\n]+)"',(ROOT/PLUGIN).read_text())
    for inc in includes:
        if inc.startswith('../'):continue
        target=stub/inc;target.parent.mkdir(parents=True,exist_ok=True)
        target.write_text('#pragma once\n#include "'+str(ROOT/'tests/native/workforce/stubs.h')+'"\n')
    def execute(compiler,root,name):
        binary=build/name
        cmd=[compiler,'-std=c++17','-Wall','-Wextra','-Werror','-pedantic','-fsanitize=undefined',
             '-fno-sanitize-recover=all','-UNDEBUG','-I',str(stub),'-I',str(root),str(root/'tests/native/workforce/handler.cpp'),'-o',str(binary)]
        done=subprocess.run(cmd,text=True,capture_output=True,timeout=90)
        if done.returncode:raise RuntimeError(done.stderr)
        return subprocess.run([str(binary)],text=True,capture_output=True,timeout=15)
    report={'scope':'actual plugin with SDK/protobuf API doubles; not real SDK or live game',
            'sources':{f:hashlib.sha256((ROOT/f).read_bytes()).hexdigest() for f in FILES},'runs':[]}
    mutations={
        'missing-recompute':('for (auto *u : targets) Units::setAutomaticProfessions(u);','(void)targets;'),
        'wrong-detail':('detail->assigned_units.swap(replacement);','(void)detail;'),
        'stale-ordering':('current.encode() == before.encode()','current.folder == before.folder'),
    }
    for i,compiler in enumerate(compilers):
        result=execute(compiler,ROOT,f'handler-{i}');result.check_returncode();run={'compiler':compiler,'result':result.stdout.strip(),'rejected_mutants':[]}
        if args.mutations:
            for name,(old,new) in mutations.items():
                root=build/f'source-{i}-{name}'
                for f in FILES:
                    target=root/f;target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/f,target)
                path=root/PLUGIN;content=path.read_text()
                if content.count(old)!=1:raise AssertionError('mutation site drift')
                content=content.replace(old,new);path.write_text(content)
                if name=='stale-ordering':
                    path=root/'bridge/common/workforce_control.h';s=path.read_text();s=s.replace('valid = observe(r.before.ids(), read).encode() == r.before.encode();','valid = observe(r.before.ids(), read).folder == r.before.folder;');path.write_text(s)
                result=execute(compiler,root,f'mutant-{i}-{name}')
                if not result.returncode:raise AssertionError(f'mutant escaped {name}')
                run['rejected_mutants'].append(name)
        report['runs'].append(run);print(json.dumps(run,sort_keys=True))
    (build/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(f'Retained artifacts: {build}')
if __name__=='__main__':main()
