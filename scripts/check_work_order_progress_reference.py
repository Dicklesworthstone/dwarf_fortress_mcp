#!/usr/bin/env python3
"""Independent codec/size models and source wiring, NOT Rust or MCP execution."""
from __future__ import annotations
import hashlib
import json
from pathlib import Path
import struct
from test_work_order_progress_native import vector

ROOT = Path(__file__).resolve().parents[1]
NAMES = {1: 'ConstructBed', 2: 'ConstructDoor', 3: 'ConstructTable', 4: 'ConstructThrone'}


def decode(data: bytes, ids: list[int]) -> dict:
    if not 1 <= len(ids) <= 32 or ids != sorted(set(ids)) or any(i < 0 or i > 2**31-1 for i in ids):
        raise ValueError('selection')
    if len(data) > 16384 or data[:8] != b'DFMWP012':
        raise ValueError('wire generation')
    pos = 8
    def get(fmt: str):
        nonlocal pos
        n = struct.calcsize('>' + fmt)
        values = struct.unpack_from('>' + fmt, data, pos)
        pos += n
        return values
    def text(maximum: int, empty: bool = False):
        nonlocal pos
        (n,) = get('H')
        if not int(not empty) <= n <= maximum or pos + n > len(data):
            raise ValueError('text extent')
        s = data[pos:pos+n].decode('utf-8', errors='strict')
        pos += n
        if '\0' in s:
            raise ValueError('NUL')
        return s
    generation, sequence, tick, site, horizon, count, paused = get('QQQIIIB')
    if not 0 < generation < 2**64-1 or not 0 < sequence < 2**64-1 or tick > (2**32-1)*403200+403199:
        raise ValueError('capture identity')
    if site > 2**31-1 or horizon > 2**31-1 or count > 4096 or count > horizon or paused not in (0, 1):
        raise ValueError('queue metadata')
    folder = text(512)
    (rows,) = get('I')
    if rows != len(ids):
        raise ValueError('omitted selection')
    output = []
    for expected in ids:
        id_, present = get('IB')
        if id_ != expected or present not in (0, 1):
            raise ValueError('selected identity/presence')
        if not present:
            output.append({'id': id_, 'phase': 'absent'})
            continue
        if id_ >= horizon:
            raise ValueError('allocation horizon')
        job_type, recipe, left, total, status, frequency, workshop, maximum, year, year_tick, items, orders = get('iBiiIiiiiiII')
        type_key, reaction = text(128), text(128, True)
        if job_type < 0 or recipe not in range(5) or not -32768 <= left <= 32767 or not -32768 <= total <= 32767 or items > 4096 or orders > 4096:
            raise ValueError('native scalar')
        if recipe:
            if not 1 <= total <= 100 or not 0 <= left <= total or status & ~3 or frequency != 0 or workshop != -1 or maximum != 1 or items or orders or reaction or type_key != NAMES[recipe]:
                raise ValueError('contradictory recognized template')
        phase = ('unrecognized_or_modified' if not recipe else 'awaiting_validation' if not status & 1
                 else 'reported_zero_remaining' if left == 0 else 'active' if status & 2 else 'validated_inactive')
        output.append({'id': id_, 'remaining': left, 'total': total, 'phase': phase})
    if pos != len(data) or sum('remaining' in r for r in output) > count:
        raise ValueError('extent or impossible presence count')
    return {'generation': generation, 'sequence': sequence, 'tick': tick, 'folder': folder,
            'site': site, 'horizon': horizon, 'rows': output}


def rejected(data: bytes, ids: list[int]) -> None:
    try:
        decode(data, ids)
    except (ValueError, UnicodeError, struct.error):
        return
    raise AssertionError('invalid capture accepted')


