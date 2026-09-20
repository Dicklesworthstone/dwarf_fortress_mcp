#!/usr/bin/env python3
"""Independent archive format/continuity model. Does not execute Rust or fsync."""
from __future__ import annotations
import hashlib
import json
from pathlib import Path
import struct
from check_work_order_progress_reference import decode as decode_capture
from test_work_order_progress_native import vector

ROOT = Path(__file__).resolve().parents[1]
HEADER = b'DFMWPA12'
FRAME = b'DFMWPR12'
FOOTER = b'DFMWPE12'
HD = b'dfmcp-progress-archive-header/1\0'
FD = b'dfmcp-progress-archive-frame/1\0'
sha = lambda b: hashlib.sha256(b).digest()
u64 = lambda n: struct.pack('>Q', n)
text = lambda b: struct.pack('>H', len(b)) + b

def sample(sequence=1, tick=10, remaining=5):
    b = bytearray(vector())
    struct.pack_into('>QQ', b, 16, sequence, tick)
    struct.pack_into('>i', b, 68, remaining)
    return bytes(b)

def lineage(c):
    return int.from_bytes(sha(b'dfmcp-live-fortress-id-v1\0' + c['folder'].encode() + b'\0' + struct.pack('>I', c['site']))[:8], 'big') | 1

def body(raw, segment=1, df=b'df', dh=b'dfhack', ids=(3, 8), generation=7):
    return u64(segment) + u64(generation) + text(df) + text(dh) + bytes([len(ids)]) + b''.join(struct.pack('>I', i) for i in ids) + struct.pack('>I', len(raw)) + raw

def make_archive(bodies):
    fortress = lineage(decode_capture(sample(), [3, 8]))
    identity = sha(b'fixed-independent-archive')
    header = HEADER + u64(fortress) + identity
    head = sha(HD + header)
    result = header + head
    for number, data in enumerate(bodies, 1):
        prefix = FRAME + struct.pack('>IQ', len(data), number) + head
        head = sha(FD + identity + prefix + data)
        result += prefix + data + head + FOOTER
    return result

def decode(data):
    if len(data) < 80 or len(data) > 64*1024*1024 or data[:8] != HEADER or sha(HD+data[:48]) != data[48:80]:
        raise ValueError('archive header')
    fortress, = struct.unpack_from('>Q', data, 8)
    if not fortress:
        raise ValueError('fortress')
    identity, head, pos, records = data[16:48], data[48:80], 80, []
    while pos < len(data):
        if len(records) >= 4096 or len(data)-pos < 92 or data[pos:pos+8] != FRAME:
            raise ValueError('frame header or limit')
        size, number = struct.unpack_from('>IQ', data, pos+8)
        end = pos+52+size
        if size > 18*1024 or end+40 > len(data) or number != len(records)+1 or data[pos+20:pos+52] != head:
            raise ValueError('length or chain order')
        calculated = sha(FD+identity+data[pos:end])
        if calculated != data[end:end+32] or data[end+32:end+40] != FOOTER:
            raise ValueError('checksum or footer')
        value = data[pos+52:end]
        segment, generation = struct.unpack_from('>QQ', value)
        p = 16
        versions = []
        for _ in range(2):
            n, = struct.unpack_from('>H', value, p); p += 2
            s = value[p:p+n].decode('utf-8'); p += n
            if not 1 <= n <= 128 or len(s.encode()) != n or '\0' in s:
                raise ValueError('manifest text')
            versions.append(s)
        count = value[p]; p += 1
        if not 1 <= count <= 32:
            raise ValueError('selection count')
        ids = list(struct.unpack_from('>'+'I'*count, value, p)); p += 4*count
        n, = struct.unpack_from('>I', value, p); p += 4
        if p+n != len(value):
            raise ValueError('capture extent')
        capture = decode_capture(value[p:], ids)
        if not segment or generation != capture['generation'] or lineage(capture) != fortress:
            raise ValueError('capture identity')
        if records:
            prior = records[-1]
            if segment == prior['segment']:
                a = prior['capture']
                if prior['versions'] != versions or generation != a['generation'] or ids != prior['ids'] or capture['sequence'] <= a['sequence'] or capture['tick'] < a['tick'] or capture['horizon'] < a['horizon']:
                    raise ValueError('continuity')
            elif segment != prior['segment']+1:
                raise ValueError('segment sequence')
        elif segment != 1:
            raise ValueError('first segment')
        records.append({'number': number, 'segment': segment, 'versions': versions, 'ids': ids, 'capture': capture, 'digest': calculated.hex()})
        head, pos = calculated, end+40
    return records

def reject(data):
    try:
        decode(data)
    except (ValueError, struct.error, IndexError, UnicodeError):
        return
    raise AssertionError('invalid archive accepted')

def main():
    raw = make_archive([body(sample()), body(sample(2, 20, 2))])
    rows = decode(raw)
    assert [r['capture']['rows'][0]['remaining'] for r in rows] == [5, 2]
    for n in range(len(raw)):
        bad = bytearray(raw); bad[n] ^= 1; reject(bytes(bad))
    good_prefixes = {80, len(make_archive([body(sample())]))}
    prefixes = 0
    for n in range(len(raw)):
        if n not in good_prefixes:
            reject(raw[:n]); prefixes += 1
    invalid_bodies = [body(sample(2), 0), body(sample(2), 3), body(sample(1)),
                      body(sample(2, 9)), body(sample(2), df=b'changed'),
                      body(sample(2), generation=8), body(sample(2), ids=(3,)),
                      body(sample(2), ids=(8, 3)), body(sample(2), df=b''),
                      body(sample(2), dh=b'\0'), body(sample(2)) + b'\0']
    for bad in invalid_bodies:
        reject(make_archive([body(sample()), bad]))
    positive = 0
    for sequence in range(2, 12):
        for remaining in range(6):
            for segment in (1, 2):
                entries = decode(make_archive([body(sample()), body(sample(sequence, 20, remaining), segment)]))
                assert entries[0]['segment'] == 1 and entries[1]['segment'] == segment
                # Distinct segments are never eligible for endpoint comparison.
                assert (entries[0]['segment'] == entries[1]['segment']) == (segment == 1)
                positive += 1
    files = ['crates/dfmcp-adapter/src/work_order_progress/archive.rs',
             'crates/dfmcp-adapter/src/work_order_progress/archive_file.rs',
             'crates/dfmcp-adapter/src/work_order_progress/archive_tests.rs',
             'crates/dfmcp-adapter/src/work_order_progress.rs',
             'crates/dfmcp-adapter/src/work_order_progress/tests.rs',
             'scripts/check_progress_archive_reference.py']
    report = {'schema': 'dfmcp.progress-archive-reference/1', 'status': 'passed_reference_only',
              'archive_bytes': len(raw), 'archive_sha256': sha(raw).hex(),
              'byte_corruptions_rejected': len(raw), 'incomplete_prefixes_rejected': prefixes,
              'illegal_rehashed_histories_rejected': len(invalid_bodies),
              'valid_segment_counter_cases': positive, 'new_rust_test_groups': 13,
              'rust_compiled': False, 'rust_tests_executed': False, 'filesystem_executed': False,
              'mcp_executed': False, 'live_game_executed': False,
              'scope': 'Independent Python framing/continuity model, not Rust, filesystem durability or game truth',
              'source_sha256': {f: sha((ROOT/f).read_bytes()).hex() for f in files}}
    print(json.dumps(report, indent=2, sort_keys=True))

if __name__ == '__main__':
    main()
