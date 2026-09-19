#!/usr/bin/env python3
"""Validate the standalone query component; no Rust/MCP execution is claimed."""
import copy
import hashlib
import json
from pathlib import Path
from jsonschema import Draft202012Validator


def main():
    path = Path(__file__).resolve().parents[1] / 'schemas/mcp_map_connectivity_v1.json'
    raw = path.read_bytes()
    schema = json.loads(raw)
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    valid = {"kind": "map_connectivity"}
    point = {"key": "workshop-west", "position": [0, 0, 5]}
    cases = [(valid, True), ({**valid, "landmarks": []}, True)]
    for section in [None, 'all', 'components', 'bottlenecks', 'bridges', 'landmarks']:
        cases.append(({**valid, 'section': section}, True))
    for field, values in {
        'limit': [None, 1, 128], 'max_work': [None, 1, 1000000],
        'continuation': [None, 'sp1:1:' + 'a'*64, 'sp1:123456:' + '0'*64]
    }.items():
        cases.extend(({**valid, field: value}, True) for value in values)
    cases += [({**valid, 'landmarks': [point]}, True),
              ({**valid, 'landmarks': [{'key': str(i), 'position': [32767, 0, 0]} for i in range(128)]}, True)]
    for field, values in {
        'kind': ['map', None, 1], 'section': ['', 'regions', 0, True],
        'limit': [0, -1, 129, 1.5, True, '1'], 'max_work': [0, -1, 1000001, 1.5, True, '1'],
        'continuation': ['', 'sp1:0:'+'0'*64, 'sp1:01:'+'0'*64, 'sp1:1000000:'+'0'*64,
                         'sp1:1:'+'0'*64+'\n', 'sp1:1:'+'A'*64, 'sp1:1:'+'0'*63, 1],
        'landmarks': [None, {}, [point]*129], 'unknown': [True]
    }.items():
        cases.extend(({**valid, field: value}, False) for value in values)
    for field, values in {
        'key': ['', 'a'*65, 'a\n', 'a\x00', 'a b', 'é', True, None],
        'position': [[], [0,0], [0,0,0,0], [-1,0,5], [32768,0,0], [True,0,0], [0.5,0,0], None],
        'extra': [True]
    }.items():
        for value in values:
            modified = copy.deepcopy(point)
            modified[field] = value
            cases.append(({**valid, 'landmarks': [modified]}, False))
    for field in ['key', 'position']:
        modified = copy.deepcopy(point)
        del modified[field]
        cases.append(({**valid, 'landmarks': [modified]}, False))
    cases += [({}, False), ([], False)]
    for value, expected in cases:
        assert validator.is_valid(value) == expected, (value, expected, list(validator.iter_errors(value)))
    print(json.dumps({'cases': len(cases), 'accepted': sum(expected for _, expected in cases),
        'rejected': sum(not expected for _, expected in cases), 'schema_sha256': hashlib.sha256(raw).hexdigest(),
        'evidence': 'Standalone JSON Schema only; duplicate keys, authority, Rust, composed discovery and MCP are not executed'}, indent=2))


if __name__ == '__main__':
    main()
