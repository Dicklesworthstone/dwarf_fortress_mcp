#!/usr/bin/env python3
"""Independent request/output and source-wiring checks, NOT execution of Rust/MCP."""
from __future__ import annotations
import hashlib
import json
from pathlib import Path
import re
import copy
import jsonschema

ROOT=Path(__file__).resolve().parents[1]
MAX_TICK=(2**32-1)*403200+403199
DIGEST={'type':'string','pattern':'^[0-9a-f]{64}$'}
KEY={'type':'string','pattern':'^[A-Za-z0-9_.-]{1,64}$'}
integer=lambda lo,hi:{'type':'integer','minimum':lo,'maximum':hi}

def schema():
    def variant(mode,properties,required=None):
        fields={'mode':{'const':mode},**properties}
        return {'type':'object','properties':fields,'additionalProperties':False,
                'required':list(fields) if required is None else ['mode']+required}
    register=variant('watch_register',{
        'archive_id':DIGEST,'key':KEY,'native_order_id':integer(0,2**31-1),
        'goal':{'enum':['validated','active','remaining_at_most']},'threshold':{'type':['integer','null'],'minimum':0,'maximum':100},
        'deadline_game_tick':integer(0,MAX_TICK),'cadence_game_ticks':integer(1,10000),
        'stable_samples':integer(1,16),'origin_number':integer(1,4096),'origin_digest':DIGEST},
        ['archive_id','key','native_order_id','goal','deadline_game_tick','cadence_game_ticks','stable_samples','origin_number','origin_digest'])
    register['allOf']=[{'if':{'properties':{'goal':{'const':'remaining_at_most'}}},
                       'then':{'required':['threshold'],'properties':{'threshold':integer(0,100)}},
                       'else':{'properties':{'threshold':{'type':'null'}}}}]
    status={'archive_id':DIGEST,'key':KEY,'definition_digest':DIGEST}
    return {'$schema':'https://json-schema.org/draft/2020-12/schema',
            'title':'Progress/1.12 durable watch requests inside fortress.query history',
            'description':'Wire envelope only. Exact archive, latest origin, authority, finite schedule and cancellation semantics require runtime verification.',
            'oneOf':[register,variant('watch_list',{}),variant('watch_status',status),
                     variant('watch_cancel',{**status,'expected_archive_head':DIGEST})]}

def unique(pairs):
    out={}
    for key,value in pairs:
        if key in out:raise ValueError('duplicate key')
        out[key]=value
    return out

def parse(raw):
    if not 1<=len(raw.encode('utf-8'))<=2048 or '\0' in raw:raise ValueError('extent')
    value=json.loads(raw,object_pairs_hook=unique)
    jsonschema.validate(value,schema())
    return value

def rejected(raw):
    try:parse(raw)
    except (ValueError,jsonschema.ValidationError):return
    raise AssertionError('accepted invalid request')

def compact(value):return json.dumps(value,separators=(',',':'),ensure_ascii=True)

def size_models():
    reference={'number':4096,'record_digest':'f'*64}
    definition={'key':'k'*64,'definition_digest':'f'*64,'archive_id':'f'*64,'native_order_id':2**31-1,
        'goal':'remaining_at_most','threshold':100,'deadline_game_tick':MAX_TICK,'cadence_game_ticks':10000,
        'stable_samples':16,'origin':reference,'registered_tick':MAX_TICK-120000,'comparison_segment':4096}
    evaluation={'definition':definition,'state':'satisfied_observation','terminal':True,
        'evaluated_through':reference,'evaluated_records':4096,'positive_samples':[reference]*16,
        'next_sample_tick':MAX_TICK,'production_completion_proven':False,'continuous_history_proven':False,
        'historical_creation_identity_proven':False,'current_freshness_proven':False}
    index={'configured':True,'book_id':'f'*64,'book_head':'f'*64,'archive_id':'f'*64,'retained_bytes':131072,
        'events':64,'read_only':False,'definitions':[{'key':f'{i:02}'+('k'*62),'definition_digest':'f'*64} for i in range(32)],
        'definition_count':32,'outcomes_evaluated':False,'pending_absence_proven':False,
        'discovery':{'tool':'fortress.query','session_id':'f'*32,'history':'{"mode":"watch_list"}'}}
    result={'ok':True,'watch_result':{'watches':[evaluation]*32,'complete_watch_set':True,'archive_head':'f'*64,
        'archive_records':4096,'watch_book_head':'f'*64,'pending':32},'native_calls':0,'game_mutation_dispatched':False,
        'production_completion_proven':False,'observations_are_historical':True}
    row_bytes=len(compact(evaluation));index_bytes=len(compact(index))
    # Independent 16 KiB whole-envelope allowance INCLUDING the separate index;
    # enforce index plus an additional 8 KiB original packet allowance fits it.
    assert index_bytes+8192<=16384
    response_bytes=len(compact(result))+16384
    assert row_bytes<4096 and response_bytes<=16384+32*4096
    return {'max_watch_model_bytes':row_bytes,'book_index_model_bytes':index_bytes,
            'full_response_model_bytes':response_bytes,'full_response_reservation_bytes':147456}

