#!/usr/bin/env python3
"""Strict actual-handler tree/1.21 component checks; explicit SDK/protobuf doubles only."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time
from read_stable_repository_file import read_stable_regular_file

ROOT=Path(__file__).resolve().parents[1]
HANDLER="bridge/dfhack-tree-harvest-v1_21/dfmcp_tree_harvest_v1_21.cpp"
HEADER="bridge/common/tree_harvest.h"
TEST="tests/native/tree_harvest/handler.cpp"
VECTORS="tests/native/tree_harvest/vectors.json"
INPUTS=["architecture/tree_harvest_v1_21.json",HEADER,"bridge/common/one_attempt_native.h","bridge/common/workshop_construction.h",
        "bridge/common/retained_snapshot.h",HANDLER,"bridge/dfhack-tree-harvest-v1_21/DfmcpTreeHarvestV1_21.proto",
        "bridge/dfhack-tree-harvest-v1_21/CMakeLists.txt",TEST,"tests/native/tree_harvest/stubs.h","tests/native/workshop/stubs.h",VECTORS,
        "scripts/test_tree_harvest_native.py","scripts/read_stable_repository_file.py"]
FLAGS=["-std=c++17","-Wall","-Wextra","-Werror","-pedantic","-g","-fsanitize=undefined","-fno-sanitize-recover=all"]
GROUPS=["rpc_closed_surface_and_gates","designate_supported_southeast_once","computed_target_approach_witnesses",
        "tree_visibility_geometry_and_job_conflicts","worker_and_tool_offers_are_separate","stale_source_lifetime_and_replay",
        "lost_reply_partial_write_and_recovery","progress_root_predicate_and_sorted_supply"]
MUTATIONS={
    "worker_approaches_root_instead_of_chop_target":(HANDLER,"workers(out.designation,out.complete)","workers(r,out.complete)"),
    "tool_approaches_root_instead_of_chop_target":(HANDLER,"tools(out.designation,out.complete)","tools(r,out.complete)"),
    "missing_body_visibility_barrier":(HANDLER,"if(!out.complete) return out;","if(false) return out;"),
    "structural_tool_role_offered":(HEADER,"&& role == 0;","&& role >= 0;"),
    "missing_sorted_progress":(HANDLER,"std::sort(rows.begin(),rows.end(),[](const auto&a,const auto&b){return a.first<b.first;});",""),
    "missing_tree_occupancy_witness":(HEADER,"tiletype, designation_other, occupancy, block_other","tiletype, designation_other, block_other, block_other"),
    "missing_writer_credential_recheck":(HANDLER,"authorize_credential(credential);","(void)credential;"),
}

def inventory(root:Path)->dict[str,str]:
    out={}
    for name in INPUTS:
        value=read_stable_regular_file(root/name,4*1024*1024,name)
        if "\0" in value.content.decode("utf-8",errors="strict"):
            raise ValueError(f"NUL in required tree source: {name}")
        out[name]=value.sha256
    return out

def commitments(v:dict[str,str])->None:
    if set(v)!={"capture","plan","token","prepared","designated","indeterminate","refused","cancelled","progress_wait","progress_supply"}:
        raise AssertionError("tree vector set differs")
    raw={k:bytes.fromhex(value) for k,value in v.items()}
    capture=raw["capture"]
    if capture[:8]!=b"DFMTC021":raise AssertionError("tree capture version")
    # Independently fixed test source/root; no fake plant ID is encoded.
    if capture[8:32]!=b"".join(n.to_bytes(8,"big") for n in (7,0,12345)):
        raise AssertionError("tree fixture frontier")
    folder_len=int.from_bytes(capture[52:54],"big")
    if capture[54:54+folder_len]!=b"region1":raise AssertionError("tree fixture source")
    root=capture[54+folder_len:66+folder_len]
    if root!=b"".join(n.to_bytes(4,"big") for n in (15,15,2)):raise AssertionError("tree fixture root")
    plan=hashlib.sha256(b"dfmcp-tree-harvest-plan/1\0"+root+hashlib.sha256(capture).digest()).digest()
    token=hashlib.sha256(b"dfmcp-tree-harvest-token/1\0"+len(b"tree-001").to_bytes(2,"big")+b"tree-001"+plan).digest()[:16]
    if raw["plan"]!=plan or raw["token"]!=token:raise AssertionError("tree plan/token commitment")
    for name,phase in {"prepared":0,"indeterminate":1,"designated":2,"refused":3,"cancelled":4}.items():
        record=raw[name]
        if record[:8]!=b"DFMTR021" or hashlib.sha256(b"dfmcp-tree-harvest-receipt/1\0"+record[:-32]).digest()!=record[-32:]:
            raise AssertionError(f"tree {name} receipt checksum")
        n=int.from_bytes(record[8:10],"big");at=10+n
        if record[10:at]!=b"tree-001" or record[at:at+32]!=plan or record[at+32:at+48]!=token:raise AssertionError(f"tree {name} plan binding")
        at+=48;n=int.from_bytes(record[at:at+4],"big");at+=4
        if record[at:at+n]!=capture or record[at+n]!=phase:raise AssertionError(f"tree {name} original capture/phase")
    for name in ["progress_wait","progress_supply"]:
        if raw[name][:8]!=b"DFMTP021":raise AssertionError("tree progress version")

def main()->None:
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler",default="g++")
    parser.add_argument("--mutations",action="store_true")
    parser.add_argument("--evidence",type=Path)
    parser.add_argument("--emit-new-vectors",type=Path,help="initial NEW generation candidate only; never a qualification result")
    args=parser.parse_args()
    directory=args.evidence.parent if args.evidence else ROOT/"target/tree-native-component"/str(time.time_ns())
    directory.mkdir(mode=0o700,parents=True,exist_ok=True)
    evidence=args.evidence or directory/"component.json"
    trace=evidence.with_name(evidence.stem+"-commands");trace.mkdir(mode=0o700)
    before=inventory(ROOT);sequence=0
    compiler=shutil.which(args.compiler)
    if compiler is None:raise RuntimeError("requested compiler unavailable")
    contract=json.loads((ROOT/"architecture/tree_harvest_v1_21.json").read_text())
    if contract["native_component_source_paths"]!=INPUTS or contract["native_scenarios"]!=GROUPS or contract["native_mutants"]!=list(MUTATIONS):
        raise AssertionError("tree machine contract and actual native inventory differ")
    def run(argv:list[str])->subprocess.CompletedProcess[str]:
        nonlocal sequence
        sequence+=1
        result=subprocess.run(argv,text=True,capture_output=True,timeout=180,check=False)
        for suffix,value in [("argv.json",json.dumps(argv)+"\n"),("stdout",result.stdout),("stderr",result.stderr),("exit",str(result.returncode)+"\n")]:
            Path(f"{trace/str(sequence).zfill(3)}.{suffix}").write_text(value,encoding="utf-8")
        return result
    def compile_test(root:Path,work:Path)->Path:
        include=work/"include";include.mkdir(parents=True)
        for name in re.findall(r'#include "([^"\n]+)"',(root/HANDLER).read_text()):
            if name.startswith(".."):continue
            target=include/name;target.parent.mkdir(parents=True,exist_ok=True);target.write_text("#pragma once\n")
        binary=work/"tree-native-test"
        result=run([compiler,*FLAGS,f"-I{include}",str(root/TEST),"-o",str(binary)])
        if result.returncode:raise RuntimeError(f"tree handler compilation failed; raw {trace}:\n{result.stderr}")
        return binary
    with tempfile.TemporaryDirectory(prefix="dfmcp-tree-native-") as name:
        work=Path(name);binary=compile_test(ROOT,work)
        result=run([str(binary)])
        if result.returncode:raise RuntimeError(f"tree actual-handler regression failed; raw {trace}:\n{result.stderr}")
        report=json.loads(result.stdout)
        if report["groups"]!=GROUPS or report["scenarios"]!=len(GROUPS) or report["ignored"]!=0:raise AssertionError("tree scenario denominator differs")
        result=run([str(binary),"--vectors"])
        if result.returncode:raise RuntimeError(f"tree vectors failed: {result.stderr}")
        vectors=json.loads(result.stdout);commitments(vectors)
        if args.emit_new_vectors:
            with args.emit_new_vectors.open("x") as output:output.write(json.dumps(vectors,sort_keys=True,indent=2)+"\n")
        elif json.loads((ROOT/VECTORS).read_text())!=vectors:raise AssertionError("tree vectors differ; no regeneration is performed")
        killed=[]
        if args.mutations:
            for label,(path,old,new) in MUTATIONS.items():
                copy=work/label
                for name in INPUTS:
                    target=copy/name;target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/name,target)
                target=copy/path;source=target.read_text()
                if source.count(old)!=1:raise AssertionError(f"mutant does not apply exactly once: {label}")
                target.write_text(source.replace(old,new));mutant=compile_test(copy,copy)
                rejected=run([str(mutant)])
                if rejected.returncode==0 or "check failed:" not in rejected.stderr:raise AssertionError(f"mutant not rejected by runtime assertion: {label}: {rejected.stderr}")
                killed.append(label)
    if inventory(ROOT)!=before:raise AssertionError("required tree source changed during component execution")
    report={"schema":"dfmcp.tree-harvest-native-component/1","status":"new_fixture_candidate_unqualified" if args.emit_new_vectors else "passed_sdk_doubles_only",
            "actual_handler":report,"hashlib_verified_vectors":len(vectors),"killed_mutants":killed,"flags":FLAGS,"source_sha256":before,
            "real_dfhack_sdk":False,"protobuf_serialization":False,"live_fortress":False,"rust_executed":False,"qualification":False}
    with evidence.open("x") as output:output.write(json.dumps(report,indent=2,sort_keys=True)+"\n")
    print(json.dumps(report,sort_keys=True))
if __name__=="__main__":main()
