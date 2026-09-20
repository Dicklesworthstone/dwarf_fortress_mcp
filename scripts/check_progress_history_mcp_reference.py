#!/usr/bin/env python3
"""Independent request/cursor/output models; NOT Rust, MCP or filesystem execution."""
from __future__ import annotations
from collections import deque
import hashlib
import json
from pathlib import Path
import struct
from check_progress_archive_reference import body, decode, make_archive, sample

ROOT = Path(__file__).resolve().parents[1]
DIGEST = 'a' * 64

def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate field')
        result[key] = value
    return result

def digest(value):
    if not isinstance(value, str) or len(value) != 64 or any(c not in '0123456789abcdef' for c in value):
        raise ValueError('canonical digest')

def request(raw):
    if len(raw.encode()) > 2048:
        raise ValueError('request bound')
    value = json.loads(raw, object_pairs_hook=unique_object)
    if not isinstance(value, dict):
        raise ValueError('object required')
    mode = value.get('mode')
    fields = {'list': {'mode', 'limit', 'continuation'},
              'record': {'mode', 'archive_id', 'number', 'record_digest'},
              'changes': {'mode', 'archive_id', 'before_number', 'before_digest', 'after_number', 'after_digest'}}
    if not isinstance(mode, str) or mode not in fields or set(value) - fields[mode]:
        raise ValueError('unknown fields or mode')
    if mode == 'list':
        if value.get('limit') is not None and (type(value['limit']) is not int or not 1 <= value['limit'] <= 64):
            raise ValueError('limit')
        if value.get('continuation') is not None:
            digest(value['continuation'])
    else:
        if set(value) != fields[mode]:
            raise ValueError('missing field')
        digest(value['archive_id'])
        keys = ['number'] if mode == 'record' else ['before_number', 'after_number']
        for key in keys:
            if type(value[key]) is not int or not 1 <= value[key] < 2**64:
                raise ValueError('record number')
        for key in ['record_digest'] if mode == 'record' else ['before_digest', 'after_digest']:
            digest(value[key])
        if mode == 'changes' and value['before_number'] >= value['after_number']:
            raise ValueError('ordered distinct records')
    return value

class Cursors:
    def __init__(self):
        self.entries = deque(maxlen=64)
    def issue(self, session, archive, head, after, limit):
        raw = b'dfmcp-progress-history-cursor/1\0' + session.to_bytes(16, 'big') + bytes.fromhex(archive + head) + struct.pack('>QQ', after, limit)
        token = hashlib.sha256(raw).hexdigest()
        record = (token, session, archive, head, after, limit)
        if record not in self.entries:
            self.entries.append(record)
        return token
    def resolve(self, token, session, archive, head, limit):
        for t, s, a, h, after, l in self.entries:
            if (t, s, a, h, l) == (token, session, archive, head, limit):
                return after
        raise ValueError('unissued or wrong-binding cursor')

def must_refuse(function, *args):
    try:
        function(*args)
    except (ValueError, TypeError, KeyError, OverflowError):
        return
    raise AssertionError('invalid input accepted')

