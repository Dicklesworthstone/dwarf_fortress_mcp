#!/usr/bin/env python3
"""Independent schema/three-valued set reference, NOT execution of Rust/MCP."""
from __future__ import annotations
import hashlib
import copy
import itertools
import json
from pathlib import Path
import sys
import jsonschema

ROOT = Path(__file__).resolve().parents[1]

def target(i=0, items=True):
    t = dict(building_native_id=10+i, building_generation=1, kind='bed', max_stage=3)
    if items:
        t.update(item_native_id=100+i, item_generation=1)
    return t

def condition(n=1, test='all_complete'):
    return dict(op='furniture_set', targets=[target(i) for i in range(n)], test=test)

def semantics(value):
    ts = value['targets']
    return (all(t['building_generation'] > 0 and 1 <= t['max_stage'] <= 32 for t in ts)
        and all(a['building_native_id'] < b['building_native_id'] for a,b in zip(ts,ts[1:]))
        and len([t['item_native_id'] for t in ts if t.get('item_native_id') is not None])
            == len({t['item_native_id'] for t in ts if t.get('item_native_id') is not None}))

def fold(values, all_complete):
    decisive = False if all_complete else True
    if decisive in values:
        return decisive
    if None in values:
        return None
    return all_complete

def input_allowance(value):
    pending=[(value,0)]; nodes=0; size=0
    while pending:
        v,depth=pending.pop(); nodes+=1;size+=32
        assert depth<=24 and nodes<=1024
        if isinstance(v,str):size+=len(v.encode())
        elif isinstance(v,list):pending.extend((n,depth+1) for n in v)
        elif isinstance(v,dict):
            size+=sum(len(k.encode()) for k in v)
            pending.extend((n,depth+1) for n in v.values())
        assert size<=32768
    return nodes,size

