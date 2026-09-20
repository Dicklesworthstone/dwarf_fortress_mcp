#!/usr/bin/env python3
"""Independent condition/pagination/output models and lexical wiring checks.

This script does not compile or execute Rust, generated macros, MCP, storage or
DFHack. The Rust tests exercise those seams when a toolchain is available.
"""
from __future__ import annotations
import itertools
import json
from pathlib import Path
import re
import unittest

ROOT=Path(__file__).resolve().parents[1]
TOOLS=['open_session','observe','query','plan','commit','wait','cancel','checkpoint','restore','explain','doctor']
HANDLERS=['Fortress'+''.join(p.title() for p in name.split('_')) for name in TOOLS]
REQUIRED={'order_id','predicate','game_ticks','wall_millis'}
OPTIONAL={'threshold','stable_samples','interval_ticks'}
BASE=16384
ROW=16384
VIEW=3*1024*1024
REPORT={'scope':'independent Python models and lexical checks only', 'rust_or_mcp_executed':False}

def check(ok):
    if not ok:raise ValueError('closed condition rejected')

def parse(raw):
    check(len(raw.encode())<=2048)
    def fields(pairs):
        out={}
        for k,v in pairs:
            check(k not in out);out[k]=v
        return out
    value=json.loads(raw,object_pairs_hook=fields)
    check(isinstance(value,dict) and REQUIRED<=value.keys() and value.keys()<=REQUIRED|OPTIONAL)
    def integer(name,lo,hi,default=None):
        v=value.get(name,default)
        if v is None and name in OPTIONAL:v=default
        check(type(v) is int and lo<=v<=hi)
        return v
    integer('order_id',0,2**31-1)
    ticks=integer('game_ticks',1,1200);integer('wall_millis',1,60000)
    threshold=integer('threshold',0,100,0)
    samples=integer('stable_samples',1,16,1);interval=integer('interval_ticks',1,1200,1)
    check(samples*interval<=ticks)
    check(value['predicate'] in ('approved','active','remaining_at_most'))
    check(value['predicate']=='remaining_at_most' or threshold==0)
    return value

def compact(v):return json.dumps(v,sort_keys=True,separators=(',',':'),ensure_ascii=True)

def sample():
    return {'fortress_id':'f'*32,'world_folder':'\x01'*512,'site_id':2**31-1,
        'generation':2**64-2,'sequence':2**64-2,'tick':2**64-1,'paused':False,
        'order_id':2**31-1,'allocation_horizon':2**31-1,'present':True,'recipe_code':4,
        'amount_total':100,'amount_remaining':100,'status_bits':3,'witness':'f'*64,
        'current_freshness_proven':False,'evidence_scope':'one_native_capture','goods_produced_proven':False}

def entry():
    return {'plan':{'idempotency_key':'k'*128,'plan_digest':'f'*64,
        'condition':{'order_id':2**31-1,'predicate':'remaining_at_most','threshold':100,'stable_samples':16,
                     'interval_ticks':1200,'game_ticks':1200,'wall_millis':60000},'before':sample()},
        'state':'cancelled_before_dispatch','settled_in_this_coordinator':False,'reconciliation_required':True,
        'native':{'phase':'source_lost','clock_reason':'clock_regression','trigger':'counter_regression',
            'unpause_attempted':True,'pause_verified':True,'observed_tick':2**64-1,'predicate_observed':True,
            'reported_stable_samples':16,'counted_tick':2**64-1,'sample':sample(),'receipt_digest':'f'*64,
            'observed_tick_overshoot':2**64-1,'full_sample_trace_available':False},
        'historical_evidence_only':True,'current_pause_unproved':True,'goods_produced_proven':False}