def main() -> None:
    raw = vector()
    fixture = ROOT / 'crates/dfmcp-adapter/tests/fixtures/work_order_progress_v1_12.hex'
    assert bytes.fromhex(fixture.read_text()) == raw
    actual = decode(raw, [3,8])
    assert actual['rows'] == [{'id':3,'remaining':3,'total':5,'phase':'active'}, {'id':8,'phase':'absent'}]
    for n in range(len(raw)):
        rejected(raw[:n], [3,8])
    rejected(raw+b'\0', [3,8])
    selections = [[],[3],[8,3],[3,3],[3,9],[2**32-1]]
    for ids in selections:
        rejected(raw, ids)
    invalid = 0
    for offset, fmt, values in [(8,'Q',[0,2**64-1]),(16,'Q',[0,2**64-1]),(40,'I',[0,4097]),
                                (44,'B',[2,255]),(62,'B',[2,255]),(67,'B',[2,5,255]),
                                (68,'i',[-1,6,32768]),(72,'i',[0,101]),(76,'I',[4,2**32-1]),
                                (80,'i',[1]),(84,'i',[0]),(88,'i',[0]),(100,'I',[1]),(104,'I',[1])]:
        for value in values:
            bad = bytearray(raw)
            struct.pack_into('>'+fmt,bad,offset,value)
            rejected(bytes(bad),[3,8])
            invalid += 1
    positive = 0
    for recipe, type_key in NAMES.items():
        for total in range(1,101):
            for left in (0,total//2,total):
                for status in range(4):
                    value = bytearray(raw[:108])
                    value[67] = recipe
                    struct.pack_into('>iiI',value,68,left,total,status)
                    key = type_key.encode()
                    value += struct.pack('>H',len(key))+key+b'\0\0'+struct.pack('>IB',8,0)
                    row = decode(bytes(value),[3,8])['rows'][0]
                    expected = ('awaiting_validation' if not status & 1 else 'reported_zero_remaining' if left==0
                                else 'active' if status & 2 else 'validated_inactive')
                    assert row['phase']==expected
                    positive += 1
    # A maximally escaped row model, including one endpoint-change record.
    details = {'job_type':2**31-1,'job_type_key':'\x01'*128,'reaction':'\x01'*128,'recognized_recipe':None,
               'reported_remaining':-32768,'reported_total':-32768,'validated':False,'active':False,'raw_status_bits':2**32-1,
               'frequency':-2**31,'workshop_id':-2**31,'max_workshops':2**31-1,'next_check_year':-2**31,'next_check_tick':-2**31,
               'item_condition_count':4096,'order_condition_count':4096}
    row = {'native_order_id':2**31-1,'present':True,'phase':'unrecognized_or_modified','details':details,
           'production_completion_proven':False,'historical_creation_identity_proven':False}
    change = {'native_order_id':2**31-1,'kind':'disappeared_outcome_unknown','before_phase':'unrecognized_or_modified',
              'after_phase':'reported_zero_remaining','remaining_counter_decrease':100,'remaining_counter_increase':100,'goods_produced_proven':False}
    encode = lambda x: json.dumps(x,separators=(',',':'),ensure_ascii=True).encode()
    per_row = len(encode(row))+len(encode(change))+2
    assert per_row < 4096
    modeled_page = len(encode({'rows':[row]*32,'changes':[change]*32,'world_folder':'\x01'*512})) + 12*1024
    assert modeled_page < 16*1024+32*4096
    source = ROOT/'crates/dfmcp-mcp/src/live_work_order_progress_server.rs'
    s=source.read_text()
    names=['open_session','observe','query','plan','commit','wait','cancel','checkpoint','restore','explain','doctor']
    assert all(f'pub fn fortress_{name}(' in s for name in names)
    assert s.count('#[tool(')==11
    assert 'pub mod work_order_progress;' in (ROOT/'crates/dfmcp-adapter/src/lib.rs').read_text()
    assert 'pub mod live_work_order_progress_server;' in (ROOT/'crates/dfmcp-mcp/src/lib.rs').read_text()
    assert '.commit_prepared(' not in s and '.prepare(' not in s
    files=['crates/dfmcp-adapter/src/lib.rs','crates/dfmcp-mcp/src/lib.rs','crates/dfmcp-adapter/src/work_order_progress.rs','crates/dfmcp-adapter/src/work_order_progress/rpc.rs',
           'crates/dfmcp-adapter/src/work_order_progress/tests.rs','crates/dfmcp-adapter/src/work_order_progress/rpc_tests.rs',
           'crates/dfmcp-mcp/src/live_work_order_progress_server.rs','crates/dfmcp-mcp/src/live_work_order_progress_server_tests.rs',
           'crates/dfmcp-mcp/src/bin/dfmcp-live-work-order-progress-dev-server.rs','scripts/check_work_order_progress_reference.py']
    tests=sum((ROOT/f).read_text().count('#[test]') for f in files if f.endswith('tests.rs'))
    report={'schema':'dfmcp.work-order-progress-reference/1','status':'passed_reference_only',
            'native_fixture_bytes':len(raw),'native_fixture_sha256':hashlib.sha256(raw).hexdigest(),
            'truncated_prefixes_rejected':len(raw),'trailing_records_rejected':1,'selection_cases_rejected':len(selections),
            'invalid_identity_or_template_cases_rejected':invalid,'recipe_amount_counter_status_cases':positive,
            'maximal_row_plus_change_model_bytes':per_row,'maximal_page_with_envelope_model_bytes':modeled_page,
            'rust_tests_registered':tests,'rust_compiled':False,'rust_tests_executed':False,'mcp_executed':False,
            'scope':'Independent Python wire/phase/size models and source wiring; not Rust type checking, serialization or execution',
            'source_sha256':{f:hashlib.sha256((ROOT/f).read_bytes()).hexdigest() for f in files}}
    print(json.dumps(report,indent=2,sort_keys=True))


if __name__=='__main__':
    main()
