#!/usr/bin/env python3
"""Execute real workforce C++ sources with doubles, not a DFHack SDK/live claim.

Build products and deliberately weakened copies are retained outside the checkout.
"""
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
SOURCES = ('bridge/common/retained_snapshot.h', 'bridge/common/workforce_control.h',
           'tests/native/workforce/engine.cpp')

def field(raw: bytes) -> bytes:
    return struct.pack('>H', len(raw)) + raw

def hashed(domain: bytes, raw: bytes) -> bytes:
    return hashlib.sha256(domain + b'\0' + raw).digest()

def reference_vectors() -> dict[str, str]:
    keys = (b'MINE', b'CARPENTER', b'HAUL')
    def capture(after=False):
        out = b'DFMWF017' + struct.pack('>QQQI', 42, int(after), 100, 7) + field(b'region1') + b'\1\1'
        out += struct.pack('>H', 3) + b''.join(field(k) for k in keys) + struct.pack('>H', 2)
        for name, labors, members in ((b'Miners', b'\1\0\0', [2,5] if after else [5]),
                                      (b'Carpenters', b'\0\1\0', [2])):
            out += field(name) + struct.pack('>IB', 2, 1) + labors + struct.pack('>H', len(members))
            out += b''.join(struct.pack('>I', m) for m in members)
        return out + struct.pack('>H', 2) + units(after)
    def units(after):
        return (struct.pack('>IIB',2,102,1) + (b'\1\1\1' if after else b'\0\1\1')
                + struct.pack('>IIB',5,105,1) + b'\1\0\1')
    witness = hashlib.sha256(capture()).digest()
    spec = struct.pack('>IB',0,1)
    plan = hashed(b'dfmcp-workforce-plan/1', spec+witness)
    token = hashed(b'dfmcp-workforce-token/1',field(b'assign')+plan)[:16]
    def record(after=False):
        out = b'DFMWE017'+field(b'assign')+plan+token+spec+witness+struct.pack('>QQQB',42,0,100,2 if after else 0)
        out += hashlib.sha256(capture(True)).digest() if after else bytes(32)
        out += struct.pack('>HH',3,2 if after else 0)
        if after: out += units(True)
        return out+hashed(b'dfmcp-workforce-receipt/1',out)
    return {'capture': capture().hex(), 'prepared': record().hex(), 'applied': record(True).hex()}

def build_run(compiler: str, root: Path, build: Path, unit='engine') -> subprocess.CompletedProcess:
    binary = build / unit
    cmd = [compiler,'-std=c++17','-Wall','-Wextra','-Werror','-pedantic','-fsanitize=undefined',
           '-fno-sanitize-recover=all','-UNDEBUG','-I',str(root),str(root/f'tests/native/workforce/{unit}.cpp'),'-o',str(binary)]
    subprocess.run(cmd,check=True,timeout=90,capture_output=True,text=True)
    return subprocess.run([str(binary)],timeout=15,capture_output=True,text=True)

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler',action='append'); parser.add_argument('--mutations',action='store_true')
    args=parser.parse_args()
    compilers=args.compiler or [c for c in ('g++','clang++') if shutil.which(c)]
    if not compilers: parser.error('a C++ compiler is required')
    build=Path(tempfile.mkdtemp(prefix='dfmcp-workforce-'))
    report={'evidence':'actual C++ with callback/API doubles; not SDK or live qualification',
            'sources':{p:hashlib.sha256((ROOT/p).read_bytes()).hexdigest() for p in SOURCES},'runs':[]}
    vectors=reference_vectors()
    for index,compiler in enumerate(compilers):
        dest=build/str(index); dest.mkdir()
        result=build_run(compiler,ROOT,dest); result.check_returncode()
        native=dict(line.split('=',1) for line in result.stdout.splitlines() if '=' in line)
        if native != vectors: raise AssertionError('actual C++ and independent Python vectors disagree')
        for name,value in vectors.items():
            existing=ROOT/f'tests/native/workforce/vectors/{name}.hex'
            if existing.exists() and existing.read_text().strip()!=value: raise AssertionError('checked-in fixture drift')
        run={'compiler':compiler,'engine':result.stdout.splitlines()[-1],'vectors':len(vectors),'rejected_mutants':[]}
        if args.mutations:
            mutations={
                'stale-witness':('valid = observe(r.before.ids(), read).encode() == r.before.encode();','valid = true;'),
                'readback':('if (!verify_after(r.before, r.spec, after)) return r;','if (after.units.empty()) return r;'),
                'unknown-retry':('if (r.phase != Phase::Prepared) return r;','if (r.phase == Phase::Applied || r.phase == Phase::Refused || r.phase == Phase::Cancelled) return r;'),
            }
            for name,(old,new) in mutations.items():
                base=dest/name; base.mkdir(); copy=base/'source'
                for relative in SOURCES:
                    target=copy/relative; target.parent.mkdir(parents=True,exist_ok=True); shutil.copyfile(ROOT/relative,target)
                path=copy/SOURCES[1]; content=path.read_text()
                if content.count(old)!=1: raise AssertionError(f'mutation site drift: {name}')
                content=content.replace(old,new)
                # Remove the uncertainty fence as well for the deliberately retrying variant.
                if name=='unknown-retry': content=content.replace('require(!unresolved_, 8);','')
                path.write_text(content)
                killed=build_run(compiler,copy,base)
                if killed.returncode==0: raise AssertionError(f'mutant escaped: {name}')
                run['rejected_mutants'].append(name)
        report['runs'].append(run)
        print(json.dumps(run,sort_keys=True))
    (build/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    print(f'Retained artifacts: {build}')

if __name__=='__main__': main()