def packet_model(rows,mode):
    # Conservative superset: four pending references plus both maximum escaped
    # captures on every row, including impossible state combinations for sizing.
    refs=[{'idempotency_key':'k'*128,'plan_digest':'f'*64,'state':'cancel_requested','reconciliation_required':True} for _ in range(4)]
    summary={'journal_id':'f'*64,'head':'f'*64,'transitions':4096,'records':256,'pending':256,'unresolved':256,'terminal':256}
    turn={'schema':'dfmcp.agent_turn/1','operation':'fortress.query','phase':'inspect',
        'session_id':'f'*32,'turn_id':None,'request_id':'f'*32,
        'anchor':{'fortress_id':'f'*32,'cursor':{'epoch':2**64-2,'sequence':4096},'tick':0,
                  'state_hash':'f'*64,'scope':'coordination_root_not_world_state','tick_is_sentinel':True},
        'continuity':{'status':'indeterminate','basis':None,'gap':{'game_history_continuity':'unestablished'},'reset_reason':None},
        'profile':'briefing','briefing':{'runtime':'unadmitted_development','bridge_protocol':'1.14','runtime_admitted':False,
            'mode':mode,'world_state_anchor':False,'global_clock_lease_established':False,
            'limit_semantics':'native_callback_stop_triggers_not_hard_real_time','current_pause_unproved':True,'goods_produced_proven':False},
        'changes':[],'attention':[],'active_work':{'scope':'this_verified_conditional_run_journal_only','inventory_verified':True,
            'absence_proven':False,'pending_plans':refs,'actions':refs,'indeterminate_effects':refs,'cancellation_drains':refs,
            'obligations':[],'publications':[],'confirmations':[],'counts':summary,'omitted_pending_references':252},
        'affordances':[],'recommendations':[{'recommendation_id':'discover-order-runs','tool':'fortress.query',
            'reason':'Inspect durable work before requesting new control.','expected_utility':'high','expected_information_value':'high',
            'risk':'read_only','reversibility':'not_applicable','estimated_cost':dict.fromkeys(['output_tokens','bridge_bytes','wall_millis','game_ticks']),
            'prerequisites':[],'invalidating_conditions':[],'confidence':{'epistemic_state':'certified_derived','value':None,'evidence':[]},
            'requires_confirmation':False,'arguments':{'session_id':'f'*32,'state':'pending','limit':2}}],
        'uncertainty':[{'uncertainty_id':'sampled-not-current','epistemic_state':'unknown',
            'statement':'Predicate samples and pause receipts are historical, not continuous truth or produced goods.',
            'consequence':'Never replay a dispatched run; recover its exact key and digest.','resolution':None,'evidence':[]}],
        'coverage':{'status':'partial','complete_domains':['this_journal_coordination'],
            'partial_domains':['sampled_native_order_and_pause_evidence'],'omitted_domains':['current_world_state','produced_goods','other_controllers']},
        'budget':{'admitted':{'max_wall_millis':60000,'max_game_ticks':1200,'max_bytes':64*1024*1024,'max_output_tokens':65536},
            'output_proxy_bytes_per_token':4,'consumed':{},'accounting':'conservative_reservation_not_measured_tokenization'},'references':[]}
    return {'result':{'ok':True,'records':[entry() for _ in range(rows)],'state':'unresolved','matching_records':256,
                     'offset':256,'continuation':'f'*64,'last_page':False,'complete_matching_set_in_this_response':False,
                     'journal':summary,'native_calls':0},'agent_turn':turn}