def run():
    schema=json.loads((ROOT/'schemas/mcp_watch_furniture_set_v1.json').read_text())
    jsonschema.Draft202012Validator.check_schema(schema)
    validator=jsonschema.Draft202012Validator(schema)
    accepted=[]
    for n in (1,2,8,24,32):
        for test in ('all_complete','any_removal'):
            for items in (False,True):
                v=condition(n,test);v['targets']=[target(i,items) for i in range(n)];accepted.append(v)
    invalid=[]
    for n in (0,33):invalid.append(condition(n))
    for field,value in [('building_native_id',-1),('building_native_id',2147483647),
        ('building_generation',0),('building_generation',4294967296),('kind','workshop'),
        ('max_stage',0),('max_stage',33),('max_stage',True),('item_native_id',-1),
        ('item_generation',0),('skip_missing',True),('item_native_id',None),('item_generation',None)]:
        v=condition();v['targets'][0][field]=value;invalid.append(v)
    for key in ('building_native_id','building_generation','kind','max_stage','item_native_id','item_generation'):
        v=condition();del v['targets'][0][key];invalid.append(v)
    for field,value in [('op','construction_complete'),('test','any_complete'),('fallback',True)]:
        v=condition();v[field]=value;invalid.append(v)
    for v in accepted:assert validator.is_valid(v) and semantics(v),v
    for v in invalid:assert not validator.is_valid(v),v
    for edit in ('duplicate_building','unsorted','duplicate_item'):
        v=condition(2)
        if edit=='duplicate_building':v['targets'][1]['building_native_id']=10
        elif edit=='unsorted':v['targets'].reverse()
        else:v['targets'][1]['item_native_id']=100
        assert not semantics(v)
    query_schema=json.loads((ROOT/'schemas/mcp_construction_progress_v1.json').read_text())
    jsonschema.Draft202012Validator.check_schema(query_schema)
    qv=jsonschema.Draft202012Validator(query_schema)
    query={'kind':'construction_progress','targets':[{'building_native_id':10,'item_native_id':20}],
        'monitor':{'mode':'all_targets','key_prefix':'bedrooms','deadline_tick':1000}}
    query_valid=[]
    for count in (1,8,32):
        for mode in ('absent',None,'all_targets'):
            value=copy.deepcopy(query)
            value['targets']=[{'building_native_id':10+i,'item_native_id':100+i} for i in range(count)]
            if mode=='absent':del value['monitor']['mode']
            else:value['monitor']['mode']=mode
            query_valid.append(value)
    query_invalid=[]
    for mode in ('per_target','any_target','',True,1,[],{}):
        value=copy.deepcopy(query);value['monitor']['mode']=mode;query_invalid.append(value)
    for field,value in [('commit',True),('skip_missing',True),('protocol','1.19')]:
        changed=copy.deepcopy(query);changed['monitor'][field]=value;query_invalid.append(changed)
    for value in query_valid:assert qv.is_valid(value),value
    for value in query_invalid:assert not qv.is_valid(value),value
    # Schema extension must retain every prior property and constraint verbatim.
    baseline=copy.deepcopy(query_schema)
    del baseline['properties']['monitor']['oneOf'][1]['properties']['mode']
    raw=(json.dumps(baseline,indent=2)+'\n').encode()
    assert hashlib.sha1(b'blob '+str(len(raw)).encode()+b'\0'+raw).hexdigest()=='4ef4be92f33f1deb3a2d5fd4551384da8a6563c1'
    cases=0
    for n in range(1,9):
        for values in itertools.product((False,None,True),repeat=n):
            for all_complete in (True,False):
                result=fold(values,all_complete)
                lower=[False if v is None else v for v in values]
                upper=[True if v is None else v for v in values]
                op=all if all_complete else any
                expected=op(lower) if op(lower)==op(upper) else None
                assert result is expected
                assert result is fold(tuple(reversed(values)),all_complete)
                cases+=1
    request={'schema':'dfmcp.query/1','query':{'kind':'watch','key':'furnished-plan','label':'All furniture',
        'condition':condition(32),'failure_condition':condition(32,'any_removal'),
        'deadline_tick':1000,'poll_interval_ticks':1,'stable_observations':2}}
    for node in (request['query']['condition'],request['query']['failure_condition']):
        for i,t in enumerate(node['targets']):
            t.update(building_native_id=2147483614+i,building_generation=4294967295,
                item_native_id=2147483614+i,item_generation=4294967295)
    nodes,allowance=input_allowance(request)
    sources={}
    for file in ['crates/dfmcp-mcp/src/query_watch.rs','crates/dfmcp-mcp/src/query_watch_construction.rs',
        'crates/dfmcp-mcp/src/query_watch_construction_tests.rs','schemas/mcp_watch_furniture_set_v1.json',
        'crates/dfmcp-mcp/src/spatial_construction.rs','crates/dfmcp-mcp/src/spatial_construction_set.rs',
        'crates/dfmcp-mcp/src/spatial_construction_set_tests.rs','schemas/mcp_construction_progress_v1.json',
        'scripts/check_furniture_set_reference.py']:
        raw=(ROOT/file).read_bytes()
        sources[file]={'sha256':hashlib.sha256(raw).hexdigest(),
            'git_blob':hashlib.sha1(b'blob '+str(len(raw)).encode()+b'\0'+raw).hexdigest()}
    return dict(schema='dfmcp.furniture-set-reference/1',evidence_scope='independent JSON schema, target and truth-table reference only',
        accepted_schema_cases=len(accepted),rejected_schema_cases=len(invalid),rejected_semantic_cases=3,
        accepted_query_schema_cases=len(query_valid),rejected_query_schema_cases=len(query_invalid),
        prior_query_schema_preserved_except_additive_mode=True,
        rust_test_functions_authored={'shared_condition':10,'shared_spatial_query':5},
        ternary_set_cases=cases,max_request_nodes=nodes,max_request_accounted_bytes=allowance,
        max_request_serialized_bytes=len(json.dumps(request,separators=(',',':')).encode()),
        rust_compiled=False,rust_tests_executed=False,mcp_executed=False,native_executed=False,
        live_fortress=False,full_repository_qualification=False,source_files=sources)

if __name__=='__main__':
    json.dump(run(),sys.stdout,indent=2);print()