def output_sizes():
    encode = lambda v: json.dumps(v, ensure_ascii=True, separators=(',', ':')).encode()
    details = {'job_type': 2**31-1, 'job_type_key': '\x01'*128, 'reaction': '\x01'*128,
               'recognized_recipe': None, 'reported_remaining': -32768, 'reported_total': -32768,
               'validated': False, 'active': False, 'raw_status_bits': 2**32-1, 'frequency': -2**31,
               'workshop_id': -2**31, 'max_workshops': 2**31-1, 'next_check_year': -2**31,
               'next_check_tick': -2**31, 'item_condition_count':4096,'order_condition_count':4096}
    row = {'native_order_id': 2**31-1, 'present': True, 'phase': 'unrecognized_or_modified', 'details':details,
           'production_completion_proven': False,'historical_creation_identity_proven':False}
    change = {'native_order_id':2**31-1,'kind':'disappeared_outcome_unknown','before_phase':'unrecognized_or_modified',
              'after_phase':'reported_zero_remaining','remaining_counter_decrease':100,'remaining_counter_increase':100,
              'goods_produced_proven':False}
    entry = {'archive_id':DIGEST,'record_number':4096,'segment':4096,'record_digest':DIGEST,'previous_digest':DIGEST,
             'observation_witness':DIGEST,'game_tick':2**64-1,'native_order_ids':[2**31-1]*32}
    summary = {'archive_id':DIGEST,'head':DIGEST,'fortress_id':str(2**64-1),'records':4096,'comparison_segments':4096,
               'retained_bytes':64*1024*1024,'authority_tick_floor':2**64-1,'read_only':False,
               'continuous_history_proven':False,'external_anti_rollback_floor':False}
    observation = {'witness':DIGEST,'bridge_generation':2**64-2,'capture_sequence':2**64-2,'game_tick':2**64-1,
                   'fortress_id':str(2**64-1),'world_folder':'\x01'*512,'site_id':2**31-1,'paused':False,
                   'next_order_id':2**31-1,'queue_count':4096,'rows':[row]*32,'selected_presence_complete':True,
                   'continuous_history_proven':False}
    record = {'reference':entry,'source_manifest':{'generation':2**64-2,'df_version':'\x01'*128,'dfhack_version':'\x01'*128},
              'observation':observation,'historical':True,'current_freshness_proven':False}
    comparison = {'status':'compared','reset_reason':None,'baseline_witness':DIGEST,'elapsed_game_ticks':2**64-1,
                  'client_acknowledged_baseline':False,'continuous_history_proven':False,'changes':[change]*32}
    values = {
        'metadata_page':{'ok':True,'historical':True,'native_calls':0,'entries':[entry]*64,'total_records':4096,
                         'complete_set_in_this_response':False,'continuation':DIGEST,'progress_archive':summary},
        'record':{'ok':True,'historical':True,'native_calls':0,'record':record,'progress_archive':summary},
        'changes':{'ok':True,'historical':True,'native_calls':0,'before':entry,'after':record,'comparison':comparison,
                   'progress_archive':summary},
    }
    # Add a conservative full 16 KiB envelope allowance on top of result JSON.
    sizes = {name: len(encode(v)) + 16*1024 for name, v in values.items()}
    assert max(sizes.values()) < 16*1024 + 32*4096
    return sizes

