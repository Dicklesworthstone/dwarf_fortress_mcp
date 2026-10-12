#!/usr/bin/env python3
"""Check handoff schema and full-target fixtures, not the Rust/MCP implementation.

Owning bead: dfmcp-wp-pln03-labor-civilian-alert-fsm-zdm.
The fixture lowerer below is an independent structural model only.
"""
from __future__ import annotations
import copy
import json
from pathlib import Path
from jsonschema import Draft202012Validator
from test_shelter_watch_schema import input_shape

ROOT = Path(__file__).resolve().parent.parent


def fixture_arguments(query: dict, *, extreme: bool = False) -> dict:
    """Model the closed prospective request shape, retaining every target."""
    watch = query['watch']
    anchor = {
        'fortress_id': 'f' * 16, 'epoch': 2**64-1, 'sequence': 2**64-1,
        'game_tick': 2**64-1001, 'state_hash': 'f' * 64,
    } if extreme else {
        'fortress_id': '0000000000000001', 'epoch': 0, 'sequence': 0,
        'game_tick': 1, 'state_hash': 'a' * 64,
    }
    return {'session_id': 'f' * 32, 'query': {
        'schema': 'dfmcp.query/1', 'expected_anchor': anchor, 'query': {
            'kind': 'watch', 'key': watch['key'],
            'label': watch.get('label') if watch.get('label') is not None else watch['key'],
            'condition': {'op': 'shelter_set',
                'citizens': sorted(copy.deepcopy(query['citizens']), key=lambda c: int(c['entity_id'])),
                'shelters': sorted(copy.deepcopy(query['shelters']), key=lambda a: (a['min'], a['max']))},
            'deadline_tick': watch['deadline_tick'],
            'poll_interval_ticks': watch.get('poll_interval_ticks') or 1,
            'stable_observations': watch.get('stable_observations') or 2,
        }}}


def main() -> None:
    status_schema = json.loads((ROOT/'schemas/mcp_shelter_status_v1.json').read_text())
    condition_schema = json.loads((ROOT/'schemas/mcp_watch_shelter_set_v1.json').read_text())
    for schema in [status_schema, condition_schema]:
        Draft202012Validator.check_schema(schema)
    status, condition = map(Draft202012Validator, [status_schema, condition_schema])
    example = json.loads((ROOT/'schemas/examples/shelter_watch_handoff_v1.json').read_text())
    original = example['query']
    status.validate(original)
    invalid = []
    for field in ('key', 'deadline_tick'):
        q = copy.deepcopy(original)
        del q['watch'][field]
        invalid.append(q)
    for field in ('source_policy', 'positions', 'citizens', 'authority', 'release'):
        q = copy.deepcopy(original)
        q['watch'][field] = True
        invalid.append(q)
    for field, values in {
        'key': ['', 'x'*65, '\0', None, 1, True],
        'label': ['', 'x'*257, '\0', 7, True],
        'deadline_tick': [0, -1, 2**64, None, '1000', 0.5, True],
        'poll_interval_ticks': [0, -1, 1000001, 0.5, '10', True],
        'stable_observations': [0, -1, 65, 1.5, '2', True],
    }.items():
        for value in values:
            q = copy.deepcopy(original)
            q['watch'][field] = value
            invalid.append(q)
    for value in ([], 'watch', 1, True):
        q = copy.deepcopy(original)
        q['watch'] = value
        invalid.append(q)
    for q in invalid:
        if status.is_valid(q):
            raise AssertionError(f'malformed handoff accepted: {q!r}')

    legacy_cases = []
    for present in (False, True):
        q = copy.deepcopy(original)
        if present:
            q['watch'] = None
        else:
            del q['watch']
        status.validate(q)
        legacy_cases.append(q)
    handoffs, maximum_nodes, maximum_accounted, maximum_bytes = 0, 0, 0, 0
    for size in (1, 2, 32, 128):
        for regions in (1, 16):
            for explicit_nulls in (False, True):
                q = copy.deepcopy(original)
                q['citizens'] = [{'entity_id': str((3<<40)+2**31-1-n), 'generation': 2**32-1}
                                 for n in range(size)]
                q['shelters'] = [{'min': [32767-x, 32767, 32767],
                                 'max': [32767-x, 32767, 32767]} for x in range(regions)]
                # Control characters (except NUL) are permitted by existing
                # watch labels and force worst-case JSON escaping.
                q['watch'] = {'key': '\x01'*64, 'label': '\x02'*256,
                    'deadline_tick': 2**64-1, 'poll_interval_ticks': 10, 'stable_observations': 64}
                if explicit_nulls:
                    q['watch']['poll_interval_ticks'] = None
                    q['watch']['stable_observations'] = None
                status.validate(q)
                args = fixture_arguments(q, extreme=True)
                goal = args['query']['query']['condition']
                condition.validate(goal)
                assert len(goal['citizens']) == size and len(goal['shelters']) == regions
                assert {c['entity_id'] for c in goal['citizens']} == {c['entity_id'] for c in q['citizens']}
                nodes, accounted = input_shape(args['query'])
                raw = json.dumps(args, sort_keys=True, separators=(',', ':'), ensure_ascii=False)
                assert len(raw.encode()) <= 16384
                assert json.loads(raw) == args
                maximum_nodes = max(maximum_nodes, nodes)
                maximum_accounted = max(maximum_accounted, accounted)
                maximum_bytes = max(maximum_bytes, len(raw.encode()))
                handoffs += 1
    print(json.dumps({'schema': 'shelter-watch-handoff-structural-check/1',
        'metaschemas_checked': 2, 'examples_accepted': 1,
        'malformed_handoffs_rejected': len(invalid), 'legacy_variants_accepted': len(legacy_cases),
        'complete_fixture_handoffs_checked': handoffs,
        'maximum_fixture_request_nodes': maximum_nodes,
        'maximum_fixture_accounted_request_bytes': maximum_accounted,
        'maximum_fixture_arguments_bytes': maximum_bytes,
        'rust_executed': False, 'native_game_executed': False}, sort_keys=True))


if __name__ == '__main__':
    main()
