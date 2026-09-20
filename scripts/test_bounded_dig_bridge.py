#!/usr/bin/env python3
"""Compile and execute the complete mining/1.15 plugin with explicit API doubles."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import tempfile
from test_bounded_dig_engine import execute
ROOT = Path(__file__).resolve().parents[1]
PLUGIN = 'bridge/dfhack-dig-v1_15/dfmcp_dig_v1_15.cpp'
FILES = [PLUGIN, 'bridge/dfhack-dig-v1_15/DfmcpDigV1_15.proto', 'bridge/common/dig_designation.h',
         'bridge/common/retained_snapshot.h', 'tests/native/dig/stubs.h', 'tests/native/dig/handler.cpp',
         'scripts/test_bounded_dig_bridge.py', 'scripts/test_bounded_dig_engine.py']
HEADERS = ['Core.h','Export.h','PluginManager.h','RemoteServer.h','VersionInfo.h','TileTypes.h',
           'modules/Maps.h','modules/World.h','df/map_block.h','df/tile_dig_designation.h',
           'df/tiletype_material.h','df/tiletype_shape.h','df/tiletype_special.h','DfmcpDigV1_15.pb.h']
def main() -> None:
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--compiler',default='g++');p.add_argument('--mutations',action='store_true');args=p.parse_args()
    flags=['-std=c++17','-Wall','-Wextra','-Werror','-pedantic','-fsanitize=undefined','-fno-sanitize-recover=all']
    mutants={
        'unknown_fields_accepted':('in->IsInitialized() && in->GetReflection()->GetUnknownFields(*in).empty()', 'in->IsInitialized()'),
        'block_scheduling_marker_removed':('t.block->flags.bits.designated=true;', 't.block->flags.bits.designated=false;'),
        'aquifer_classification_removed':('((d.bits.water_table || o.bits.heavy_aquifer) ? 1 : 0)', '(false ? 1 : 0)'),
        'production_revocation_ignored':('dg::require(enabled("DFMCP_DIG_ALLOW_DESIGNATE"),1);', 'dg::require(true,1);'),
    }
    with tempfile.TemporaryDirectory(prefix='dfmcp-dig-bridge-') as temporary:
        root=Path(temporary)
        for name in FILES:
            target=root/name;target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/name,target)
        includes=root/'includes';includes.mkdir()
        shutil.copyfile(ROOT/'tests/native/dig/stubs.h',includes/'stubs.h')
        for name in HEADERS:
            target=includes/name;target.parent.mkdir(parents=True,exist_ok=True);target.write_text('#include "stubs.h"\n')
        def run(name: str, ok: bool=True):
            binary=root/name
            execute([args.compiler,*flags,'-I',str(includes),str(root/'tests/native/dig/handler.cpp'),'-o',str(binary)])
            return execute([str(binary)],ok=ok)
        actual=json.loads(run('handler').stdout);killed=[];original=(ROOT/PLUGIN).read_text()
        if args.mutations:
            for name,(old,new) in mutants.items():
                if original.count(old)!=(2 if name=='production_revocation_ignored' else 1):raise AssertionError(f'gate moved: {name}')
                (root/PLUGIN).write_text(original.replace(old,new));result=run(name,False)
                if result.returncode!=1 or 'CHECK failed:' not in result.stderr:raise AssertionError(f'mutant not killed by an assertion: {name}: {result.stderr}')
                killed.append(name)
        print(json.dumps({'schema':'dfmcp.bounded-dig-bridge-evidence/1','status':'passed_actual_handler_with_api_doubles',
            'compiler':execute([args.compiler,'--version']).stdout.splitlines()[0],'flags':flags,**actual,'mutants_rejected':killed,
            'real_dfhack_sdk_or_protobuf_runtime':False,'rust_or_mcp_executed':False,'live_game_executed':False,
            'source_sha256':{name:hashlib.sha256((ROOT/name).read_bytes()).hexdigest() for name in FILES}},sort_keys=True,indent=2))
if __name__=='__main__':main()