def main():
    contract=schema();jsonschema.Draft202012Validator.check_schema(contract)
    path=ROOT/'architecture/progress_watch_requests_v1.json'
    if path.exists():assert json.loads(path.read_text())==contract
    else:path.write_text(json.dumps(contract,indent=2,sort_keys=True)+'\n')
    base={'mode':'watch_register','archive_id':'f'*64,'key':'watch','native_order_id':3,'goal':'validated',
        'deadline_game_tick':30,'cadence_game_ticks':1,'stable_samples':2,'origin_number':1,'origin_digest':'f'*64}
    valid=[]
    for goal in ('validated','active','remaining_at_most'):
        for cadence in (1,10000):
            for stable in (1,16):
                v=copy.deepcopy(base);v.update(goal=goal,cadence_game_ticks=cadence,stable_samples=stable)
                if goal=='remaining_at_most':v['threshold']=0
                valid.append(v)
    for goal in ('validated','active'):
        v=copy.deepcopy(base);v.update(goal=goal,threshold=None);valid.append(v)
    valid += [{'mode':'watch_list'}, {'mode':'watch_status','archive_id':'0'*64,'key':'key','definition_digest':'f'*64},
              {'mode':'watch_cancel','archive_id':'0'*64,'key':'key','definition_digest':'f'*64,'expected_archive_head':'a'*64}]
    for value in valid:parse(compact(value))
    bad=[]
    for field,values in {'key':['','bad/key','k'*65,'é'], 'native_order_id':[-1,2**31,True],
        'deadline_game_tick':[-1,MAX_TICK+1,False], 'cadence_game_ticks':[0,10001,1.5],
        'stable_samples':[0,17,True], 'origin_number':[0,4097,-1],
        'origin_digest':['f'*63,'A'*64,'z'*64], 'archive_id':['f'*65,None],
        'threshold':[0,100], 'goal':['completed_goods','unknown',None]}.items():
        for wrong in values:
            v=copy.deepcopy(base);v[field]=wrong;bad.append(compact(v))
    v=copy.deepcopy(base);v['goal']='remaining_at_most'
    bad.append(compact(v));v['threshold']=None;bad.append(compact(v))
    for v in valid:
        v=copy.deepcopy(v);v['unexpected']=1;bad.append(compact(v))
    bad+=['{}','[]','null','{"mode":"watch_list","mode":"watch_list"}', ' '*2049,'\0',
          compact(base).replace('"stable_samples":2','"stable_samples":2,"stable_samples":2')]
    for raw in bad:rejected(raw)
    source=(ROOT/'crates/dfmcp-mcp/src/live_work_order_progress_server.rs').read_text()
    module=(ROOT/'crates/dfmcp-mcp/src/progress_watches.rs').read_text()
    assert len(re.findall(r'#\[tool\(',source))==11
    assert 'watch_book:watch_book.take()' in source and 'session.watch_book = watch_book.take()' in source
    assert source.index('let mut watch_book =')<source.index('let token = configured(ENVIRONMENT[1]')
    assert 'book.verify_access(archive, &current)?' in source and 'watches::attach(&mut value' in source
    assert 'watches::Request::parse(raw).map(Query::Watch)' in source
    assert all('.'+call+'(' not in module for call in ('connect','commit','prepare','refresh'))
    files=['crates/dfmcp-mcp/src/live_work_order_progress_server.rs','crates/dfmcp-mcp/src/progress_watches.rs',
           'crates/dfmcp-mcp/src/progress_watches_tests.rs','architecture/progress_watch_requests_v1.json',
           'scripts/check_progress_watch_mcp_reference.py']
    print(json.dumps({'schema':'dfmcp.progress-watch-mcp-reference/1','status':'passed_reference_only',
        'accepted_request_cases':len(valid),'rejected_request_cases':len(bad),**size_models(),
        'mcp_tools_wired':11,'rust_compiled':False,'rust_tests_executed':False,
        'actual_mcp_or_native_or_filesystem_executed':False,
        'scope':'Independent Python JSON envelope/size model and lexical source checks, not Rust or MCP execution',
        'source_sha256':{p:hashlib.sha256((ROOT/p).read_bytes()).hexdigest() for p in files}},indent=2,sort_keys=True))

if __name__=='__main__':main()
