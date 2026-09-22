#!/usr/bin/env python3
"""Reproduce journal interoperability vectors; this does not execute Rust.

Uses only Python's standard JSON, struct and SHA-256 implementations. Expected
outcomes are explicit examples, not a replacement implementation of the monitor.
With --python-monitor, also execute the existing tracker against every vector.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import struct

DOMAIN = b'dfmcp-excavation-journal/1\0'
ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / 'tests/fixtures/excavation_inventory.json'


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True,
                      allow_nan=False).encode('ascii')


def capture(tick: int, *, presence: int = 2, shape: int = 3, generation: int = 7,
            folder: str = 'region1', width: int = 2, dimensions: int = 32) -> dict:
    name = folder.encode('utf-8')
    raw = b'DFMM1500' + struct.pack('>IIBIH', 0, tick, 1, 1, len(name)) + name
    raw += struct.pack('>IIIIIIIIII', dimensions, 32, 3, 15, 15, 2, width, 2, 1, width * 2)
    tile = bytes([presence])
    if presence == 2:
        tile += struct.pack('>IBBBBBBBIHH', shape, shape, 0, 0, 0, 0, 0, 0, 1, 10015, 10015)
    raw += tile * (width * 2)
    return {'manifest': {'generation': generation, 'df_version': 'df', 'dfhack_version': 'dfhack'},
            'capture_hex': raw.hex()}


def begin(sample: dict | None = None, **goal_changes) -> dict:
    goal = {'region': {'origin': [15, 15, 2], 'size': [2, 2, 1]}, 'folder': 'region1', 'site': 1,
            'deadline_tick': 100, 'stable_ticks': 10, 'required_samples': 2, 'max_gap_ticks': 20}
    goal.update(goal_changes)
    return {'kind': 'begin', 'format': 'dfmcp.excavation-goal/1', 'nonce': '01' * 32,
            'endpoint': '127.0.0.1:5000', 'goal': goal, 'sample': sample or capture(10)}


def journal(events: list[dict]) -> bytes:
    previous = '0' * 64
    frames = []
    for sequence, event in enumerate(events):
        body = {'event': event, 'sequence': sequence, 'previous': previous}
        previous = hashlib.sha256(DOMAIN + canonical(body)).hexdigest()
        frames.append(canonical({**body, 'sha256': previous}) + b'\n')
    return b''.join(frames)


def sample(value: dict) -> list[dict]:
    return [{'kind': 'read_started'}, {'kind': 'sample', 'sample': value}]


def vectors() -> dict:
    cases = []

    def add(name, events, status, streak, tick, *, pending=False):
        raw = journal(events)
        parsed = [json.loads(line) for line in raw.splitlines()]
        cases.append({'name': name, 'journal': raw.decode('ascii'), 'status': status, 'streak': streak,
                      'sample_tick': tick, 'pending_read': pending,
                      'id': parsed[0]['sha256'], 'head': parsed[-1]['sha256']})

    add('floor_baseline', [begin()], 'stabilizing', 1, 10)
    add('wall_baseline', [begin(capture(10, shape=2))], 'pending', 0, 10)
    add('hidden_baseline', [begin(capture(10, presence=1))], 'unknown', 0, 10)
    add('missing_baseline', [begin(capture(10, presence=0))], 'unknown', 0, 10)
    add('same_tick', [begin(), *sample(capture(10))], 'stabilizing', 1, 10)
    add('satisfied', [begin(), *sample(capture(20))], 'satisfied', 2, 20)
    add('deadline_inclusive', [begin(capture(90)), *sample(capture(100))], 'satisfied', 2, 100)
    add('deadline_expired', [begin(capture(90)), *sample(capture(101))], 'expired', 0, 101)
    add('read_failed', [begin(), {'kind': 'read_started'}, {'kind': 'read_failed'}], 'unknown', 0, 10)
    add('read_unfinished', [begin(), {'kind': 'read_started'}], 'unknown', 0, 10, pending=True)
    add('interrupted_resume', [begin(), {'kind': 'read_started'}, *sample(capture(20))], 'stabilizing', 1, 20)
    add('gap_reset', [begin(), *sample(capture(31))], 'stabilizing', 1, 31)
    add('generation_changed', [begin(), *sample(capture(20, generation=8))], 'invalidated', 0, 10)
    add('dimensions_changed', [begin(), *sample(capture(20, dimensions=33))], 'invalidated', 0, 10)
    add('clock_regression', [begin(), *sample(capture(9))], 'invalidated', 0, 10)
    add('cancelled_monitor', [begin(), {'kind': 'cancel'}], 'cancelled', 0, 10)
    add('single_sample', [begin(stable_ticks=0, required_samples=1)], 'satisfied', 1, 10)
    unicode_folder = 'cavern-\u00e9-\U0001f3d4'
    add('unicode_identity', [begin(capture(10, folder=unicode_folder), folder=unicode_folder)], 'stabilizing', 1, 10)

    negatives = []
    def bad(name, events):
        negatives.append({'name': name, 'journal': journal(events).decode('ascii')})
    bad('sample_without_intent', [begin(), {'kind': 'sample', 'sample': capture(20)}])
    bad('failure_without_intent', [begin(), {'kind': 'read_failed'}])
    bad('new_header_in_history', [begin(), begin()])
    bad('serialized_success_is_not_evidence', [begin(), {'kind': 'satisfied'}])
    bad('terminal_extension', [begin(stable_ticks=0, required_samples=1), {'kind': 'cancel'}])
    bad('region_substitution', [begin(), *sample(capture(20, width=1))])
    wrong = begin(); wrong['endpoint'] = '192.0.2.1:5000'
    bad('non_loopback', [wrong])
    wrong = begin(); wrong['nonce'] = '0' * 64
    bad('zero_incarnation', [wrong])
    wrong = begin(); wrong['goal']['required_samples'] = True
    bad('boolean_sample_bound', [wrong])
    wrong = begin(); wrong['goal']['required_samples'] = 2.0
    bad('float_sample_bound', [wrong])
    wrong = begin(); wrong['goal']['extra'] = 1
    bad('unknown_goal_field', [wrong])
    wrong = begin(); wrong['sample']['manifest']['generation'] = 0
    bad('zero_map_incarnation', [wrong])
    strings = ['"\\\b\t\n\f\r\x00\x1f\x7f', 'rock-\u00e9-\U0001f3d4', '\u2028\u2029', '\U00010000\uffff']
    # Deduplicate complete literal frames without changing any journal bytes.
    frames: list[str] = []
    indices: dict[str, int] = {}
    for case in cases + negatives:
        parts = case.pop('journal').splitlines(keepends=True)
        for part in parts:
            if part not in indices:
                indices[part] = len(frames)
                frames.append(part)
        case['frames'] = [indices[part] for part in parts]
    return {'schema': 'dfmcp.excavation-inventory-fixtures/1', 'accepted': cases, 'rejected': negatives,
            'frames': frames,
            'canonical_strings': [{'text': s, 'ascii_json': canonical(s).decode('ascii')} for s in strings]}


def raw_journal(value: dict, case: dict) -> bytes:
    return ''.join(value['frames'][i] for i in case['frames']).encode('ascii')


def check_fixture(value: dict) -> dict:
    frames = 0
    for case in value['accepted'] + value['rejected']:
        previous = '0' * 64
        for sequence, line in enumerate(raw_journal(value, case).splitlines(keepends=True)):
            frame = json.loads(line)
            assert canonical(frame) + b'\n' == line
            assert frame['sequence'] == sequence and frame['previous'] == previous
            body = {key: frame[key] for key in ('event', 'sequence', 'previous')}
            previous = hashlib.sha256(DOMAIN + canonical(body)).hexdigest()
            assert previous == frame['sha256']
            frames += 1
    for case in value['canonical_strings']:
        assert canonical(case['text']).decode('ascii') == case['ascii_json']
    return {'accepted_examples': len(value['accepted']), 'rejected_examples': len(value['rejected']),
            'frames_recomputed': frames, 'canonical_string_examples': len(value['canonical_strings'])}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    parser.add_argument('--python-monitor', action='store_true')
    args = parser.parse_args()
    value = vectors()
    expected = json.dumps(value, indent=2, sort_keys=True, ensure_ascii=True) + '\n'
    if args.write:
        FIXTURE.parent.mkdir(parents=True, exist_ok=True)
        if FIXTURE.exists() and FIXTURE.read_text() != expected:
            raise SystemExit('Refusing to overwrite different fixture bytes')
        FIXTURE.write_text(expected)
    assert FIXTURE.read_text() == expected, 'Fixture differs from independent deterministic encoder'
    result = check_fixture(value)
    if args.python_monitor:
        import track_excavation as tracker
        for case in value['accepted']:
            h = tracker.replay(raw_journal(value, case))
            assert ('unknown' if h.pending_read else h.progress.status) == case['status']
            assert (0 if h.pending_read else h.progress.streak) == case['streak']
            assert h.progress.latest.tick == case['sample_tick']
            assert h.identity == case['id'] and h.head == case['head']
        for case in value['rejected']:
            try:
                tracker.replay(raw_journal(value, case))
            except (ValueError, TypeError, KeyError):
                continue
            raise AssertionError('Invalid history accepted: ' + case['name'])
    result.update(rust_executed=False, python_monitor_executed=args.python_monitor,
                  fixture_sha256=hashlib.sha256(FIXTURE.read_bytes()).hexdigest(),
                  status='passed_fixture_encoding_only' if not args.python_monitor else 'passed_python_monitor_interoperability')
    print(json.dumps(result, sort_keys=True, indent=2))


if __name__ == '__main__':
    main()