class Reference(unittest.TestCase):
    def test_explicit_tools_binary_module_and_guard_wiring(self):
        source=(ROOT/'crates/dfmcp-mcp/src/live_order_run_server.rs').read_text()
        names=re.findall(r'#\[tool\(name="([^"]+)"',source)
        self.assertEqual(set(names),{'fortress.'+s for s in TOOLS});self.assertEqual(len(names),11)
        registered=re.findall(r'\.tool\((Fortress\w+)\)',source);self.assertEqual(registered,HANDLERS)
        self.assertIn('pub mod live_order_run_server;', (ROOT/'crates/dfmcp-mcp/src/lib.rs').read_text())
        self.assertIn('dfmcp_mcp::live_order_run_server::run_stdio();',(ROOT/'crates/dfmcp-mcp/src/bin/dfmcp-live-order-run-dev-server.rs').read_text())
        native=(ROOT/'crates/dfmcp-mcp/src/live_order_run_server/native.rs').read_text()
        self.assertEqual(native.count('(self.check)(true,self.inner.fortress())?'),3)
        self.assertEqual(native.count('(self.check)(false,self.inner.fortress())?'),2)
        self.assertIn('crate::run_modern_stdio(server)',source)
        self.assertNotIn('std::process::Command',source)
    def test_condition_boundary_matrix(self):
        accepted=rejected=0
        for predicate,ticks,wall,samples,interval,threshold in itertools.product(
                ['approved','active','remaining_at_most'],[1,20,1200],[1,60000],[1,2,16],[1,2,75,1200],[0,1,100]):
            value=dict(order_id=9,predicate=predicate,game_ticks=ticks,wall_millis=wall,stable_samples=samples,interval_ticks=interval,threshold=threshold)
            expected=samples*interval<=ticks and (predicate=='remaining_at_most' or threshold==0)
            if expected:parse(compact(value));accepted+=1
            else:
                with self.assertRaises(ValueError):parse(compact(value))
                rejected+=1
        REPORT.update(condition_accepted=accepted,condition_rejected=rejected)
    def test_closed_condition_duplicates_types_and_size(self):
        base=dict(order_id=9,predicate='approved',game_ticks=20,wall_millis=1000)
        parse(compact(base));parse(compact({**base,**dict.fromkeys(OPTIONAL)}))
        bad=[{**base,'method':'CommitRun'},{**base,'order_id':True},{**base,'order_id':-1},{**base,'order_id':2**31},
             {**base,'game_ticks':1.0},{**base,'wall_millis':0},{**base,'wall_millis':60001},
             {**base,'game_ticks':1201},{**base,'threshold':101},{**base,'stable_samples':0},{**base,'interval_ticks':1201}]
        for v in bad:
            with self.assertRaises(ValueError):parse(compact(v))
        for raw in [compact(base)[:-1]+',"order_id":8}',compact(base)+' '*2048,'[]','null']:
            with self.assertRaises(ValueError):parse(raw)
    def test_complete_pagination(self):
        cases=0
        for count in range(257):
            for width in range(1,9):
                offset=0;collected=[]
                while True:
                    end=min(offset+width,count);collected.extend(range(offset,end))
                    self.assertEqual(offset==0 and end==count,count<=width)
                    if end==count:break
                    self.assertGreater(end,offset);offset=end
                self.assertEqual(collected,list(range(count)));cases+=1
        REPORT['pagination_cases']=cases
    def test_cursor_binding_model(self):
        frozen=('session','journal','head','pending',2,4)
        issued={'token':frozen}
        def resolve(token,current):
            check(token in issued and issued[token][:5]==current[:5]);return issued[token][5]
        self.assertEqual(resolve('token',frozen),4)
        for i in range(5):
            changed=list(frozen);changed[i]='different'
            with self.assertRaises(ValueError):resolve('token',tuple(changed))
        with self.assertRaises(ValueError):resolve('unissued',frozen)
    def test_response_and_work_reservation_arithmetic(self):
        for rows in range(9):
            output=BASE+ROW*rows
            self.assertLessEqual(output,65536*4)
            work=32*1024*1024-output-2*VIEW
            self.assertGreater(work,8*272*1024+2*1024*1024)
            self.assertLess((output//4-1)*4,output)
        self.assertEqual(BASE+ROW*8,147456)
    def test_maximal_reference_packets(self):
        sizes=[]
        for rows in range(9):
            for mode in ['offline','recover','control']:
                raw=compact(packet_model(rows,mode)).encode();sizes.append(len(raw))
                self.assertLessEqual(len(raw),BASE+ROW*rows)
                p=json.loads(raw)
                self.assertFalse(p['agent_turn']['briefing']['runtime_admitted'])
                self.assertFalse(p['agent_turn']['active_work']['absence_proven'])
        REPORT.update(packet_shapes=len(sizes),packet_min_bytes=min(sizes),packet_max_bytes=max(sizes))
    def test_machine_contract_agrees_with_fixed_profile(self):
        contract=json.loads((ROOT/'architecture/order_run_mcp_v1_14.json').read_text())
        self.assertEqual(contract['tool_names'],['fortress.'+s for s in TOOLS])
        self.assertEqual(contract['limits']['max_page_records'],8)
        self.assertEqual(contract['limits']['output_base_bytes'],BASE)
        self.assertEqual(contract['limits']['output_row_bytes'],ROW)
        self.assertEqual(contract['modes'],['offline','recover','control'])
        self.assertFalse(contract['runtime_admitted'])
        self.assertFalse(contract['rust_tests_executed'])

if __name__=='__main__':
    suite=unittest.defaultTestLoader.loadTestsFromTestCase(Reference)
    result=unittest.TextTestRunner(verbosity=2).run(suite)
    print(json.dumps(REPORT,sort_keys=True))
    raise SystemExit(0 if result.wasSuccessful() else 1)