def main():
    accepted = 0
    for limit in range(1,65):
        for continuation in (None, DIGEST):
            request(json.dumps({'mode':'list','limit':limit,'continuation':continuation})); accepted += 1
    for number in (1,4096,2**64-1):
        request(json.dumps({'mode':'record','archive_id':DIGEST,'number':number,'record_digest':DIGEST}));accepted += 1
    for before,after in ((1,2),(1,4096),(4096,2**64-1)):
        request(json.dumps({'mode':'changes','archive_id':DIGEST,'before_number':before,'before_digest':DIGEST,
                            'after_number':after,'after_digest':DIGEST}));accepted += 1
    invalid = ['{}','[]','null','{"mode":"list","limit":1,"limit":2}', '{"mode":"list","mode":"record"}',
               '{"mode":"list"}x',' '*2049]
    for key,val in [('limit',0),('limit',65),('limit',-1),('limit',True),('limit',1.5),('limit',[]),
                    ('continuation','a'*63),('continuation','A'*64),('continuation','g'*64),('path','/tmp/x'),('mode','commit')]:
        invalid.append(json.dumps({'mode':'list',key:val}))
    record={'mode':'record','archive_id':DIGEST,'number':1,'record_digest':DIGEST}
    for key in record:
        bad=record.copy();del bad[key];invalid.append(json.dumps(bad))
    for val in (0,-1,2**64,True,1.5,'1',None):
        invalid.append(json.dumps({**record,'number':val}))
    for before,after in ((0,1),(1,1),(2,1)):
        invalid.append(json.dumps({'mode':'changes','archive_id':DIGEST,'before_number':before,'before_digest':DIGEST,
                                  'after_number':after,'after_digest':DIGEST}))
    for raw in invalid:
        must_refuse(request,raw)
    page_cases=0
    for count in (0,1,2,7,32,65,129,4096):
        for limit in (1,2,8,32,64):
            cursors=Cursors(); all_rows=[]; after=0
            while True:
                page=list(range(after+1,min(count,after+limit)+1)); all_rows+=page
                end=after+len(page)
                if end==count:
                    break
                token=cursors.issue(1,DIGEST,'b'*64,end,limit)
                assert cursors.issue(1,DIGEST,'b'*64,end,limit)==token
                after=cursors.resolve(token,1,DIGEST,'b'*64,limit)
                page_cases+=1
            assert all_rows==list(range(1,count+1));page_cases+=1
    cursors=Cursors();token=cursors.issue(1,DIGEST,'b'*64,1,1)
    wrong=[(token,2,DIGEST,'b'*64,1),(token,1,'c'*64,'b'*64,1),
           (token,1,DIGEST,'c'*64,1),(token,1,DIGEST,'b'*64,2),('0'*64,1,DIGEST,'b'*64,1)]
    for args in wrong:
        must_refuse(cursors.resolve,*args)
    must_refuse(Cursors().resolve,token,1,DIGEST,'b'*64,1)
    for i in range(2,67):
        cursors.issue(1,DIGEST,'b'*64,i,1)
    must_refuse(cursors.resolve,token,1,DIGEST,'b'*64,1)
    segment_cases=0
    for segment in (1,2):
        rows=decode(make_archive([body(sample()),body(sample(2,20,0),segment)]))
        assert (rows[0]['segment']==rows[1]['segment'])==(segment==1)
        segment_cases+=1
    source=(ROOT/'crates/dfmcp-mcp/src/live_work_order_progress_server.rs').read_text()
    history=(ROOT/'crates/dfmcp-mcp/src/progress_history.rs').read_text()
    assert source.count('#[tool(')==11
    assert 'reader: Option<Reader>' in source and 'fn offline_session(' in source
    assert source.index('let mut session = if offline') < source.index('let token = configured(ENVIRONMENT[1]')
    assert 'refresh_with_publication' in source and 'archive.reserve_capture(&storage)' in source
    assert '.commit_prepared(' not in source+history and '.prepare(' not in source+history
    assert 'render_checked("fortress.open_session"' in source and '*target = Some(session)' in source
    assert 'deny_unknown_fields' in history
    contract=json.loads((ROOT/'architecture/work_order_progress_v1_12.json').read_text())
    assert contract['bridge_protocol']=='1.12' and contract['observation_magic']=='DFMWP012'
    assert contract['methods']=={'Handshake':0,'ReadObservation':0} and not contract['production_runner_added']
    archive_contract=contract['archive']
    assert archive_contract['max_bytes']==64*1024*1024 and archive_contract['max_records']==4096
    assert archive_contract['max_history_request_bytes']==2048 and archive_contract['max_metadata_rows']==64
    assert archive_contract['history_queries']==['list','record','changes']
    assert not archive_contract['repairs'] and not archive_contract['creation_journal_reconciliation']
    files=['architecture/work_order_progress_v1_12.json','crates/dfmcp-mcp/src/live_work_order_progress_server.rs','crates/dfmcp-mcp/src/progress_history.rs',
           'crates/dfmcp-mcp/src/live_work_order_progress_server_tests.rs','crates/dfmcp-mcp/src/progress_history_tests.rs',
           'crates/dfmcp-adapter/src/work_order_progress.rs','crates/dfmcp-adapter/src/work_order_progress/archive.rs',
           'crates/dfmcp-adapter/src/work_order_progress/archive_file.rs','scripts/check_progress_history_mcp_reference.py']
    report={'schema':'dfmcp.progress-history-mcp-reference/1','status':'passed_reference_only',
            'requests_accepted':accepted,'requests_rejected':len(invalid),'pagination_cases':page_cases,
            'cursor_rebinding_restart_eviction_rejections':len(wrong)+2,'segment_comparison_cases':segment_cases,
            'output_model_bytes':output_sizes(),'complete_output_reservation_bytes':16*1024+32*4096,
            'new_mcp_history_rust_groups':11,'mcp_tools_wired':11,
            'rust_compiled':False,'rust_tests_executed':False,'filesystem_executed':False,'mcp_executed':False,'live_game_executed':False,
            'scope':'Independent Python request, cursor, pagination and JSON-size models plus lexical wiring, not Rust or MCP execution',
            'source_sha256':{f:hashlib.sha256((ROOT/f).read_bytes()).hexdigest() for f in files}}
    print(json.dumps(report,indent=2,sort_keys=True))

if __name__=='__main__':
    main()
